use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Instant;

use rayon::current_num_threads;
use tracing::info;

use crate::{
    app::state::{AppState, preview_cache_replacement_keys_for_path, thumbnail_cache_key_for_path},
    db::DatabaseQueries,
    import::{
        scanner::{IndexedFile, scan_roots_with_cancel},
        sidecar::{parse_sidecar, takeout_match_score},
        validator::validate_import_with_progress,
    },
    media::thumb::{probe_media_duration_ms, viewer_image_cache_path, viewer_video_cache_path},
    models::{FileScanRecord, ImportProgress, ParsedSidecar, RefreshRequest},
    util::errors::AppError,
};

const GRID_THUMBNAIL_SIZE: u32 = 210;
const VIEWER_IMAGE_CACHE_SIZE: u32 = 2400;

pub fn reserve_refresh(state: &AppState) -> Result<ImportProgress, AppError> {
    let mut status = state.import_status.lock();
    if status
        .as_ref()
        .is_some_and(|progress| progress.status == "running")
    {
        return Err(AppError::Message("an import is already running".into()));
    }
    state.refresh_cancel.store(false, Ordering::SeqCst);
    let progress = ImportProgress {
        import_id: 0,
        status: "running".into(),
        phase: "starting".into(),
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
        message: Some("starting refresh".into()),
    };
    *status = Some(progress.clone());
    Ok(progress)
}

pub fn refresh_takeout_index(
    state: &AppState,
    request: RefreshRequest,
    mut progress: ImportProgress,
) -> Result<ImportProgress, AppError> {
    let result = run_refresh(state, request, &mut progress);
    progress.status = match &result {
        Ok(()) => "completed",
        Err(AppError::RefreshCancelled) => "cancelled",
        Err(_) => "failed",
    }
    .into();
    progress.phase = progress.status.clone();
    if let Err(error) = &result {
        progress.message = Some(error.to_string());
    }
    let persisted = if progress.import_id > 0 {
        state.db.finish_import(&progress)
    } else {
        Ok(())
    };
    if let Err(error) = &persisted {
        tracing::error!(%error, "could not record refresh result");
        if result.is_ok() {
            progress.status = "failed".into();
            progress.phase = "failed".into();
            progress.message = Some(error.to_string());
        }
    }
    *state.import_status.lock() = Some(progress.clone());
    result.and(persisted).map(|()| progress)
}

