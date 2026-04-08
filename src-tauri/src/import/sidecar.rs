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
    let title_hint = value.get("title").and_then(Value::as_str).map(ToOwned::to_owned);

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
    let people_json = value.get("people").map(Value::to_string);

    let json_kind = if value.get("archive").is_some() || value.get("title").is_some() {
        "media_sidecar"
    } else if value.get("albumData").is_some() {
        "album"
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
        json_kind: json_kind.to_string(),
        guessed_target_stem: sidecar_target_stem(Path::new(&scan.path)),
    }))
}

fn sidecar_target_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|item| item.to_str())
        .map(|stem| stem.trim_end_matches(".supplemental-metadata").to_string())
}
