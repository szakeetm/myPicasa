use std::{
    fs,
    io::{BufRead, Cursor, Seek},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::{
    DynamicImage, ImageDecoder, ImageReader,
    codecs::jpeg::JpegEncoder,
    imageops::FilterType,
};

use crate::util::errors::AppError;

const EXTERNAL_TOOL_TIMEOUT: Duration = Duration::from_secs(12);
const VIDEO_THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(30);
pub const VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_THUMBNAIL_JPEG_QUALITY: u8 = 82;
const DEFAULT_THUMBNAIL_SOURCE_JPEG_QUALITY: u8 = 82;
const HIGH_QUALITY_THUMB_JPEG_QUALITY: u8 = 94;
const HIGH_QUALITY_THUMB_SOURCE_JPEG_QUALITY: u8 = 96;
const HIGH_QUALITY_THUMB_MAX_SIZE: u32 = 512;

pub struct ThumbnailGenerationOutput {
    pub bytes: Option<Vec<u8>>,
}

pub fn thumbnail_generator_label(path: &Path) -> &'static str {
    if is_video_path(path) {
        return "ffmpeg";
    }

    #[cfg(target_os = "macos")]
    {
        return "sips";
    }

    #[allow(unreachable_code)]
    "rust"
}

pub fn generate_thumbnail(
    path: &Path,
    size: u32,
    allow_upscale: bool,
    working_dir: &Path,
) -> Result<ThumbnailGenerationOutput, AppError> {
    if is_video_path(path) {
        let bytes = render_video_thumbnail_with_ffmpeg(path, size, allow_upscale, working_dir)?;
        return Ok(ThumbnailGenerationOutput { bytes });
    }

    #[cfg(target_os = "macos")]
    {
        let render_size = thumbnail_render_size(size);
        let source_quality = thumbnail_source_quality(size);
        let output_quality = thumbnail_output_quality(size);
        if use_high_quality_thumbnail_settings(size) {
            if let Some(bytes) = render_square_thumbnail_with_sips(
                path,
                size,
                allow_upscale,
                output_quality,
                working_dir,
            )? {
                return Ok(ThumbnailGenerationOutput { bytes: Some(bytes) });
            }
        }

        if !use_high_quality_thumbnail_settings(size) {
            if let Some(bytes) =
                render_with_sips(path, render_size, allow_upscale, source_quality, working_dir)?
            {
                return Ok(ThumbnailGenerationOutput { bytes: Some(bytes) });
            }
        }

        if let Some(bytes) =
            render_with_sips(path, render_size, allow_upscale, source_quality, working_dir)?
        {
            let normalized =
                normalize_image_bytes_to_square_jpeg(&bytes, size, allow_upscale, output_quality)?;
            return Ok(ThumbnailGenerationOutput {
                bytes: Some(normalized),
            });
        }
    }

    let image = load_image(path, size, allow_upscale, working_dir)?;
    let output_quality = thumbnail_output_quality(size);
    let bytes = encode_square_thumbnail_to_jpeg(&image, size, allow_upscale, output_quality)?;
    Ok(ThumbnailGenerationOutput {
        bytes: Some(bytes),
    })
}

pub fn generate_viewer_image(path: &Path, max_dimension: u32, working_dir: &Path) -> Result<Option<Vec<u8>>, AppError> {
    let extension = normalized_extension(path);
    if is_video_extension(&extension) {
        return Ok(None);
    }

    if matches!(extension.as_str(), "heic" | "heif") {
        if let Some(bytes) = render_with_sips_original(path, 90, working_dir)? {
            return Ok(Some(bytes));
        }
    }

    let image = load_image(path, max_dimension, false, working_dir)?;
    let fitted = if image.width() > max_dimension || image.height() > max_dimension {
        image.resize(
            max_dimension,
            max_dimension,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };

    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 90);
    encoder.encode_image(&fitted)?;
    Ok(Some(buffer))
}

pub fn generate_viewer_image_file(
    path: &Path,
    max_dimension: u32,
    cache_dir: &Path,
    working_dir: &Path,
) -> Result<Option<PathBuf>, AppError> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let cache_key = format!(
        "{}:{}:{}:{}",
        path.display(),
        max_dimension,
        metadata.len(),
        modified
    );
    fs::create_dir_all(cache_dir)?;
    let output_path = cache_dir.join(format!(
        "my-picasa-viewer-{}.jpg",
        blake3::hash(cache_key.as_bytes()).to_hex()
    ));

    if output_path.is_file() {
        return Ok(Some(output_path));
    }

    let Some(bytes) = generate_viewer_image(path, max_dimension, working_dir)? else {
        return Ok(None);
    };
    let temp_output = output_path.with_extension("tmp.jpg");
    let _ = fs::remove_file(&temp_output);
    fs::write(&temp_output, bytes)?;
    fs::rename(&temp_output, &output_path)?;
    Ok(Some(output_path))
}

