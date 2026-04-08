use std::collections::HashMap;

use rayon::current_num_threads;
use tauri::State;
use tracing::info;

use crate::{
    app::state::AppState,
    db::DatabaseQueries,
    import::{scanner::scan_roots, sidecar::parse_sidecar, validator::validate_import},
    models::{ImportProgress, ParsedSidecar, RefreshRequest},
    util::errors::AppError,
};

pub fn refresh_takeout_index(
    state: &State<AppState>,
    request: RefreshRequest,
) -> Result<ImportProgress, AppError> {
    let roots = request
        .roots
        .into_iter()
        .filter(|item| !item.trim().is_empty())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(AppError::Message("at least one takeout root is required".to_string()));
    }

    state
        .db
        .insert_log("info", "import", &format!("starting refresh for {} roots", roots.len()), None)?;

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

    let scans = scan_roots(&roots)?;
    progress.files_scanned = scans.len() as u32;
    progress.total_files = scans.len() as u32;
    progress.message = Some(format!("scanned {} files", progress.files_scanned));
    *state.import_status.lock() = Some(progress.clone());

    let mut sidecars = HashMap::<String, ParsedSidecar>::new();
    for scan in &scans {
        if let Ok(Some(sidecar)) = parse_sidecar(scan) {
            sidecars.insert(scan.path.clone(), sidecar);
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

        let sidecar = find_sidecar_for(scan, &sidecars);
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
    progress.message = Some("refresh completed".to_string());
    state.db.finish_import(&progress)?;
    state
        .db
        .insert_log("info", "import", &format!("completed import {}", import_id), None)?;
    info!(import_id, scanned = progress.files_scanned, "refresh completed");

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
    sidecars: &'a HashMap<String, ParsedSidecar>,
) -> Option<&'a ParsedSidecar> {
    let base = scan.path.clone();
    let json_path = format!("{base}.json");
    if let Some(sidecar) = sidecars.get(&json_path) {
        return Some(sidecar);
    }
    let parent = std::path::Path::new(&scan.path).parent()?.to_str()?;
    let stem = std::path::Path::new(&scan.path).file_name()?.to_str()?;
    let supplemental = format!("{parent}/{stem}.supplemental-metadata.json");
    sidecars.get(&supplemental)
}

fn is_live_photo_video(scan: &crate::models::FileScanRecord) -> bool {
    let lowercase = scan.filename.to_lowercase();
    scan.candidate_type == "video" && (lowercase.contains("motion") || lowercase.contains("live"))
}

fn find_still_pair_asset_id(
    state: &State<AppState>,
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
        use rusqlite::{params, OptionalExtension};
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