fn run_refresh(
    state: &AppState,
    request: RefreshRequest,
    progress: &mut ImportProgress,
) -> Result<(), AppError> {
    cancel_if_requested(state)?;
    let started = Instant::now();
    let roots = request
        .roots
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(AppError::Message(
            "at least one takeout root is required".into(),
        ));
    }
    let import_id = state.db.create_import(&roots.join("; "))?;
    progress.import_id = import_id;
    progress.phase = "scanning".into();
    progress.message = Some("scan in progress".into());
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
    let indexed = load_indexed_files(state)?;
    let scans = scan_roots_with_cancel(
        &roots,
        &indexed,
        Some(&state.refresh_cancel),
        |scanned_count| {
            progress.files_scanned = scanned_count;
            progress.message = Some(format!("scanning files: found {scanned_count} items"));
            *state.import_status.lock() = Some(progress.clone());
        },
    )?;
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

    let dirty_folders = dirty_folders(&scans, &indexed, &roots);
    let mut sidecars_by_parent = HashMap::<String, Vec<(String, ParsedSidecar)>>::new();
    for scan in &scans {
        cancel_if_requested(state)?;
        if !dirty_folders.contains(scan.parent_path.as_str()) {
            continue;
        }
        match parse_sidecar(scan) {
            Ok(Some(sidecar)) => sidecars_by_parent
                .entry(scan.parent_path.clone())
                .or_default()
                .push((scan.path.clone(), sidecar)),
            Ok(None) | Err(AppError::Json(_)) => {}
            Err(error) => return Err(error),
        }
    }

    progress.phase = "indexing".into();
    progress.message = Some(format!("indexing {} files", progress.total_files));
    *state.import_status.lock() = Some(progress.clone());

    let mut albums = HashMap::new();
    let unchanged: Vec<_> = scans
        .iter()
        .filter(|scan| !dirty_folders.contains(scan.parent_path.as_str()))
        .map(|scan| scan.path.as_str())
        .collect();
    let changed: Vec<_> = scans
        .iter()
        .filter(|scan| dirty_folders.contains(scan.parent_path.as_str()))
        .collect();
    state.db.mark_files_seen(import_id, &unchanged)?;
    progress.processed_files = unchanged.len() as u32;
    for batch in changed.chunks(256) {
        cancel_if_requested(state)?;
        let ids = state.db.upsert_files(import_id, batch)?;
        for (scan, file_id) in batch.iter().copied().zip(ids) {
            cancel_if_requested(state)?;
            if matches!(scan.candidate_type.as_str(), "photo" | "video") {
                let album_id = if let Some(&id) = albums.get(scan.parent_path.as_str()) {
                    id
                } else {
                    let id = state.db.upsert_album(&scan.parent_path)?;
                    albums.insert(scan.parent_path.as_str(), id);
                    id
                };
                let sidecar = find_sidecar_for(scan, &sidecars_by_parent);
                let duration_ms = if scan.candidate_type == "video" {
                    let cached = indexed
                        .get(&scan.path)
                        .filter(|entry| source_matches(scan, entry))
                        .and_then(|entry| entry.video_duration);
                    match cached {
                        Some(duration) => Some(duration),
                        None => probe_media_duration_ms(std::path::Path::new(&scan.path))?,
                    }
                } else {
                    None
                };
                let asset_id =
                    state
                        .db
                        .upsert_asset_for_file(file_id, scan, sidecar, duration_ms)?;
                state.db.attach_asset_file(asset_id, file_id, "primary")?;
                state.db.attach_album_asset(album_id, asset_id)?;
                if let Some(sidecar) = sidecar {
                    state.db.set_sidecar_metadata(asset_id, None, sidecar)?;
                } else {
                    state.db.with_connection(|conn| {
                        conn.execute(
                            "DELETE FROM sidecar_metadata WHERE asset_id = ?1",
                            [asset_id],
                        )?;
                        Ok(())
                    })?;
                }
            }
            progress.processed_files += 1;
            if should_publish_progress(progress.processed_files as usize - 1, scans.len()) {
                progress.message = Some(progress_message(progress));
                *state.import_status.lock() = Some(progress.clone());
            }
        }
    }

    progress.phase = "validating".to_string();
    progress.message = Some("ingress cleanup 1/4: removing missing files".to_string());
    *state.import_status.lock() = Some(progress.clone());

    cancel_if_requested(state)?;
    progress.files_deleted = state.db.soft_delete_missing_files(import_id, &roots)?;
    let affected_assets_before_reconcile = collect_assets_with_deleted_primary_files(state)?;
    let (deleted_asset_ids, reindexed_asset_ids) =
        state.db.reconcile_assets_after_file_deletions()?;
    progress.assets_deleted = deleted_asset_ids.len() as u32;

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
    cancel_if_requested(state)?;
    let live_photo_pairs = attach_live_photo_pairs(state, &scans, &dirty_folders)?;
    state.db.insert_log(
        "info",
        "import.cleanup",
        &format!("paired {live_photo_pairs} live photo motion files"),
        None,
    )?;

    progress.message = Some("ingress cleanup 3/4: merging duplicate assets".to_string());
    *state.import_status.lock() = Some(progress.clone());
    cancel_if_requested(state)?;
    let merged_duplicates = merge_duplicate_assets_by_hash(state, &dirty_folders, progress)?;
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
    let parsed_sidecars = sidecars_by_parent
        .values()
        .flatten()
        .map(|(path, sidecar)| (path.as_str(), sidecar))
        .collect();
    validate_import_with_progress(
        &state.db,
        import_id,
        &scans,
        &parsed_sidecars,
        &dirty_folders,
        |processed, total, publish| {
            cancel_if_requested(state)?;
            if publish {
                progress.message = Some(format!(
                    "ingress validation 4/4: checked {processed} / {total} scans"
                ));
                *state.import_status.lock() = Some(progress.clone());
            }
            Ok(())
        },
    )?;
    cancel_if_requested(state)?;

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
    progress.files_updated = scans
        .iter()
        .filter(|scan| {
            indexed
                .get(&scan.path)
                .is_some_and(|entry| !entry.deleted && !source_matches(scan, entry))
        })
        .count() as u32;
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

    progress.processed_files = progress.total_files;
    progress.message = Some(format!(
        "refresh completed in {} ms",
        started.elapsed().as_millis()
    ));
    Ok(())
}