pub fn generate_viewer_video(
    path: &Path,
    cache_dir: &Path,
    timeout: Duration,
) -> Result<Option<(PathBuf, bool, String)>, AppError> {
    let Some(output_path) = viewer_video_cache_path(path, cache_dir)? else {
        return Ok(None);
    };

    let ffmpeg = match find_command_binary("ffmpeg") {
        Some(path) => path,
        None => return Ok(None),
    };

    if output_path.is_file() {
        return Ok(Some((output_path, true, "cached_transcoded_mp4".to_string())));
    }

    let temp_output = output_path.with_extension("tmp.mp4");
    let _ = fs::remove_file(&temp_output);
    let _ = fs::remove_file(&output_path);
    let dimensions = probe_video_dimensions(path)?;

    for encoder in preferred_viewer_video_encoders() {
        let status = wait_for_status_with_timeout(
            build_viewer_transcode_command(&ffmpeg, path, &temp_output, dimensions, encoder)?.spawn()?,
            timeout,
            "ffmpeg viewer transcode",
        )?;
        if status.success() && temp_output.is_file() {
            fs::rename(&temp_output, &output_path)?;
            return Ok(Some((output_path, false, encoder.to_string())));
        }
        let _ = fs::remove_file(&temp_output);
    }

    Ok(None)
}

fn build_viewer_transcode_command(
    ffmpeg: &Path,
    input_path: &Path,
    output_path: &Path,
    dimensions: Option<(u32, u32)>,
    encoder: &str,
) -> Result<Command, AppError> {
    let (target_bitrate, maxrate, bufsize) = viewer_video_bitrate_profile(dimensions);
    let mut command = Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(input_path)
        .arg("-pix_fmt")
        .arg("yuv420p");

    match encoder {
        "hevc_videotoolbox" => {
            command
                .arg("-vcodec")
                .arg("hevc_videotoolbox")
                .arg("-tag:v")
                .arg("hvc1")
                .arg("-b:v")
                .arg(target_bitrate)
                .arg("-maxrate")
                .arg(maxrate);
        }
        "libx265" => {
            command
                .arg("-vcodec")
                .arg("libx265")
                .arg("-preset")
                .arg("medium")
                .arg("-tag:v")
                .arg("hvc1")
                .arg("-b:v")
                .arg(target_bitrate)
                .arg("-maxrate")
                .arg(maxrate)
                .arg("-bufsize")
                .arg(bufsize)
                .arg("-crf")
                .arg("27");
        }
        _ => {
            command
                .arg("-vcodec")
                .arg("libx264")
                .arg("-preset")
                .arg("veryfast")
                .arg("-b:v")
                .arg(target_bitrate)
                .arg("-maxrate")
                .arg(maxrate)
                .arg("-bufsize")
                .arg(bufsize)
                .arg("-crf")
                .arg("22");
        }
    }

    command
        .arg("-acodec")
        .arg("aac")
        .arg("-b:a")
        .arg("160k")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    Ok(command)
}

fn preferred_viewer_video_encoders() -> Vec<&'static str> {
    let mut encoders = Vec::new();
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        encoders.push("hevc_videotoolbox");
    }
    encoders.push("libx265");
    encoders.push("libx264");
    encoders
}

pub fn viewer_video_cache_path(path: &Path, cache_dir: &Path) -> Result<Option<PathBuf>, AppError> {
    if !is_video_path(path) {
        return Ok(None);
    }

    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let cache_key = format!("{}:{}:{}", path.display(), metadata.len(), modified);
    fs::create_dir_all(cache_dir)?;
    Ok(Some(cache_dir.join(format!(
        "my-picasa-viewer-{}.mp4",
        blake3::hash(cache_key.as_bytes()).to_hex()
    ))))
}

pub fn viewer_render_cache_stats(cache_dir: &Path) -> Result<(u32, u64), AppError> {
    let mut items = 0_u32;
    let mut bytes = 0_u64;

    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with("my-picasa-viewer-") {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    items += 1;
                    bytes += metadata.len();
                }
            }
        }
    }

    Ok((items, bytes))
}

