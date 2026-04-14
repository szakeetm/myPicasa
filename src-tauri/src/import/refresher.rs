use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Instant;

use rayon::current_num_threads;
use tracing::info;

use crate::{
    app::state::{AppState, preview_cache_replacement_keys_for_path, thumbnail_cache_key_for_path},
    db::DatabaseQueries,
    import::{
        scanner::scan_roots_with_cancel,
        sidecar::{parse_sidecar, takeout_match_score},
        validator::validate_import_with_progress,
    },
    media::thumb::{viewer_image_cache_path, viewer_video_cache_path},
    models::{ImportProgress, ParsedSidecar, RefreshRequest},
    util::errors::AppError,
};

const GRID_THUMBNAIL_SIZE: u32 = 210;
const VIEWER_IMAGE_CACHE_SIZE: u32 = 2400;

pub fn refresh_takeout_index(
    state: &AppState,
    request: RefreshRequest,
) -> Result<ImportProgress, AppError> {
    state.refresh_cancel.store(false, Ordering::SeqCst);
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
    let (active_files_before, active_assets_before) = state.db.with_connection(|conn| {
        use rusqlite::OptionalExtension;
        let active_files = conn
            .query_row(
                "SELECT COUNT(*) FROM file_entries WHERE is_deleted = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        let active_assets = conn
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE is_deleted = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok((active_files, active_assets))
    })?;
    let scans = scan_roots_with_cancel(&roots, Some(&state.refresh_cancel))?;
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
        cancel_if_requested(state, &mut progress)?;
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

        state.db.replace_search_row(asset_id)?;

        progress.processed_files = (index + 1) as u32;
        if should_publish_progress(index, scans.len()) {
            progress.message = Some(progress_message(&progress));
            *state.import_status.lock() = Some(progress.clone());
        }
    }

    progress.phase = "validating".to_string();
    progress.message = Some("ingress cleanup 1/4: removing missing files".to_string());
    *state.import_status.lock() = Some(progress.clone());

    cancel_if_requested(state, &mut progress)?;
    progress.files_deleted = state.db.soft_delete_missing_files(import_id, &roots)?;
    let affected_assets_before_reconcile = collect_assets_with_deleted_primary_files(state)?;
    let (deleted_asset_ids, reindexed_asset_ids) =
        state.db.reconcile_assets_after_file_deletions()?;
    progress.assets_deleted = deleted_asset_ids.len() as u32;

    for asset_id in &reindexed_asset_ids {
        state.db.replace_search_row(*asset_id)?;
    }

    if progress.files_deleted > 0 || progress.assets_deleted > 0 {
        invalidate_deleted_asset_caches(
            state,
            &affected_assets_before_reconcile,
            &deleted_asset_ids,
            &reindexed_asset_ids,
        )?;
        state.db.insert_log(
            "info",
            "import.cleanup",
            &format!(
                "removed {} files and {} assets; invalidated affected derived caches",
                progress.files_deleted, progress.assets_deleted
            ),
            None,
        )?;
    }

    progress.message = Some("ingress cleanup 2/4: pairing live photos".to_string());
    *state.import_status.lock() = Some(progress.clone());
    cancel_if_requested(state, &mut progress)?;
    let live_photo_pairs = attach_live_photo_pairs(state, &scans)?;
    state.db.insert_log(
        "info",
        "import.cleanup",
        &format!("paired {live_photo_pairs} live photo motion files"),
        None,
    )?;

    progress.message = Some("ingress cleanup 3/4: merging duplicate assets".to_string());
    *state.import_status.lock() = Some(progress.clone());
    cancel_if_requested(state, &mut progress)?;
    let duplicate_group_count = scans
        .iter()
        .filter(|scan| scan.candidate_type != "json" && scan.candidate_type != "other")
        .filter(|scan| scan.quick_hash.is_some())
        .map(|scan| {
            (
                scan.quick_hash.clone().unwrap_or_default(),
                scan.candidate_type.clone(),
            )
        })
        .collect::<HashSet<_>>()
        .len();
    progress.message = Some(format!(
        "ingress cleanup 3/4: checked 0 / {duplicate_group_count} duplicate groups, merged 0 assets"
    ));
    *state.import_status.lock() = Some(progress.clone());
    let merged_duplicates = merge_duplicate_assets_by_hash(state, &scans, &mut progress)?;
    progress.assets_updated += merged_duplicates;
    state.db.insert_log(
        "info",
        "import.cleanup",
        &format!("merged {merged_duplicates} duplicate assets"),
        None,
    )?;

    progress.message = Some(format!(
        "ingress validation 4/4: checked 0 / {} scans",
        scans.len()
    ));
    *state.import_status.lock() = Some(progress.clone());
    validate_import_with_progress(&state.db, import_id, &scans, |processed, total| {
        if state.refresh_cancel.load(Ordering::SeqCst) {
            return;
        }
        progress.message = Some(format!(
            "ingress validation 4/4: checked {processed} / {total} scans"
        ));
        *state.import_status.lock() = Some(progress.clone());
    })?;
    cancel_if_requested(state, &mut progress)?;

    let (active_files_after, active_assets_after) = state.db.with_connection(|conn| {
        use rusqlite::OptionalExtension;
        let active_files = conn
            .query_row(
                "SELECT COUNT(*) FROM file_entries WHERE is_deleted = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        let active_assets = conn
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE is_deleted = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok((active_files, active_assets))
    })?;
    progress.files_added = (active_files_after - active_files_before
        + i64::from(progress.files_deleted))
    .max(0) as u32;
    progress.assets_added = (active_assets_after - active_assets_before
        + i64::from(progress.assets_deleted))
    .max(0) as u32;
    progress.files_updated =
        active_files_after.saturating_sub(i64::from(progress.files_added)) as u32;
    let kept_assets = active_assets_after.saturating_sub(i64::from(progress.assets_added));
    state.db.insert_log(
        "info",
        "import.summary",
        &format!(
            "scanned={} active_files={} active_assets={} new_files={} new_assets={} kept_assets={} stale_files_removed={} stale_assets_removed={} merged_duplicates={} live_photo_pairs={}",
            progress.files_scanned,
            active_files_after,
            active_assets_after,
            progress.files_added,
            progress.assets_added,
            kept_assets,
            progress.files_deleted,
            progress.assets_deleted,
            merged_duplicates,
            live_photo_pairs,
        ),
        None,
    )?;

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

fn cancel_if_requested(state: &AppState, progress: &mut ImportProgress) -> Result<(), AppError> {
    if state.refresh_cancel.load(Ordering::SeqCst) {
        progress.status = "cancelled".to_string();
        progress.phase = "cancelled".to_string();
        progress.message = Some("refresh cancelled".to_string());
        *state.import_status.lock() = Some(progress.clone());
        state.db.insert_log(
            "warning",
            "import",
            &format!("refresh {} cancelled", progress.import_id),
            None,
        )?;
        return Err(AppError::Message("refresh cancelled".to_string()));
    }

    Ok(())
}

fn should_publish_progress(index: usize, total: usize) -> bool {
    index == 0 || index + 1 == total || (index + 1) % 100 == 0
}

fn collect_assets_with_deleted_primary_files(
    state: &AppState,
) -> Result<HashMap<i64, String>, AppError> {
    state.db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT a.id, f.path
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             WHERE a.is_deleted = 0
               AND f.is_deleted = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    })
}

