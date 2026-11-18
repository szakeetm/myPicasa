use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{
    db::DatabaseQueries,
    models::{FileScanRecord, ParsedSidecar},
    util::errors::AppError,
};

pub fn validate_import_with_progress<F>(
    db: &crate::db::Database,
    import_id: i64,
    scans: &[FileScanRecord],
    sidecars_by_path: &HashMap<&str, &ParsedSidecar>,
    dirty_folders: &HashSet<&str>,
    mut on_progress: F,
) -> Result<(), AppError>
where
    F: FnMut(usize, usize, bool) -> Result<(), AppError>,
{
    let mut sidecars = 0_usize;
    let mut unmatched = Vec::new();
    let total = scans.len();

    for (index, scan) in scans.iter().enumerate() {
        let processed = index + 1;
        on_progress(
            processed,
            total,
            processed == 1 || processed == total || processed % 100 == 0,
        )?;
        if scan.candidate_type == "json" && dirty_folders.contains(scan.parent_path.as_str()) {
            sidecars += 1;
            let sidecar = sidecars_by_path.get(scan.path.as_str()).copied();
            if sidecar
                .as_ref()
                .map(|item| item.json_kind == "album")
                .unwrap_or(false)
            {
                continue;
            }
            let candidate_names = build_candidate_names(&scan.path, sidecar);
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