pub fn clear_viewer_render_cache(cache_dir: &Path) -> Result<(), AppError> {
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with("my-picasa-viewer-") {
                if path.is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
    Ok(())
}

fn viewer_video_bitrate_profile(dimensions: Option<(u32, u32)>) -> (&'static str, &'static str, &'static str) {
    let max_dimension = dimensions
        .map(|(width, height)| width.max(height))
        .unwrap_or(1920);
    match max_dimension {
        0..=640 => ("900k", "1200k", "1800k"),
        641..=960 => ("1800k", "2500k", "3600k"),
        961..=1280 => ("3000k", "4000k", "6000k"),
        1281..=1920 => ("5000k", "6500k", "9000k"),
        1921..=2560 => ("8000k", "10000k", "14000k"),
        _ => ("14000k", "18000k", "24000k"),
    }
}

pub fn probe_video_dimensions(path: &Path) -> Result<Option<(u32, u32)>, AppError> {
    let Some(ffprobe) = find_command_binary("ffprobe") else {
        return Ok(None);
    };
    let output = wait_for_output_with_timeout(
        Command::new(ffprobe)
            .arg("-v")
            .arg("error")
            .arg("-select_streams")
            .arg("v:0")
            .arg("-show_entries")
            .arg("stream=width,height")
            .arg("-of")
            .arg("csv=p=0:s=x")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
        Duration::from_secs(5),
        "ffprobe dimension probe",
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = value.split('x');
    let Some(width) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return Ok(None);
    };
    let Some(height) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return Ok(None);
    };
    Ok(Some((width, height)))
}

pub fn probe_media_duration_ms(path: &Path) -> Result<Option<i64>, AppError> {
    if !is_video_path(path) {
        return Ok(None);
    }

    let Some(ffprobe) = find_command_binary("ffprobe") else {
        return Ok(None);
    };

    let output = wait_for_output_with_timeout(
        Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?,
        EXTERNAL_TOOL_TIMEOUT,
        "ffprobe duration probe",
    )?;

    if !output.status.success() {
        return Ok(None);
    }

    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok();

    Ok(duration
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| (value * 1000.0).round() as i64))
}

pub fn probe_primary_video_codec(path: &Path) -> Result<Option<String>, AppError> {
    if !is_video_path(path) {
        return Ok(None);
    }

    let Some(ffprobe) = find_command_binary("ffprobe") else {
        return Ok(None);
    };

    let output = wait_for_output_with_timeout(
        Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=codec_name")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?,
        EXTERNAL_TOOL_TIMEOUT,
        "ffprobe codec probe",
    )?;

    if !output.status.success() {
        return Ok(None);
    }

    let codec = String::from_utf8_lossy(&output.stdout).trim().to_ascii_lowercase();
    if codec.is_empty() {
        return Ok(None);
    }

    Ok(Some(codec))
}

