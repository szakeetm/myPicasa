use std::collections::HashSet;

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
                unmatched.push(scan.path.clone());
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

    for path in unmatched {
        db.add_diagnostic(
            import_id,
            "warning",
            "ambiguous_json_target",
            Some(&path),
            "could not resolve JSON sidecar target",
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
