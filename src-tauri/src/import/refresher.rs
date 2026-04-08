use std::collections::HashMap;
use std::time::Instant;

use rayon::current_num_threads;
use tracing::info;

use crate::{
    app::state::AppState,
    db::DatabaseQueries,
    import::{
        scanner::scan_roots,
        sidecar::{parse_sidecar, takeout_match_score},
        validator::validate_import,
    },
    models::{ImportProgress, ParsedSidecar, RefreshRequest},
    util::errors::AppError,
};

pub fn refresh_takeout_index(
    state: &AppState,
    request: RefreshRequest,
) -> Result<ImportProgress, AppError> {
    let started = Instant::now();
    let roots = request
        .roots
        .into_iter()
        .filter(|item| !item.trim().is_empty())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(AppError::Message(
            "at least one takeout root is required".to_string(),
        ));
    }

    state.db.insert_log(
        "info",
        "import",
        &format!("starting refresh for {} roots", roots.len()),
        None,
    )?;
    info!("refresh_takeout_index: starting for {} roots", roots.len());
    println!("refresh_takeout_index: starting for {} roots", roots.len());

    let import_id = state.db.create_import(&roots.join("; "))?;
    let mut progress = ImportProgress {
        import_id,
        status: "running".to_string(),
        phase: "scanning".to_string(),
        files_scanned: 0,
        processed_files: 0,
        total_files: 0,
        files_added: 0,
        files_updated: 0,
        files_deleted: 0,
        assets_added: 0,
        assets_updated: 0,
        assets_deleted: 0,
        worker_count: current_num_threads() as u32,
        message: Some("scan in progress".to_string()),
    };
    *state.import_status.lock() = Some(progress.clone());

    let scan_started = Instant::now();
    let scans = scan_roots(&roots)?;
    info!(
        "refresh_takeout_index: scanned {} files in {} ms",
        scans.len(),
        scan_started.elapsed().as_millis()
    );
    println!(
        "refresh_takeout_index: scanned {} files in {} ms",
        scans.len(),
        scan_started.elapsed().as_millis()
    );
    progress.files_scanned = scans.len() as u32;
    progress.total_files = scans.len() as u32;
    progress.message = Some(format!("scanned {} files", progress.files_scanned));
    *state.import_status.lock() = Some(progress.clone());

    let mut sidecars_by_parent = HashMap::<String, Vec<(String, ParsedSidecar)>>::new();
    for scan in &scans {
        if let Ok(Some(sidecar)) = parse_sidecar(scan) {
            sidecars_by_parent
                .entry(scan.parent_path.clone())
                .or_default()
                .push((scan.path.clone(), sidecar));
        }
    }

    progress.phase = "indexing".to_string();
    progress.message = Some(format!(
        "indexing {} files with {} workers",
        progress.total_files, progress.worker_count
    ));
    *state.import_status.lock() = Some(progress.clone());

    for (index, scan) in scans.iter().enumerate() {
        let file_id = state.db.upsert_file_entry(import_id, scan)?;
        let album_id = state.db.upsert_album(&scan.parent_path)?;
        if scan.candidate_type == "json" || scan.candidate_type == "other" {
            progress.processed_files = (index + 1) as u32;
            if should_publish_progress(index, scans.len()) {
                progress.message = Some(progress_message(&progress));
                *state.import_status.lock() = Some(progress.clone());
            }
            continue;
        }

        let sidecar = find_sidecar_for(scan, &sidecars_by_parent);
        let asset_id = state.db.upsert_asset_for_file(file_id, scan, sidecar)?;
        state.db.attach_asset_file(asset_id, file_id, "primary")?;
        state.db.attach_album_asset(album_id, asset_id)?;

        if let Some(sidecar) = sidecar {
            state.db.set_sidecar_metadata(asset_id, None, sidecar)?;
        }

        if is_live_photo_video(scan) {
            if let Some(still_asset_id) = find_still_pair_asset_id(state, scan)? {
                state
                    .db
                    .attach_asset_file(still_asset_id, file_id, "live_photo_video")?;
            }
        }

        state.db.replace_search_row(asset_id)?;

        progress.processed_files = (index + 1) as u32;
        if should_publish_progress(index, scans.len()) {
            progress.message = Some(progress_message(&progress));
            *state.import_status.lock() = Some(progress.clone());
        }
    }

    progress.phase = "validating".to_string();
    progress.message = Some("running ingress validation".to_string());
    *state.import_status.lock() = Some(progress.clone());

    progress.files_deleted = state.db.soft_delete_missing_files(import_id, &roots)?;
    validate_import(&state.db, import_id, &scans)?;

    progress.status = "completed".to_string();
    progress.phase = "completed".to_string();
    progress.processed_files = progress.total_files;
    progress.message = Some(format!(
        "refresh completed in {} ms",
        started.elapsed().as_millis()
    ));
    state.db.finish_import(&progress)?;
    state.db.insert_log(
        "info",
        "import",
        &format!("completed import {}", import_id),
        None,
    )?;
    info!(
        import_id,
        scanned = progress.files_scanned,
        elapsed_ms = started.elapsed().as_millis(),
        "refresh completed"
    );
    println!(
        "refresh_takeout_index: import {} completed in {} ms",
        import_id,
        started.elapsed().as_millis()
    );

    *state.import_status.lock() = Some(progress.clone());
    Ok(progress)
}

