use std::{fs, io::Cursor, path::{Path, PathBuf}, process::Command, time::{SystemTime, UNIX_EPOCH}};

use image::{codecs::jpeg::JpegEncoder, ImageReader};

use crate::util::errors::AppError;

pub fn generate_thumbnail(path: &Path, size: u32) -> Result<Option<Vec<u8>>, AppError> {
    let extension = path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if matches!(extension.as_str(), "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm") {
        return Ok(None);
    }

    if matches!(extension.as_str(), "heic" | "heif") {
        if let Some(bytes) = render_thumbnail_with_quicklook(path, size.max(256))? {
            return Ok(Some(bytes));
        }
        if let Some(bytes) = render_with_sips(path, size.max(256), 82)? {
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
    let extension = path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if matches!(extension.as_str(), "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm") {
        return Ok(None);
    }

    if matches!(extension.as_str(), "heic" | "heif") {
        if let Some(bytes) = render_with_sips(path, max_dimension, 90)? {
            return Ok(Some(bytes));
        }
    }

    let image = load_image(path, max_dimension)?;
    let fitted = if image.width() > max_dimension || image.height() > max_dimension {
        image.resize(max_dimension, max_dimension, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };

    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 90);
    encoder.encode_image(&fitted)?;
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

fn load_image_with_sips(path: &Path, size_hint: u32) -> Result<Option<image::DynamicImage>, AppError> {
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
            .arg("--resampleWidth")
            .arg(width.to_string())
            .arg(path)
            .arg("--out")
            .arg(&temp_path)
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
            .status()?;

        if !status.success() {
            let _ = fs::remove_dir_all(&output_dir);
            return Ok(None);
        }

        let generated_file = fs::read_dir(&output_dir)?
            .filter_map(Result::ok)
            .find_map(|entry| {
                let path = entry.path();
                if path.is_file() {
                    Some(path)
                } else {
                    None
                }
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
    let stem = path.file_stem().and_then(|item| item.to_str()).unwrap_or("thumb");
    std::env::temp_dir().join(format!("mypicasa-{stem}-{stamp}.jpg"))
}

fn temp_render_dir(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = path.file_stem().and_then(|item| item.to_str()).unwrap_or("thumb");
    std::env::temp_dir().join(format!("mypicasa-ql-{stem}-{stamp}"))
}