fn render_video_thumbnail_with_ffmpeg(
    path: &Path,
    size: u32,
    allow_upscale: bool,
    working_dir: &Path,
) -> Result<Option<Vec<u8>>, AppError> {
    let ffmpeg = match find_command_binary("ffmpeg") {
        Some(path) => path,
        None => return Ok(None),
    };

    let seek_time = probe_video_seek_seconds(path).unwrap_or(1.0).max(0.0);
    let temp_output = temp_jpeg_path(path, working_dir);
    let _ = fs::remove_file(&temp_output);
    let status = wait_for_status_with_timeout(
        Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{seek_time:.3}"))
        .arg("-i")
        .arg(path)
        .arg("-frames:v")
        .arg("1")
        .arg("-y")
        .arg(&temp_output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?,
        VIDEO_THUMBNAIL_TIMEOUT,
        "ffmpeg thumbnail render",
    )?;

    if !status.success() || !temp_output.is_file() {
        let _ = fs::remove_file(&temp_output);
        return Ok(None);
    }

    let bytes = fs::read(&temp_output)?;
    let _ = fs::remove_file(&temp_output);
    let image = image::load_from_memory(&bytes)?;
    Ok(Some(encode_square_thumbnail_to_jpeg(
        &image,
        size,
        allow_upscale,
        thumbnail_output_quality(size),
    )?))
}

fn load_image(
    path: &Path,
    size_hint: u32,
    allow_upscale: bool,
    working_dir: &Path,
) -> Result<image::DynamicImage, AppError> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    match decode_with_orientation(reader) {
        Ok(image) => Ok(image),
        Err(error) => {
            let extension = path
                .extension()
                .and_then(|item| item.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if matches!(extension.as_str(), "heic" | "heif") {
                if let Some(image) = load_image_with_sips(path, size_hint, allow_upscale, working_dir)? {
                    return Ok(image);
                }
            }
            Err(AppError::Image(error))
        }
    }
}

fn decode_with_orientation<R: BufRead + Seek>(
    reader: ImageReader<R>,
) -> Result<DynamicImage, image::ImageError> {
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn probe_video_seek_seconds(path: &Path) -> Option<f64> {
    let duration = probe_media_duration_ms(path).ok().flatten()? as f64 / 1000.0;
    if duration <= 0.0 {
        return Some(0.0);
    }

    Some(if duration < 1.0 {
        0.0
    } else {
        (duration * 0.1).clamp(0.5, 2.0)
    })
}

fn use_high_quality_thumbnail_settings(size: u32) -> bool {
    size <= HIGH_QUALITY_THUMB_MAX_SIZE
}

fn thumbnail_output_quality(size: u32) -> u8 {
    if use_high_quality_thumbnail_settings(size) {
        HIGH_QUALITY_THUMB_JPEG_QUALITY
    } else {
        DEFAULT_THUMBNAIL_JPEG_QUALITY
    }
}

fn thumbnail_source_quality(size: u32) -> u8 {
    if use_high_quality_thumbnail_settings(size) {
        HIGH_QUALITY_THUMB_SOURCE_JPEG_QUALITY
    } else {
        DEFAULT_THUMBNAIL_SOURCE_JPEG_QUALITY
    }
}

fn thumbnail_render_size(size: u32) -> u32 {
    let requested_size = size.max(1);
    if use_high_quality_thumbnail_settings(requested_size) {
        requested_size
            .saturating_mul(2)
            .clamp(requested_size, requested_size.max(512))
    } else {
        requested_size.max(192)
    }
}

fn find_command_binary(name: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn normalized_extension(path: &Path) -> String {
    path.extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

fn is_video_path(path: &Path) -> bool {
    is_video_extension(&normalized_extension(path))
}

fn is_video_extension(extension: &str) -> bool {
    matches!(extension, "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm")
}

fn load_image_with_sips(
    path: &Path,
    size_hint: u32,
    allow_upscale: bool,
    working_dir: &Path,
) -> Result<Option<image::DynamicImage>, AppError> {
    #[cfg(target_os = "macos")]
    {
        if let Some(bytes) = render_with_sips(
            path,
            size_hint.max(512),
            allow_upscale,
            90,
            working_dir,
        )? {
            let image = image::load_from_memory(&bytes)?;
            return Ok(Some(image));
        }
        Ok(None)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, size_hint, working_dir);
        Ok(None)
    }
}

fn probe_image_dimensions_with_sips(path: &Path) -> Result<Option<(u32, u32)>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let output = wait_for_output_with_timeout(
            Command::new("sips")
                .arg("-g")
                .arg("pixelWidth")
                .arg("-g")
                .arg("pixelHeight")
                .arg(path)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "sips image dimension probe",
        )?;

        if !output.status.success() {
            return Ok(None);
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let width = text
            .lines()
            .find_map(|line| line.split_once("pixelWidth: ").and_then(|(_, value)| value.trim().parse::<u32>().ok()));
        let height = text
            .lines()
            .find_map(|line| line.split_once("pixelHeight: ").and_then(|(_, value)| value.trim().parse::<u32>().ok()));

        Ok(width.zip(height))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(None)
    }
}

fn render_square_thumbnail_with_sips(
    path: &Path,
    size: u32,
    allow_upscale: bool,
    quality: u8,
    working_dir: &Path,
) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let Some((source_width, source_height)) = probe_image_dimensions_with_sips(path)? else {
            return Ok(None);
        };
        let size = if allow_upscale {
            size.max(1)
        } else {
            size.max(1).min(source_width.min(source_height).max(1))
        };
        let temp_path = temp_jpeg_path(path, working_dir);
        let _ = fs::remove_file(&temp_path);

        let (resize_flag, resize_value, offset_y, offset_x) = if source_width >= source_height {
            let target_width = ((source_width as u64 * size as u64) + source_height as u64 - 1)
                / source_height as u64;
            (
                "--resampleHeight",
                size.to_string(),
                0_u32,
                ((target_width as u32).saturating_sub(size)) / 2,
            )
        } else {
            let target_height = ((source_height as u64 * size as u64) + source_width as u64 - 1)
                / source_width as u64;
            (
                "--resampleWidth",
                size.to_string(),
                ((target_height as u32).saturating_sub(size)) / 2,
                0_u32,
            )
        };

        let status = wait_for_status_with_timeout(
            Command::new("sips")
                .arg("-s")
                .arg("format")
                .arg("jpeg")
                .arg("-s")
                .arg("formatOptions")
                .arg(quality.to_string())
                .arg(resize_flag)
                .arg(resize_value)
                .arg(path)
                .arg("--out")
                .arg(&temp_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "sips square thumbnail resample",
        )?;

        if !status.success() || !temp_path.is_file() {
            let _ = fs::remove_file(&temp_path);
            return Ok(None);
        }

        let crop_status = wait_for_status_with_timeout(
            Command::new("sips")
                .arg("-c")
                .arg(size.to_string())
                .arg(size.to_string())
                .arg("--cropOffset")
                .arg(offset_y.to_string())
                .arg(offset_x.to_string())
                .arg(&temp_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "sips square thumbnail crop",
        )?;

        if !crop_status.success() {
            let _ = fs::remove_file(&temp_path);
            return Ok(None);
        }

        let bytes = fs::read(&temp_path)?;
        let _ = fs::remove_file(&temp_path);
        return Ok(Some(bytes));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, size, quality, working_dir);
        Ok(None)
    }
}

fn render_with_sips(
    path: &Path,
    width: u32,
    allow_upscale: bool,
    quality: u8,
    working_dir: &Path,
) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let width = if allow_upscale {
            width.max(1)
        } else if let Some((source_width, source_height)) = probe_image_dimensions_with_sips(path)? {
            width.max(1).min(source_width.max(source_height).max(1))
        } else {
            width.max(1)
        };
        let temp_path = temp_jpeg_path(path, working_dir);
        let status = wait_for_status_with_timeout(
            Command::new("sips")
            .arg("-s")
            .arg("format")
            .arg("jpeg")
            .arg("-s")
            .arg("formatOptions")
            .arg(quality.to_string())
            .arg("-Z")
            .arg(width.to_string())
            .arg(path)
            .arg("--out")
            .arg(&temp_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "sips thumbnail render",
        )?;

        if !status.success() {
            let _ = fs::remove_file(&temp_path);
            return Ok(None);
        }

        let bytes = fs::read(&temp_path)?;
        let _ = fs::remove_file(&temp_path);
        return Ok(Some(bytes));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, width, allow_upscale, quality, working_dir);
        Ok(None)
    }
}