fn should_publish_progress(index: usize, total: usize) -> bool {
    index == 0 || index + 1 == total || (index + 1) % 100 == 0
}

fn progress_message(progress: &ImportProgress) -> String {
    if progress.total_files == 0 {
        return "indexing files".to_string();
    }
    let percent = (progress.processed_files as f64 / progress.total_files as f64) * 100.0;
    format!(
        "indexed {} / {} files ({percent:.1}%) using {} workers",
        progress.processed_files, progress.total_files, progress.worker_count
    )
}

fn find_sidecar_for<'a>(
    scan: &crate::models::FileScanRecord,
    sidecars_by_parent: &'a HashMap<String, Vec<(String, ParsedSidecar)>>,
) -> Option<&'a ParsedSidecar> {
    let parent = std::path::Path::new(&scan.path).parent()?.to_str()?;
    let folder_sidecars = sidecars_by_parent.get(parent)?;

    for candidate_filename in sidecar_candidate_filenames(&scan.filename) {
        let candidate_path = format!("{parent}/{candidate_filename}");
        let json_path = format!("{candidate_path}.json");
        if let Some((_, sidecar)) = folder_sidecars.iter().find(|(path, _)| path == &json_path) {
            return Some(sidecar);
        }

        let supplemental = format!("{candidate_path}.supplemental-metadata.json");
        if let Some((_, sidecar)) = folder_sidecars
            .iter()
            .find(|(path, _)| path == &supplemental)
        {
            return Some(sidecar);
        }
    }

    folder_sidecars
        .iter()
        .filter_map(|(_, sidecar)| {
            score_sidecar_match(&scan.filename, sidecar).map(|score| (score, sidecar))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, sidecar)| sidecar)
}

fn sidecar_candidate_filenames(filename: &str) -> Vec<String> {
    let mut candidates = vec![filename.to_string()];

    if let Some(unedited) = strip_edited_marker(filename) {
        candidates.push(unedited);
    }

    candidates
}

fn strip_edited_marker(filename: &str) -> Option<String> {
    let path = std::path::Path::new(filename);
    let stem = path.file_stem()?.to_str()?;
    let extension = path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default();

    let normalized = if let Some(stripped) = stem.strip_suffix("-edited") {
        stripped.to_string()
    } else if let Some(index) = stem.rfind("-edited(") {
        if stem.ends_with(')') {
            stem[..index].to_string()
        } else {
            return None;
        }
    } else {
        return None;
    };

    if extension.is_empty() {
        Some(normalized)
    } else {
        Some(format!("{normalized}.{extension}"))
    }
}

fn score_sidecar_match(filename: &str, sidecar: &ParsedSidecar) -> Option<usize> {
    let mut best = sidecar
        .guessed_target_stem
        .as_deref()
        .and_then(|candidate| takeout_match_score(filename, candidate));

    if let Some(title_hint) = sidecar.title_hint.as_deref() {
        let title_score = takeout_match_score(filename, title_hint);
        best = match (best, title_score) {
            (Some(current), Some(candidate)) => Some(current.max(candidate)),
            (None, Some(candidate)) => Some(candidate),
            (Some(current), None) => Some(current),
            (None, None) => None,
        };
    }

    best
}

fn is_live_photo_video(scan: &crate::models::FileScanRecord) -> bool {
    let lowercase = scan.filename.to_lowercase();
    scan.candidate_type == "video" && (lowercase.contains("motion") || lowercase.contains("live"))
}

fn find_still_pair_asset_id(
    state: &AppState,
    scan: &crate::models::FileScanRecord,
) -> Result<Option<i64>, AppError> {
    let still_stem = scan
        .filename
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches("-motion")
        .trim_end_matches("-live")
        .to_string();
    state.db.with_connection(|conn| {
        use rusqlite::{OptionalExtension, params};
        conn.query_row(
            "SELECT a.id
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             WHERE f.parent_path = ?1
               AND f.filename LIKE ?2
             LIMIT 1",
            params![scan.parent_path, format!("{still_stem}.%")],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(AppError::from)
    })
}
