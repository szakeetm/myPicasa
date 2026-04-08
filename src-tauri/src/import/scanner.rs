use std::{
    fs,
    path::Path,
    time::UNIX_EPOCH,
};

use mime_guess::MimeGuess;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::{
    hash::quick_hash::quick_hash,
    models::FileScanRecord,
    util::{errors::AppError, path::normalize_path, time::epoch_to_utc},
};

pub fn scan_roots(roots: &[String]) -> Result<Vec<FileScanRecord>, AppError> {
    let mut file_paths = Vec::new();

    for root in roots {
        let normalized_root = normalize_path(Path::new(root));
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            file_paths.push((normalized_root.clone(), path.to_path_buf()));
        }
    }

    let records = file_paths
        .into_par_iter()
        .map(|(normalized_root, path)| -> Result<FileScanRecord, AppError> {
            let metadata = fs::metadata(&path)?;
            let file_size = metadata.len();
            let extension = path
                .extension()
                .and_then(|item| item.to_str())
                .map(|item| item.to_lowercase());
            let candidate_type = classify(&extension);
            let mtime_utc = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| epoch_to_utc(duration.as_secs() as i64))
                .unwrap_or_else(crate::util::time::utc_now);

            let ctime_utc = metadata
                .created()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| epoch_to_utc(duration.as_secs() as i64));

            let detected_format = detect_format(&path, &extension);
            let mime_type = MimeGuess::from_path(&path)
                .first_raw()
                .map(ToOwned::to_owned)
                .or_else(|| detected_format.clone());

            let quick_hash = if candidate_type != "other" {
                Some(quick_hash(&path, file_size)?)
            } else {
                None
            };

            Ok(FileScanRecord {
                path: normalize_path(&path),
                root_path: normalized_root.clone(),
                parent_path: normalize_path(path.parent().unwrap_or(Path::new(&normalized_root))),
                filename: path
                    .file_name()
                    .and_then(|item| item.to_str())
                    .unwrap_or_default()
                    .to_string(),
                extension,
                detected_format,
                mime_type,
                file_size: file_size as i64,
                mtime_utc,
                ctime_utc,
                candidate_type: candidate_type.to_string(),
                json_kind: None,
                quick_hash,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(records)
}

fn classify(extension: &Option<String>) -> &'static str {
    match extension.as_deref() {
        Some("jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "gif") => "photo",
        Some("mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm") => "video",
        Some("json") => "json",
        _ => "other",
    }
}

fn detect_format(path: &Path, extension: &Option<String>) -> Option<String> {
    if let Ok(bytes) = fs::read(path) {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some("jpeg".to_string());
        }
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Some("png".to_string());
        }
        if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            return Some("webp".to_string());
        }
        if bytes.get(4..8) == Some(b"ftyp") {
            return Some(extension.clone().unwrap_or_else(|| "isobmff".to_string()));
        }
        if bytes.starts_with(b"{") || bytes.starts_with(b"[") {
            return Some("json".to_string());
        }
    }
    extension.clone()
}