fn load_indexed_files(state: &AppState) -> Result<HashMap<String, IndexedFile>, AppError> {
    state.db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT f.path, f.parent_path, f.file_size, f.mtime_utc, f.detected_format,
                    f.quick_hash, COALESCE(i.status = 'completed', 0), f.is_deleted,
                    (SELECT max(a.duration_ms) FROM assets a
                     WHERE a.primary_file_id = f.id AND a.is_deleted = 0)
             FROM file_entries f
             LEFT JOIN imports i ON i.id = f.last_seen_import_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                IndexedFile {
                    parent_path: row.get(1)?,
                    size: row.get(2)?,
                    mtime: row.get(3)?,
                    detected_format: row.get(4)?,
                    quick_hash: row.get(5)?,
                    complete: row.get(6)?,
                    deleted: row.get(7)?,
                    video_duration: row.get(8)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

fn source_matches(scan: &FileScanRecord, entry: &IndexedFile) -> bool {
    scan.file_size == entry.size && scan.mtime_utc == entry.mtime
}

fn dirty_folders<'a>(
    scans: &'a [FileScanRecord],
    indexed: &'a HashMap<String, IndexedFile>,
    roots: &[String],
) -> HashSet<&'a str> {
    let seen: HashSet<_> = scans.iter().map(|scan| scan.path.as_str()).collect();
    let mut dirty: HashSet<_> = scans
        .iter()
        .filter(|scan| {
            !indexed.get(&scan.path).is_some_and(|entry| {
                entry.complete && !entry.deleted && source_matches(scan, entry)
            })
        })
        .map(|scan| scan.parent_path.as_str())
        .collect();
    dirty.extend(
        indexed
            .iter()
            .filter(|(path, entry)| {
                !entry.deleted
                    && !seen.contains(path.as_str())
                    && roots
                        .iter()
                        .any(|root| std::path::Path::new(path).starts_with(root))
            })
            .map(|(_, entry)| entry.parent_path.as_str()),
    );
    dirty
}