fn render_with_sips_original(path: &Path, quality: u8, working_dir: &Path) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let temp_path = temp_jpeg_path(path, working_dir);
        let status = wait_for_status_with_timeout(
            Command::new("sips")
            .arg("-s")
            .arg("format")
            .arg("jpeg")
            .arg("-s")
            .arg("formatOptions")
            .arg(quality.to_string())
            .arg(path)
            .arg("--out")
            .arg(&temp_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "sips viewer render",
        )?;

        if !status.success() {
            let _ = fs::remove_file(&temp_path);
            return Ok(None);
        }

        let bytes = fs::read(&temp_path)?;
        let _ = fs::remove_file(&temp_path);
        return Ok(Some(bytes));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, quality, working_dir);
        Ok(None)
    }
}

fn temp_jpeg_path(path: &Path, working_dir: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or("thumb");
    let _ = fs::create_dir_all(working_dir);
    working_dir.join(format!("mypicasa-{stem}-{stamp}.jpg"))
}

fn render_center_square(image: &DynamicImage, size: u32, allow_upscale: bool) -> DynamicImage {
    let target = if allow_upscale {
        size.max(1)
    } else {
        size.max(1).min(image.width().min(image.height()).max(1))
    };
    image.resize_to_fill(target, target, FilterType::Lanczos3)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, AppError> {
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder.encode_image(image)?;
    Ok(buffer)
}

fn encode_square_thumbnail_to_jpeg(
    image: &DynamicImage,
    size: u32,
    allow_upscale: bool,
    quality: u8,
) -> Result<Vec<u8>, AppError> {
    let thumb = render_center_square(image, size.max(1), allow_upscale);
    encode_jpeg(&thumb, quality)
}

fn normalize_image_bytes_to_square_jpeg(
    bytes: &[u8],
    size: u32,
    allow_upscale: bool,
    quality: u8,
) -> Result<Vec<u8>, AppError> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let image = decode_with_orientation(reader)?;
    encode_square_thumbnail_to_jpeg(&image, size, allow_upscale, quality)
}

fn wait_for_status_with_timeout(
    mut child: Child,
    timeout: Duration,
    label: &str,
) -> Result<std::process::ExitStatus, AppError> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Message(format!(
                "{label} timed out after {} ms",
                timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_output_with_timeout(
    mut child: Child,
    timeout: Duration,
    label: &str,
) -> Result<Output, AppError> {
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map_err(AppError::from);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Message(format!(
                "{label} timed out after {} ms",
                timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
