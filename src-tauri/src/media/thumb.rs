use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{ImageReader, codecs::jpeg::JpegEncoder};

use crate::util::errors::AppError;

pub fn generate_thumbnail(path: &Path, size: u32) -> Result<Option<Vec<u8>>, AppError> {
    if is_video_path(path) {
        return render_video_thumbnail_with_ffmpeg(path, size);
    }

    let extension = normalized_extension(path);

    #[cfg(target_os = "macos")]
    {
        if matches!(extension.as_str(), "heic" | "heif") {
            if let Some(bytes) = render_thumbnail_with_quicklook(path, size.max(192))? {
                return Ok(Some(bytes));
            }
        }
        if let Some(bytes) = render_with_sips(path, size.max(192), 82)? {
            return Ok(Some(bytes));
        }
    }

    let image = load_image(path, size)?;
    let thumb = image.thumbnail(size, size);
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 82);
    encoder.encode_image(&thumb)?;
    Ok(Some(buffer))
}

pub fn generate_viewer_image(path: &Path, max_dimension: u32) -> Result<Option<Vec<u8>>, AppError> {
    let extension = normalized_extension(path);
    if is_video_extension(&extension) {
        return Ok(None);
    }

    if matches!(extension.as_str(), "heic" | "heif") {
        if let Some(bytes) = render_with_sips_original(path, 90)? {
            return Ok(Some(bytes));
        }
    }

    let image = load_image(path, max_dimension)?;
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

pub fn generate_viewer_image_file(path: &Path, max_dimension: u32) -> Result<Option<PathBuf>, AppError> {
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
    let output_path = std::env::temp_dir().join(format!(
        "my-picasa-viewer-{}.jpg",
        blake3::hash(cache_key.as_bytes()).to_hex()
    ));

    if output_path.is_file() {
        return Ok(Some(output_path));
    }

    let Some(bytes) = generate_viewer_image(path, max_dimension)? else {
        return Ok(None);
    };
    let temp_output = output_path.with_extension("tmp.jpg");
    let _ = fs::remove_file(&temp_output);
    fs::write(&temp_output, bytes)?;
    fs::rename(&temp_output, &output_path)?;
    Ok(Some(output_path))
}

pub fn generate_viewer_video(path: &Path) -> Result<Option<PathBuf>, AppError> {
    if !is_video_path(path) {
        return Ok(None);
    }

    let ffmpeg = match find_command_binary("ffmpeg") {
        Some(path) => path,
        None => return Ok(None),
    };

    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    let cache_key = format!("{}:{}:{}", path.display(), metadata.len(), modified);
    let output_path = std::env::temp_dir().join(format!(
        "my-picasa-viewer-{}.mp4",
        blake3::hash(cache_key.as_bytes()).to_hex()
    ));

    if output_path.is_file() {
        return Ok(Some(output_path));
    }

    let temp_output = output_path.with_extension("tmp.mp4");
    let _ = fs::remove_file(&temp_output);

    let status = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(path)
        .arg("-movflags")
        .arg("+faststart")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-vcodec")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("22")
        .arg("-acodec")
        .arg("aac")
        .arg("-b:a")
        .arg("160k")
        .arg(&temp_output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() || !temp_output.is_file() {
        let _ = fs::remove_file(&temp_output);
        return Ok(None);
    }

    fs::rename(&temp_output, &output_path)?;
    Ok(Some(output_path))
}

pub fn viewer_render_cache_stats() -> Result<(u32, u64), AppError> {
    let temp_dir = std::env::temp_dir();
    let mut items = 0_u32;
    let mut bytes = 0_u64;

    if let Ok(entries) = fs::read_dir(temp_dir) {
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

pub fn clear_viewer_render_cache() -> Result<(), AppError> {
    let temp_dir = std::env::temp_dir();
    if let Ok(entries) = fs::read_dir(temp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with("my-picasa-viewer-") {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(())
}

pub fn probe_media_duration_ms(path: &Path) -> Result<Option<i64>, AppError> {
    if !is_video_path(path) {
        return Ok(None);
    }

    let Some(ffprobe) = find_command_binary("ffprobe") else {
        return Ok(None);
    };

    let output = Command::new(ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

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

fn render_video_thumbnail_with_ffmpeg(path: &Path, size: u32) -> Result<Option<Vec<u8>>, AppError> {
    let ffmpeg = match find_command_binary("ffmpeg") {
        Some(path) => path,
        None => return Ok(None),
    };

    let seek_time = probe_video_seek_seconds(path).unwrap_or(1.0).max(0.0);
    let output = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{seek_time:.3}"))
        .arg("-i")
        .arg(path)
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("mjpeg")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }

    let image = image::load_from_memory(&output.stdout)?;
    let thumb = image.thumbnail(size, size);
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 82);
    encoder.encode_image(&thumb)?;
    Ok(Some(buffer))
}

fn load_image(path: &Path, size_hint: u32) -> Result<image::DynamicImage, AppError> {
    match ImageReader::open(path)?.with_guessed_format()?.decode() {
        Ok(image) => Ok(image),
        Err(error) => {
            let extension = path
                .extension()
                .and_then(|item| item.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if matches!(extension.as_str(), "heic" | "heif") {
                if let Some(image) = load_image_with_sips(path, size_hint)? {
                    return Ok(image);
                }
            }
            Err(AppError::Image(error))
        }
    }
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
) -> Result<Option<image::DynamicImage>, AppError> {
    #[cfg(target_os = "macos")]
    {
        if let Some(bytes) = render_with_sips(path, size_hint.max(512), 90)? {
            let image = image::load_from_memory(&bytes)?;
            return Ok(Some(image));
        }
        Ok(None)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, size_hint);
        Ok(None)
    }
}

fn render_with_sips(path: &Path, width: u32, quality: u8) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let temp_path = temp_jpeg_path(path);
        let status = Command::new("sips")
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
            .status()?;

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
        let _ = (path, width, quality);
        Ok(None)
    }
}

fn render_with_sips_original(path: &Path, quality: u8) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let temp_path = temp_jpeg_path(path);
        let status = Command::new("sips")
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
            .status()?;

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
        let _ = (path, quality);
        Ok(None)
    }
}

fn render_thumbnail_with_quicklook(path: &Path, width: u32) -> Result<Option<Vec<u8>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let output_dir = temp_render_dir(path);
        fs::create_dir_all(&output_dir)?;

        let status = Command::new("qlmanage")
            .arg("-t")
            .arg("-s")
            .arg(width.to_string())
            .arg("-o")
            .arg(&output_dir)
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

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
        let _ = (path, width);
        Ok(None)
    }
}

fn temp_jpeg_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or("thumb");
    std::env::temp_dir().join(format!("mypicasa-{stem}-{stamp}.jpg"))
}

fn temp_render_dir(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or("thumb");
    std::env::temp_dir().join(format!("mypicasa-ql-{stem}-{stamp}"))
}