fn cancel_if_requested(state: &AppState) -> Result<(), AppError> {
    if state.refresh_cancel.load(Ordering::SeqCst) {
        return Err(AppError::RefreshCancelled);
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
            viewer_image_cache_path(&path_buf, VIEWER_IMAGE_CACHE_SIZE, &viewer_cache_dir)
                .ok()
                .flatten()
        {
            let _ = std::fs::remove_file(image_cache_path);
        }
        if let Some(video_cache_path) = viewer_video_cache_path(&path_buf, &viewer_cache_dir)
            .ok()
            .flatten()
        {
            let _ = std::fs::remove_file(&video_cache_path);
            let _ = std::fs::remove_file(video_cache_path.with_extension("tmp.mp4"));
        }
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
    let parent = scan.parent_path.as_str();
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
                       AND a.is_deleted = 0 AND f.is_deleted = 0
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
    scans: &[FileScanRecord],
    dirty_folders: &HashSet<&str>,
) -> Result<u32, AppError> {
    if dirty_folders.is_empty() {
        return Ok(0);
    }
    state.db.with_connection(|conn| {
        let folders = serde_json::to_string(dirty_folders)?;
        conn.execute(
            "DELETE FROM asset_files
             WHERE role = 'live_photo_video' AND file_id IN (
                SELECT id FROM file_entries
                WHERE is_deleted = 1 OR parent_path IN (SELECT value FROM json_each(?1))
             )",
            [folders],
        )?;
        Ok(())
    })?;
    let mut paired = 0_u32;
    for scan in scans.iter().filter(|scan| {
        is_live_photo_video(scan) && dirty_folders.contains(scan.parent_path.as_str())
    }) {
        cancel_if_requested(state)?;
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
    dirty_folders: &HashSet<&str>,
    progress: &mut ImportProgress,
) -> Result<u32, AppError> {
    if dirty_folders.is_empty() {
        return Ok(0);
    }
    let groups = state.db.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "WITH candidates AS (
                SELECT a.id, f.path, f.quick_hash, a.media_kind,
                       f.parent_path IN (SELECT value FROM json_each(?1)) AS touched
                FROM file_entries f JOIN assets a ON a.primary_file_id = f.id
                WHERE f.is_deleted = 0 AND a.is_deleted = 0 AND f.quick_hash IS NOT NULL
            ), repeated AS (
                SELECT quick_hash, media_kind FROM candidates
                GROUP BY quick_hash, media_kind HAVING count(*) > 1 AND max(touched)
            )
            SELECT c.id, c.path, c.quick_hash, c.media_kind
            FROM repeated r JOIN candidates c USING (quick_hash, media_kind)
            ORDER BY c.id",
        )?;
        let rows = stmt.query_map([serde_json::to_string(dirty_folders)?], |row| {
            Ok((
                (row.get::<_, Vec<u8>>(2)?, row.get::<_, String>(3)?),
                (row.get::<_, i64>(0)?, row.get::<_, String>(1)?),
            ))
        })?;
        let mut groups = HashMap::<_, Vec<_>>::new();
        for row in rows {
            let (key, asset) = row?;
            groups.entry(key).or_default().push(asset);
        }
        Ok(groups)
    })?;
    let total = groups.len();
    let mut merged = 0;
    for (index, ((_, media_kind), assets)) in groups.into_iter().enumerate() {
        let mut verified = HashMap::<_, Vec<_>>::new();
        for (id, path) in assets {
            cancel_if_requested(state)?;
            let digest = match full_source_hash(state, &path) {
                Ok(digest) => digest,
                Err(AppError::RefreshCancelled) => return Err(AppError::RefreshCancelled),
                Err(error) => {
                    tracing::warn!(%path, %error, "skipping unreadable duplicate candidate");
                    continue;
                }
            };
            verified.entry(digest).or_default().push((id, path));
        }
        for assets in verified.values().filter(|assets| assets.len() > 1) {
            cancel_if_requested(state)?;
            merged += merge_verified_assets(state, assets, &media_kind)? as u32;
        }
        if should_publish_progress(index, total) {
            progress.message = Some(format!(
                "ingress cleanup 3/4: checked {} / {total} duplicate groups, merged {merged} assets",
                index + 1
            ));
            *state.import_status.lock() = Some(progress.clone());
        }
    }
    Ok(merged)
}

fn full_source_hash(state: &AppState, path: &str) -> Result<blake3::Hash, AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0; 128 * 1024];
    let mut hasher = blake3::Hasher::new();
    loop {
        cancel_if_requested(state)?;
        let len = file.read(&mut buffer)?;
        if len == 0 {
            return Ok(hasher.finalize());
        }
        hasher.update(&buffer[..len]);
    }
}

