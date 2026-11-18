use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
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

pub(super) struct IndexedFile {
    pub parent_path: String,
    pub size: i64,
    pub mtime: String,
    pub detected_format: Option<String>,
    pub quick_hash: Option<Vec<u8>>,
    pub complete: bool,
    pub deleted: bool,
    pub video_duration: Option<i64>,
}

pub(super) fn scan_roots_with_cancel(
    roots: &[String],
    indexed: &HashMap<String, IndexedFile>,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: impl FnMut(u32),
) -> Result<Vec<FileScanRecord>, AppError> {
    let mut file_paths = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut scanned_count = 0_u32;

    for root in roots {
        if cancel_flag.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(AppError::RefreshCancelled);
        }
        let normalized_root = normalize_path(Path::new(root));
        if !fs::metadata(root).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(AppError::Message(format!(
                "Takeout root is unavailable or is not a directory: {root}"
            )));
        }
        for entry in WalkDir::new(root).follow_links(false) {
            if cancel_flag.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                return Err(AppError::RefreshCancelled);
            }
            let entry =
                entry.map_err(|err| AppError::Message(format!("Could not scan {root}: {err}")))?;
            if !(entry.file_type().is_file()
                || entry.file_type().is_symlink() && entry.path().is_file())
                || !seen_paths.insert(normalize_path(entry.path()))
            {
                continue;
            }
            file_paths.push((normalized_root.clone(), entry.into_path()));
            scanned_count = scanned_count.saturating_add(1);
            if scanned_count == 1 || scanned_count % 200 == 0 {
                on_progress(scanned_count);
            }
        }
    }
    on_progress(scanned_count);

    let records = file_paths
        .into_par_iter()
        .map(
            |(normalized_root, path)| -> Result<FileScanRecord, AppError> {
                if cancel_flag.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    return Err(AppError::RefreshCancelled);
                }
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
                    .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
                    .unwrap_or_else(crate::util::time::utc_now);

                let ctime_utc = metadata
                    .created()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| epoch_to_utc(duration.as_secs() as i64));

                let normalized_path = normalize_path(&path);
                let previous = indexed
                    .get(&normalized_path)
                    .filter(|entry| entry.size == file_size as i64 && entry.mtime == mtime_utc);
                let (detected_format, quick_hash) = if let Some(previous) = previous {
                    (
                        previous.detected_format.clone(),
                        previous.quick_hash.clone(),
                    )
                } else {
                    let format = detect_format(&path, &extension)?;
                    let hash = if matches!(candidate_type, "photo" | "video") {
                        Some(quick_hash(&path, file_size)?)
                    } else {
                        None
                    };
                    (format, hash)
                };
                let mime_type = MimeGuess::from_path(&path)
                    .first_raw()
                    .map(ToOwned::to_owned)
                    .or_else(|| detected_format.clone());

                Ok(FileScanRecord {
                    path: normalized_path,
                    root_path: normalized_root.clone(),
                    parent_path: normalize_path(
                        path.parent().unwrap_or(Path::new(&normalized_root)),
                    ),
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
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    Ok(records)
}

fn classify(extension: &Option<String>) -> &'static str {
    match extension.as_deref() {
        Some("jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "gif") => "photo",
        Some("mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" | "mpg" | "mpeg") => "video",
        Some("json") => "json",
        _ => "other",
    }
}

fn detect_format(path: &Path, extension: &Option<String>) -> Result<Option<String>, AppError> {
    let mut bytes = Vec::with_capacity(12);
    fs::File::open(path)?.take(12).read_to_end(&mut bytes)?;
    let format = (|| {
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
        extension.clone()
    })();
    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_root_is_a_scan_error() {
        let path =
            std::env::temp_dir().join(format!("mypicasa-missing-scan-root-{}", std::process::id()));
        assert!(
            scan_roots_with_cancel(&[normalize_path(&path)], &HashMap::new(), None, |_| {})
                .is_err()
        );
    }
}
