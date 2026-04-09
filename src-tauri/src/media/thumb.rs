use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use image::{DynamicImage, ImageDecoder, ImageReader, codecs::jpeg::JpegEncoder};

use crate::util::errors::AppError;

const EXTERNAL_TOOL_TIMEOUT: Duration = Duration::from_secs(12);
const VIDEO_THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(30);
pub const VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT: Duration = Duration::from_secs(30);

pub fn thumbnail_generator_label(path: &Path) -> &'static str {
    if is_video_path(path) {
        return "ffmpeg";
    }

    let extension = normalized_extension(path);

    #[cfg(target_os = "macos")]
    {
        if matches!(extension.as_str(), "heic" | "heif") {
            return "quicklook_or_sips";
        }
        return "sips_or_rust";
    }

    #[allow(unreachable_code)]
    "rust"
}

pub fn generate_thumbnail(path: &Path, size: u32, working_dir: &Path) -> Result<Option<Vec<u8>>, AppError> {
    if is_video_path(path) {
        return render_video_thumbnail_with_ffmpeg(path, size, working_dir);
    }

    let extension = normalized_extension(path);

    #[cfg(target_os = "macos")]
    {
        if matches!(extension.as_str(), "heic" | "heif") {
            if let Some(bytes) = render_thumbnail_with_quicklook(path, size.max(192), working_dir)? {
                return Ok(Some(normalize_image_bytes_to_jpeg(&bytes, 82)?));
            }
        }
        if let Some(bytes) = render_with_sips(path, size.max(192), 82, working_dir)? {
            return Ok(Some(normalize_image_bytes_to_jpeg(&bytes, 82)?));
        }
    }

    let image = load_image(path, size, working_dir)?;
    let thumb = image.thumbnail(size, size);
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 82);
    encoder.encode_image(&thumb)?;
    Ok(Some(buffer))
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

    let image = load_image(path, max_dimension, working_dir)?;
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

fn probe_video_dimensions(path: &Path) -> Result<Option<(u32, u32)>, AppError> {
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
    let thumb = image.thumbnail(size, size);
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 82);
    encoder.encode_image(&thumb)?;
    Ok(Some(buffer))
}

fn load_image(path: &Path, size_hint: u32, working_dir: &Path) -> Result<image::DynamicImage, AppError> {
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
                if let Some(image) = load_image_with_sips(path, size_hint, working_dir)? {
                    return Ok(image);
                }
            }
            Err(AppError::Image(error))
        }
    }
}

fn decode_with_orientation(reader: ImageReader<std::io::BufReader<std::fs::File>>) -> Result<DynamicImage, image::ImageError> {
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
    working_dir: &Path,
) -> Result<Option<image::DynamicImage>, AppError> {
    #[cfg(target_os = "macos")]
    {
        if let Some(bytes) = render_with_sips(path, size_hint.max(512), 90, working_dir)? {
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

fn render_with_sips(path: &Path, width: u32, quality: u8, working_dir: &Path) -> Result<Option<Vec<u8>>, AppError> {
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
        let _ = (path, width, quality, working_dir);
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

fn render_thumbnail_with_quicklook(path: &Path, width: u32, working_dir: &Path) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let output_dir = temp_render_dir(path, working_dir);
        fs::create_dir_all(&output_dir)?;

        let status = wait_for_status_with_timeout(
            Command::new("qlmanage")
            .arg("-t")
            .arg("-s")
            .arg(width.to_string())
            .arg("-o")
            .arg(&output_dir)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
            EXTERNAL_TOOL_TIMEOUT,
            "qlmanage thumbnail render",
        )?;

        if !status.success() {
            let _ = fs::remove_dir_all(&output_dir);
            return Ok(None);
        }

        let generated_file = fs::read_dir(&output_dir)?
            .filter_map(Result::ok)
            .find_map(|entry| {
                let path = entry.path();
                if path.is_file() { Some(path) } else { None }
            });

        let Some(generated_file) = generated_file else {
            let _ = fs::remove_dir_all(&output_dir);
            return Ok(None);
        };

        let bytes = fs::read(&generated_file)?;
        let _ = fs::remove_dir_all(&output_dir);
        return Ok(Some(bytes));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, width, working_dir);
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

fn temp_render_dir(path: &Path, working_dir: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or("thumb");
    let _ = fs::create_dir_all(working_dir);
    working_dir.join(format!("mypicasa-ql-{stem}-{stamp}"))
}

fn normalize_image_bytes_to_jpeg(bytes: &[u8], quality: u8) -> Result<Vec<u8>, AppError> {
    let image = image::load_from_memory(bytes)?;
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder.encode_image(&image)?;
    Ok(buffer)
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
