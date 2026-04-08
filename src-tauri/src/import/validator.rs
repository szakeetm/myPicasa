use std::collections::HashSet;
use std::path::Path;

use crate::{
    db::DatabaseQueries, import::sidecar::parse_sidecar, models::FileScanRecord,
    util::errors::AppError,
};

pub fn validate_import(
    db: &crate::db::Database,
    import_id: i64,
    scans: &[FileScanRecord],
) -> Result<(), AppError> {
    let mut sidecars = 0_usize;
    let mut unmatched = Vec::new();
    let mut duplicate_hashes = HashSet::new();

    for scan in scans {
        if scan.candidate_type == "json" {
            sidecars += 1;
            let sidecar = parse_sidecar(scan).ok().flatten();
            if sidecar
                .as_ref()
                .map(|item| item.json_kind == "album")
                .unwrap_or(false)
            {
                continue;
            }
            let candidate_names = build_candidate_names(&scan.path, sidecar.as_ref());
            if db
                .resolve_sidecar_target(&scan.path, &candidate_names)?
                .is_none()
            {
                println!(
                    "ambiguous_json_target path=\"{}\" candidates={:?}",
                    scan.path, candidate_names
                );
                unmatched.push((scan.path.clone(), describe_target_guess(&candidate_names)));
            }
        }
        if let Some(hash) = &scan.quick_hash {
            if !duplicate_hashes.insert(hex(hash))
                && has_unmerged_duplicate_hash(db, hash, &scan.candidate_type)?
            {
                db.add_diagnostic(
                    import_id,
                    "info",
                    "unmerged_duplicate_candidate",
                    Some(&scan.path),
                    "quick hash collision suggests a duplicate candidate",
                )?;
            }
        }
    }

    for (path, target_name) in unmatched {
        db.add_diagnostic(
            import_id,
            "warning",
            "ambiguous_json_target",
            Some(&path),
            &format!("could not resolve JSON sidecar target for {target_name}"),
        )?;
    }

    db.insert_log(
        "info",
        "validator",
        &format!("validated {} scans and {} sidecars", scans.len(), sidecars),
        None,
    )?;

    Ok(())
}

fn has_unmerged_duplicate_hash(
    db: &crate::db::Database,
    hash: &[u8],
    media_kind: &str,
) -> Result<bool, AppError> {
    db.with_connection(|conn| {
        use rusqlite::params;
        let count = conn.query_row(
            "SELECT COUNT(DISTINCT a.id)
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             WHERE a.is_deleted = 0
               AND a.media_kind = ?2
               AND f.quick_hash = ?1",
            params![hash, media_kind],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count > 1)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|item| format!("{item:02x}")).collect()
}

fn guessed_target_name(path: &str) -> String {
    let Some(filename) = Path::new(path).file_name().and_then(|item| item.to_str()) else {
        return "unknown target".to_string();
    };

    filename
        .trim_end_matches(".supplemental-metadata.json")
        .trim_end_matches(".json")
        .to_string()
}

fn build_candidate_names(
    path: &str,
    sidecar: Option<&crate::models::ParsedSidecar>,
) -> Vec<String> {
    let mut candidates = vec![guessed_target_name(path)];
    if let Some(title_hint) = sidecar.and_then(|item| item.title_hint.as_ref()) {
        let trimmed = title_hint.trim().to_string();
        if !trimmed.is_empty() && !candidates.iter().any(|existing| existing == &trimmed) {
            candidates.push(trimmed);
        }
    }
    candidates
}

fn describe_target_guess(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "unknown target".to_string()
    } else if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        format!("{} (sidecar title hint: {})", candidates[0], candidates[1])
    }
}
