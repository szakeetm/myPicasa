use std::{io::Cursor, path::Path};

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

    let reader = ImageReader::open(path)?;
    let image = reader.decode()?;
    let thumb = image.thumbnail(size, size);
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 82);
    encoder.encode_image(&thumb)?;
    Ok(Some(buffer))
}