fn merge_verified_assets(
    state: &AppState,
    assets: &[(i64, String)],
    media_kind: &str,
) -> Result<usize, AppError> {
    let merged_assets = state.db.with_connection(|conn| {
        use rusqlite::{OptionalExtension, params};
        let tx = conn.unchecked_transaction()?;
        let conn = &tx;
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

        tx.commit()?;
        Ok(std::iter::once(assets[0].clone())
            .chain(merged)
            .collect::<Vec<_>>())
    })?;

    if let Some((canonical_asset_id, canonical_path)) = merged_assets.first().cloned() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::builder::build_app_state;
    use std::{fs, path::PathBuf};

    struct Library {
        state: AppState,
        directory: PathBuf,
        roots: Vec<String>,
    }

    impl Library {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "mypicasa-refresh-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let roots = ["Photos", "Photos-old"]
                .map(|name| {
                    let root = directory.join(name);
                    fs::create_dir_all(&root).unwrap();
                    root.to_string_lossy().into_owned()
                })
                .to_vec();
            let state = build_app_state(directory.join("app"), Some(0)).unwrap();
            Self {
                state,
                directory,
                roots,
            }
        }

        fn refresh(&self, roots: Vec<String>) -> Result<ImportProgress, AppError> {
            let progress = reserve_refresh(&self.state)?;
            refresh_takeout_index(&self.state, RefreshRequest { roots }, progress)
        }

        fn active_titles(&self) -> Vec<String> {
            self.state
                .db
                .with_connection(|conn| {
                    let mut stmt = conn
                        .prepare("SELECT title FROM assets WHERE is_deleted = 0 ORDER BY title")?;
                    let titles = stmt.query_map([], |row| row.get(0))?;
                    Ok(titles.collect::<Result<_, _>>()?)
                })
                .unwrap()
        }
    }

    impl Drop for Library {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn complete_refresh_is_root_scoped_and_unavailable_root_preserves_index() {
        let library = Library::new("roots");
        let first = library.directory.join("Photos/a.jpg");
        let second = library.directory.join("Photos-old/b.jpg");
        fs::write(&first, b"photo A").unwrap();
        fs::write(&second, b"photo B").unwrap();
        library.refresh(library.roots.clone()).unwrap();
        fs::rename(
            library.directory.join("Photos-old"),
            library.directory.join("offline"),
        )
        .unwrap();
        assert!(library.refresh(library.roots.clone()).is_err());
        assert_eq!(library.active_titles(), ["a.jpg", "b.jpg"]);
        fs::remove_file(first).unwrap();
        let progress = library.refresh(vec![library.roots[0].clone()]).unwrap();
        assert_eq!(progress.files_deleted, 1);
        assert_eq!(library.active_titles(), ["b.jpg"]);
    }

    #[test]
    fn unchanged_refresh_preserves_asset_and_source_revision_changes_cache_key() {
        let library = Library::new("incremental");
        let source = library.directory.join("Photos/a.jpg");
        fs::write(&source, b"photo A").unwrap();
        library.refresh(library.roots.clone()).unwrap();
        let asset_row = || {
            library
                .state
                .db
                .with_connection(|conn| {
                    Ok(
                        conn.query_row("SELECT id, updated_at FROM assets", [], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                        })?,
                    )
                })
                .unwrap()
        };
        let before = asset_row();
        let progress = library.refresh(library.roots.clone()).unwrap();
        assert_eq!(progress.files_added, 0);
        assert_eq!(progress.files_updated, 0);
        assert_eq!(asset_row(), before);
        let old_key = thumbnail_cache_key_for_path(&source, 210, false);
        fs::write(&source, b"different photo").unwrap();
        assert_ne!(old_key, thumbnail_cache_key_for_path(&source, 210, false));
        library.refresh(library.roots.clone()).unwrap();
        assert_ne!(asset_row().1, before.1);
    }

    #[test]
    fn verified_duplicates_merge_but_quick_hash_collisions_stay_separate() {
        let library = Library::new("duplicates");
        let bytes = vec![0; 150_000];
        let mut different_middle = bytes.clone();
        different_middle[75_000] = 1;
        fs::write(library.directory.join("Photos/a.jpg"), &bytes).unwrap();
        fs::write(
            library.directory.join("Photos-old/b.jpg"),
            &different_middle,
        )
        .unwrap();
        fs::write(library.directory.join("Photos-old/c.jpg"), &bytes).unwrap();
        library.refresh(library.roots.clone()).unwrap();
        assert_eq!(library.active_titles().len(), 2);
        assert!(library.active_titles().contains(&"b.jpg".into()));
    }

    #[test]
    fn reserved_refresh_rejects_concurrent_start_and_publishes_cancellation() {
        let library = Library::new("cancel");
        let progress = reserve_refresh(&library.state).unwrap();
        assert!(reserve_refresh(&library.state).is_err());
        library.state.refresh_cancel.store(true, Ordering::SeqCst);
        let error = refresh_takeout_index(
            &library.state,
            RefreshRequest {
                roots: library.roots.clone(),
            },
            progress,
        )
        .unwrap_err();
        assert!(matches!(error, AppError::RefreshCancelled));
        assert_eq!(
            library.state.import_status.lock().as_ref().unwrap().status,
            "cancelled"
        );
        assert!(library.refresh(library.roots.clone()).is_ok());
    }
}
