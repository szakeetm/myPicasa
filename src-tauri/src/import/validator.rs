use std::collections::HashSet;
use std::path::Path;

use crate::{db::DatabaseQueries, models::FileScanRecord, util::errors::AppError};

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
            if db.resolve_sidecar_target(&scan.path)?.is_none() {
                unmatched.push((scan.path.clone(), guessed_target_name(&scan.path)));
            }
        }
        if let Some(hash) = &scan.quick_hash {
            if !duplicate_hashes.insert(hex(hash)) {
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
