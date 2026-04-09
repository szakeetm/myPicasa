use std::{fs, path::Path};

use serde_json::Value;

use crate::{
    models::{FileScanRecord, ParsedSidecar},
    util::errors::AppError,
};

pub fn parse_sidecar(scan: &FileScanRecord) -> Result<Option<ParsedSidecar>, AppError> {
    if scan.candidate_type != "json" {
        return Ok(None);
    }

    let raw = fs::read_to_string(&scan.path)?;
    let value: Value = serde_json::from_str(&raw)?;
    let photo_taken_time_utc = value
        .pointer("/photoTakenTime/timestamp")
        .and_then(Value::as_str)
        .and_then(|item| item.parse::<i64>().ok())
        .map(crate::util::time::epoch_to_utc);
    let title_hint = value
        .get("title")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let geo_lat = value
        .pointer("/geoDataExif/latitude")
        .and_then(Value::as_f64)
        .or_else(|| value.pointer("/geoData/latitude").and_then(Value::as_f64));
    let geo_lon = value
        .pointer("/geoDataExif/longitude")
        .and_then(Value::as_f64)
        .or_else(|| value.pointer("/geoData/longitude").and_then(Value::as_f64));
    let geo_alt = value
        .pointer("/geoDataExif/altitude")
        .and_then(Value::as_f64)
        .or_else(|| value.pointer("/geoData/altitude").and_then(Value::as_f64));

    let google_photos_origin = value
        .get("googlePhotosOrigin")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let google_photos_url = value
        .get("url")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            google_photos_origin
                .as_deref()
                .filter(|item| item.starts_with("https://"))
                .map(ToOwned::to_owned)
        });
    let people_json = value.get("people").map(Value::to_string);

    let is_album_metadata_file = scan.filename.eq_ignore_ascii_case("metadata.json");

    let json_kind = if value.get("albumData").is_some() || is_album_metadata_file {
        "album"
    } else if value.get("archive").is_some() || value.get("title").is_some() {
        "media_sidecar"
    } else {
        "unknown"
    };

    Ok(Some(ParsedSidecar {
        json_raw: raw,
        title_hint,
        photo_taken_time_utc,
        geo_lat,
        geo_lon,
        geo_alt,
        people_json,
        google_photos_origin,
        google_photos_url,
        json_kind: json_kind.to_string(),
        guessed_target_stem: sidecar_target_stem(Path::new(&scan.path)),
    }))
}

fn sidecar_target_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|item| item.to_str())
        .map(|stem| stem.trim_end_matches(".supplemental-metadata").to_string())
}

pub fn normalize_takeout_name(name: &str) -> Option<String> {
    let stem = Path::new(name).file_stem().and_then(|item| item.to_str())?;
    let mut normalized = stem
        .trim_end_matches(".supplemental-metadata")
        .to_lowercase();

    if normalized.ends_with(')') {
        if let Some(index) = normalized.rfind('(') {
            if normalized[index + 1..normalized.len() - 1]
                .chars()
                .all(|item| item.is_ascii_digit())
            {
                normalized.truncate(index);
            }
        }
    }

    for suffix in [
        "_photo_original",
        "_photo_edited",
        "_original",
        "_edited",
        "-edited",
    ] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }

    let squashed = normalized
        .chars()
        .filter(|item| item.is_ascii_alphanumeric())
        .collect::<String>();

    if squashed.is_empty() {
        None
    } else {
        Some(squashed)
    }
}

pub fn takeout_match_score(candidate: &str, target: &str) -> Option<usize> {
    let candidate_normalized = normalize_takeout_name(candidate)?;
    let target_normalized = normalize_takeout_name(target)?;

    if candidate_normalized == target_normalized {
        return Some(candidate_normalized.len() + 32);
    }

    let (shorter, longer) = if candidate_normalized.len() <= target_normalized.len() {
        (&candidate_normalized, &target_normalized)
    } else {
        (&target_normalized, &candidate_normalized)
    };

    if !longer.starts_with(shorter) {
        return None;
    }

    let difference = longer.len().saturating_sub(shorter.len());
    if shorter.len() < 16 || difference > 8 {
        return None;
    }

    Some(shorter.len().saturating_sub(difference))
}