fn invalidate_deleted_asset_caches(
    state: &AppState,
    old_primary_paths: &HashMap<i64, String>,
    deleted_asset_ids: &[i64],
    reindexed_asset_ids: &[i64],
) -> Result<(), AppError> {
    state.thumbnail_generation.fetch_add(1, Ordering::SeqCst);
    let mut affected_asset_ids = deleted_asset_ids.to_vec();
    affected_asset_ids.extend(reindexed_asset_ids.iter().copied());
    affected_asset_ids.sort_unstable();
    affected_asset_ids.dedup();

    let mut affected_primary_paths = old_primary_paths
        .iter()
        .filter(|(asset_id, _)| affected_asset_ids.contains(asset_id))
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    for asset_id in reindexed_asset_ids {
        if let Ok(detail) = state.db.get_asset_detail(*asset_id) {
            if let Some(primary_path) = detail.primary_path {
                affected_primary_paths.push(primary_path);
            }
        }
    }
    affected_primary_paths.sort();
    affected_primary_paths.dedup();

    let mut affected_thumbnail_keys = HashSet::new();
    let mut affected_preview_keys = HashSet::new();
    for path in &affected_primary_paths {
        let path_buf = std::path::PathBuf::from(path);
        affected_thumbnail_keys.insert(thumbnail_cache_key_for_path(
            &path_buf,
            GRID_THUMBNAIL_SIZE,
            false,
        ));
        affected_preview_keys.insert(thumbnail_cache_key_for_path(
            &path_buf,
            state.viewer_preview_size(),
            true,
        ));
        for replacement_key in
            preview_cache_replacement_keys_for_path(&path_buf, state.viewer_preview_size())
        {
            affected_preview_keys.insert(replacement_key);
        }
    }

    {
        let mut inflight = state.inflight_thumbnails.lock();
        inflight.retain(|key| {
            !affected_thumbnail_keys.contains(key) && !affected_preview_keys.contains(key)
        });
    }
    {
        let mut failed = state.failed_thumbnails.lock();
        failed.retain(|key| {
            !affected_thumbnail_keys.contains(key) && !affected_preview_keys.contains(key)
        });
    }
    {
        let mut thumbnail_cache = state.thumbnail_cache.lock();
        for key in &affected_thumbnail_keys {
            thumbnail_cache.remove(key);
        }
    }
    {
        let mut preview_cache = state.preview_cache.lock();
        for key in &affected_preview_keys {
            preview_cache.remove(key);
        }
    }

    let viewer_cache_dir = state.viewer_cache_dir();
    for path in affected_primary_paths {
        let path_buf = std::path::PathBuf::from(path);
        if let Some(image_cache_path) =
            viewer_image_cache_path(&path_buf, VIEWER_IMAGE_CACHE_SIZE, &viewer_cache_dir)?
        {
            let _ = std::fs::remove_file(image_cache_path);
        }
        if let Some(video_cache_path) = viewer_video_cache_path(&path_buf, &viewer_cache_dir)? {
            let _ = std::fs::remove_file(&video_cache_path);
            let _ = std::fs::remove_file(video_cache_path.with_extension("tmp.mp4"));
        }
    }

    {
        let mut jobs = state.viewer_video_jobs.lock();
        jobs.retain(|path, _| {
            !old_primary_paths
                .values()
                .any(|candidate| candidate == path)
        });
    }
    state
        .db
        .clear_viewer_video_transcode_statuses_for_assets(&affected_asset_ids)?;
    Ok(())
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
    scan.candidate_type == "video"
}

fn find_still_pair_asset_id(
    state: &AppState,
    scan: &crate::models::FileScanRecord,
) -> Result<Option<i64>, AppError> {
    let still_stems = still_pair_stems(&scan.filename);
    state.db.with_connection(|conn| {
        use rusqlite::{OptionalExtension, params};
        for still_stem in still_stems {
            let result = conn
                .query_row(
                    "SELECT a.id
                     FROM assets a
                     JOIN file_entries f ON f.id = a.primary_file_id
                     WHERE f.parent_path = ?1
                       AND a.media_kind IN ('photo', 'live_photo')
                       AND (
                         f.filename LIKE ?2
                         OR f.filename LIKE ?3
                       )
                     LIMIT 1",
                    params![
                        scan.parent_path,
                        format!("{still_stem}.%"),
                        format!("{still_stem}(%).%"),
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if result.is_some() {
                return Ok(result);
            }
        }
        Ok(None)
    })
}

fn attach_live_photo_pairs(
    state: &AppState,
    scans: &[crate::models::FileScanRecord],
) -> Result<u32, AppError> {
    let mut paired = 0_u32;
    for scan in scans.iter().filter(|scan| is_live_photo_video(scan)) {
        let file_id = state.db.with_connection(|conn| {
            use rusqlite::{OptionalExtension, params};
            conn.query_row(
                "SELECT id FROM file_entries WHERE path = ?1 LIMIT 1",
                params![scan.path],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(AppError::from)
        })?;

        let Some(file_id) = file_id else {
            continue;
        };

        if let Some(still_asset_id) = find_still_pair_asset_id(state, scan)? {
            state
                .db
                .attach_asset_file(still_asset_id, file_id, "live_photo_video")?;
            paired += 1;
        }
    }

    Ok(paired)
}

fn merge_duplicate_assets_by_hash(
    state: &AppState,
    scans: &[crate::models::FileScanRecord],
    progress: &mut ImportProgress,
) -> Result<u32, AppError> {
    let mut groups = HashSet::<(Vec<u8>, String)>::new();
    for scan in scans {
        if scan.candidate_type == "json" || scan.candidate_type == "other" {
            continue;
        }
        if let Some(hash) = &scan.quick_hash {
            groups.insert((hash.clone(), scan.candidate_type.clone()));
        }
    }

    let total_groups = groups.len();
    let mut processed_groups = 0_usize;
    let mut merged_assets = 0_u32;

    for (hash, media_kind) in groups {
        cancel_if_requested(state, progress)?;
        merged_assets += merge_asset_group_for_hash(state, &hash, &media_kind)? as u32;
        processed_groups += 1;
        if processed_groups == 1 || processed_groups == total_groups || processed_groups % 100 == 0
        {
            progress.message = Some(format!(
                "ingress cleanup 3/4: checked {processed_groups} / {total_groups} duplicate groups, merged {merged_assets} assets"
            ));
            *state.import_status.lock() = Some(progress.clone());
        }
    }

    Ok(merged_assets)
}

fn merge_asset_group_for_hash(
    state: &AppState,
    hash: &[u8],
    media_kind: &str,
) -> Result<usize, AppError> {
    let merged_assets = state.db.with_connection(|conn| {
        use rusqlite::{OptionalExtension, params};

        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.id, f.path
             FROM assets a
             JOIN file_entries f ON f.id = a.primary_file_id
             WHERE a.is_deleted = 0
               AND a.media_kind = ?2
               AND f.quick_hash = ?1
             ORDER BY a.id",
        )?;
        let assets = stmt
            .query_map(params![hash, media_kind], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();

        if assets.len() <= 1 {
            return Ok(Vec::new());
        }

        let canonical_asset_id = assets[0].0;
        let canonical_has_sidecar = conn
            .query_row(
                "SELECT 1 FROM sidecar_metadata WHERE asset_id = ?1 LIMIT 1",
                params![canonical_asset_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        let mut canonical_has_sidecar = canonical_has_sidecar;
        let mut merged = Vec::new();

        for (duplicate_asset_id, duplicate_path) in assets.iter().skip(1) {
            conn.execute(
                "INSERT OR IGNORE INTO album_assets (album_id, asset_id, position_hint, added_at)
                 SELECT album_id, ?1, position_hint, added_at
                 FROM album_assets
                 WHERE asset_id = ?2",
                params![canonical_asset_id, duplicate_asset_id],
            )?;
            conn.execute(
                "DELETE FROM album_assets WHERE asset_id = ?1",
                params![duplicate_asset_id],
            )?;

            conn.execute(
                "INSERT OR IGNORE INTO asset_files (asset_id, file_id, role)
                 SELECT ?1,
                        file_id,
                        CASE WHEN role = 'primary' THEN 'duplicate' ELSE role END
                 FROM asset_files
                 WHERE asset_id = ?2",
                params![canonical_asset_id, duplicate_asset_id],
            )?;
            conn.execute(
                "DELETE FROM asset_files WHERE asset_id = ?1",
                params![duplicate_asset_id],
            )?;

            let duplicate_has_sidecar = conn
                .query_row(
                    "SELECT 1 FROM sidecar_metadata WHERE asset_id = ?1 LIMIT 1",
                    params![duplicate_asset_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if duplicate_has_sidecar {
                if canonical_has_sidecar {
                    conn.execute(
                        "DELETE FROM sidecar_metadata WHERE asset_id = ?1",
                        params![duplicate_asset_id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE sidecar_metadata SET asset_id = ?1 WHERE asset_id = ?2",
                        params![canonical_asset_id, duplicate_asset_id],
                    )?;
                    canonical_has_sidecar = true;
                }
            }

            conn.execute(
                "DELETE FROM search_fts WHERE asset_id = ?1",
                params![duplicate_asset_id],
            )?;
            conn.execute(
                "DELETE FROM asset_relationships
                 WHERE src_asset_id = ?1 OR dst_asset_id = ?1",
                params![duplicate_asset_id],
            )?;
            conn.execute(
                "UPDATE assets
                 SET is_deleted = 1, updated_at = ?2
                 WHERE id = ?1",
                params![duplicate_asset_id, crate::util::time::utc_now()],
            )?;
            merged.push((*duplicate_asset_id, duplicate_path.clone()));
        }

        Ok(std::iter::once(assets[0].clone())
            .chain(merged)
            .collect::<Vec<_>>())
    })?;

    if let Some((canonical_asset_id, canonical_path)) = merged_assets.first().cloned() {
        state.db.replace_search_row(canonical_asset_id)?;
        let merged_paths = merged_assets
            .iter()
            .skip(1)
            .map(|(_, path)| format!("merged: {path}"))
            .collect::<Vec<_>>();
        state.db.insert_log(
            "info",
            "merge",
            &format!(
                "merged {} duplicate assets for media_kind={media_kind}\nkept: {canonical_path}\n{}",
                merged_assets.len() - 1,
                merged_paths.join("\n")
            ),
            Some(canonical_asset_id),
        )?;
    }

    Ok(merged_assets.len().saturating_sub(1))
}

fn still_pair_stems(filename: &str) -> Vec<String> {
    let stem = filename.split('.').next().unwrap_or_default().to_string();
    let mut stems = vec![stem.clone()];

    for suffix in ["-motion", "-live"] {
        if let Some(stripped) = stem.strip_suffix(suffix) {
            if !stems.iter().any(|existing| existing == stripped) {
                stems.push(stripped.to_string());
            }
        }
    }

    stems
}
