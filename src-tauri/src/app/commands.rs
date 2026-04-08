use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Instant;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{generate_handler, ipc::InvokeError, State};
use tracing::{error, info};

use crate::{
    app::state::AppState,
    db::DatabaseQueries,
    import::refresher::refresh_takeout_index,
    media::thumb::{generate_thumbnail, generate_viewer_image},
    models::{
        AlbumSummary, AssetDetail, AssetListRequest, AssetListResponse, CacheStats, DiagnosticEntry,
        ImportProgress, LogEntry, RefreshRequest, ThumbnailBatchItem,
    },
    search::query_service,
};

type CommandResult<T> = Result<T, InvokeError>;

fn map_error<E: std::fmt::Display>(error: E) -> InvokeError {
    InvokeError::from(error.to_string())
}

fn media_debug_info(path: &str) -> (String, u64) {
    let filename = PathBuf::from(path)
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or(path)
        .to_string();
    let file_size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    (filename, file_size)
}

fn spawn_thumbnail_job(state: AppState, asset_id: i64, size: u32, key: String) {
    thread::spawn(move || {
        let result = (|| -> Result<Option<Vec<u8>>, String> {
            let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(|error| error.to_string())?;
            let Some(primary_path) = detail.primary_path else {
                return Ok(None);
            };
            let (filename, file_size) = media_debug_info(&primary_path);
            let started = Instant::now();
            let generated = generate_thumbnail(&PathBuf::from(primary_path), size).map_err(|error| error.to_string())?;
            match &generated {
                Some(bytes) => {
                    println!(
                        "thumbnail_job asset_id={asset_id} filename=\"{filename}\" file_size={} generated_bytes={} elapsed_ms={}",
                        file_size,
                        bytes.len(),
                        started.elapsed().as_millis()
                    );
                }
                None => {
                    println!(
                        "thumbnail_job asset_id={asset_id} filename=\"{filename}\" file_size={} unavailable elapsed_ms={}",
                        file_size,
                        started.elapsed().as_millis()
                    );
                }
            }
            Ok(generated)
        })();

        match result {
            Ok(Some(bytes)) => {
                state.thumbnail_cache.lock().insert(key.clone(), bytes);
                state.failed_thumbnails.lock().remove(&key);
            }
            Ok(None) => {
                state.failed_thumbnails.lock().insert(key.clone());
            }
            Err(error) => {
                let _ = state
                    .db
                    .insert_log("error", "thumbnail_job", &error, Some(asset_id));
                state.failed_thumbnails.lock().insert(key.clone());
            }
        }

        state.inflight_thumbnails.lock().remove(&key);
    });
}

#[tauri::command]
pub fn refresh_index(request: RefreshRequest, state: State<AppState>) -> CommandResult<ImportProgress> {
    info!(roots = ?request.roots, "refresh_index");
    let progress = refresh_takeout_index(&state, request).map_err(map_error)?;
    Ok(progress)
}

#[tauri::command]
pub fn start_refresh_index(request: RefreshRequest, state: State<AppState>) -> CommandResult<()> {
    if matches!(
        state.import_status.lock().as_ref().map(|item| item.status.as_str()),
        Some("running")
    ) {
        return Err(map_error("an import is already running"));
    }

    let state = state.inner().clone();
    thread::spawn(move || {
        if let Err(error) = refresh_takeout_index(&state, request) {
            let message = error.to_string();
            error!(%message, "background refresh failed");
            println!("refresh_takeout_index: failed: {message}");
            *state.import_status.lock() = Some(ImportProgress {
                import_id: 0,
                status: "failed".to_string(),
                phase: "failed".to_string(),
                files_scanned: 0,
                processed_files: 0,
                total_files: 0,
                files_added: 0,
                files_updated: 0,
                files_deleted: 0,
                assets_added: 0,
                assets_updated: 0,
                assets_deleted: 0,
                worker_count: 0,
                message: Some(message.clone()),
            });
            let _ = state
                .db
                .insert_log("error", "import", &format!("background refresh failed: {message}"), None);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn get_import_status(state: State<AppState>) -> CommandResult<Option<ImportProgress>> {
    Ok(state.import_status.lock().clone())
}

#[tauri::command]
pub fn list_albums(state: State<AppState>) -> CommandResult<Vec<AlbumSummary>> {
    query_service::list_albums(&state.db).map_err(map_error)
}

#[tauri::command]
pub fn list_assets_by_date(
    request: AssetListRequest,
    state: State<AppState>,
) -> CommandResult<AssetListResponse> {
    query_service::list_assets_by_date(&state.db, request).map_err(map_error)
}

#[tauri::command]
pub fn list_assets_by_album(
    album_id: i64,
    request: AssetListRequest,
    state: State<AppState>,
) -> CommandResult<AssetListResponse> {
    query_service::list_assets_by_album(&state.db, album_id, request).map_err(map_error)
}

#[tauri::command]
pub fn search_assets(
    request: AssetListRequest,
    state: State<AppState>,
) -> CommandResult<AssetListResponse> {
    query_service::search_assets(&state.db, request).map_err(map_error)
}

#[tauri::command]
pub fn get_asset_detail(asset_id: i64, state: State<AppState>) -> CommandResult<AssetDetail> {
    query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)
}

#[tauri::command]
pub fn get_ingress_diagnostics(state: State<AppState>) -> CommandResult<Vec<DiagnosticEntry>> {
    query_service::get_ingress_diagnostics(&state.db).map_err(map_error)
}

#[tauri::command]
pub fn request_thumbnail(
    asset_id: i64,
    size: u32,
    state: State<AppState>,
) -> CommandResult<Option<String>> {
    let started = Instant::now();
    let key = format!("{asset_id}:{size}");
    if let Some(bytes) = state.thumbnail_cache.lock().get(&key) {
        let elapsed = started.elapsed().as_millis();
        info!(asset_id, elapsed_ms = elapsed, "thumbnail cache hit");
        println!("thumbnail asset_id={asset_id} cache_hit elapsed_ms={elapsed}");
        return Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))));
    }

    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path.clone() else {
        return Ok(None);
    };
    let (filename, file_size) = media_debug_info(&primary_path);

    match generate_thumbnail(&PathBuf::from(primary_path), size) {
        Ok(Some(bytes)) => {
            state.thumbnail_cache.lock().insert(key, bytes.clone());
            let elapsed = started.elapsed().as_millis();
            info!(
                asset_id,
                elapsed_ms = elapsed,
                bytes = bytes.len(),
                filename = %filename,
                file_size,
                "thumbnail generated"
            );
            println!(
                "thumbnail asset_id={asset_id} filename=\"{filename}\" file_size={} generated_bytes={} elapsed_ms={elapsed}",
                file_size,
                bytes.len()
            );
            Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))))
        }
        Ok(None) => {
            let elapsed = started.elapsed().as_millis();
            info!(asset_id, elapsed_ms = elapsed, filename = %filename, file_size, "thumbnail unavailable");
            println!(
                "thumbnail asset_id={asset_id} filename=\"{filename}\" file_size={} unavailable elapsed_ms={elapsed}",
                file_size
            );
            Ok(None)
        }
        Err(error) => {
            error!(asset_id, %error, "thumbnail generation failed");
            println!(
                "thumbnail asset_id={asset_id} filename=\"{filename}\" file_size={} failed error={error}",
                file_size
            );
            state
                .db
                .insert_log("error", "thumbnail", &error.to_string(), Some(asset_id))
                .map_err(map_error)?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn request_thumbnails_batch(
    asset_ids: Vec<i64>,
    size: u32,
    state: State<AppState>,
) -> CommandResult<Vec<ThumbnailBatchItem>> {
    let started = Instant::now();
    let cache = state.thumbnail_cache.clone();

    let items = asset_ids
        .into_iter()
        .map(|asset_id| {
            let key = format!("{asset_id}:{size}");

            if let Some(bytes) = cache.lock().get(&key) {
                return ThumbnailBatchItem {
                    asset_id,
                    status: "ready".to_string(),
                    data_url: Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))),
                };
            }

            if state.failed_thumbnails.lock().contains(&key) {
                return ThumbnailBatchItem {
                    asset_id,
                    status: "unavailable".to_string(),
                    data_url: None,
                };
            }

            let mut inflight = state.inflight_thumbnails.lock();
            if inflight.insert(key.clone()) {
                drop(inflight);
                spawn_thumbnail_job(state.inner().clone(), asset_id, size, key.clone());
            }

            ThumbnailBatchItem {
                asset_id,
                status: "pending".to_string(),
                data_url: None,
            }
        })
        .collect::<Vec<_>>();

    let elapsed = started.elapsed().as_millis();
    let hits = items.iter().filter(|item| item.status == "ready").count();
    let pending = items.iter().filter(|item| item.status == "pending").count();
    info!(
        elapsed_ms = elapsed,
        item_count = items.len(),
        hit_count = hits,
        pending_count = pending,
        "thumbnail batch completed"
    );
    println!(
        "thumbnail_batch item_count={} hit_count={} pending_count={} elapsed_ms={elapsed}",
        items.len(),
        hits
        ,
        pending
    );

    Ok(items)
}

#[tauri::command]
pub fn load_viewer_frame(asset_id: i64, state: State<AppState>) -> CommandResult<Option<String>> {
    let started = Instant::now();
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Ok(None);
    };
    let (filename, file_size) = media_debug_info(&primary_path);

    match generate_viewer_image(&PathBuf::from(primary_path), 2400) {
        Ok(Some(bytes)) => {
            let elapsed = started.elapsed().as_millis();
            info!(
                asset_id,
                elapsed_ms = elapsed,
                bytes = bytes.len(),
                filename = %filename,
                file_size,
                "viewer image generated"
            );
            println!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} generated_bytes={} elapsed_ms={elapsed}",
                file_size,
                bytes.len()
            );
            Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))))
        }
        Ok(None) => {
            let elapsed = started.elapsed().as_millis();
            info!(asset_id, elapsed_ms = elapsed, filename = %filename, file_size, "viewer image unavailable");
            println!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} unavailable elapsed_ms={elapsed}",
                file_size
            );
            Ok(None)
        }
        Err(error) => {
            error!(asset_id, %error, "viewer frame load failed");
            println!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} failed error={error}",
                file_size
            );
            state
                .db
                .insert_log("error", "viewer", &error.to_string(), Some(asset_id))
                .map_err(map_error)?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn get_live_photo_pair(asset_id: i64, state: State<AppState>) -> CommandResult<Option<String>> {
    query_service::get_live_photo_pair(&state.db, asset_id).map_err(map_error)
}

#[tauri::command]
pub fn get_cache_stats(state: State<AppState>) -> CommandResult<CacheStats> {
    Ok(state.thumbnail_cache.lock().stats())
}

#[tauri::command]
pub fn get_recent_logs(limit: Option<u32>, state: State<AppState>) -> CommandResult<Vec<LogEntry>> {
    query_service::get_recent_logs(&state.db, limit.unwrap_or(300)).map_err(map_error)
}

#[tauri::command]
pub fn record_client_log(
    level: String,
    scope: String,
    message: String,
    state: State<AppState>,
) -> CommandResult<()> {
    state
        .db
        .insert_log(&level, &scope, &message, None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn reset_local_database(state: State<AppState>) -> CommandResult<()> {
    state.db.reset().map_err(map_error)?;
    *state.import_status.lock() = None;
    state.thumbnail_cache.lock().clear();
    state
        .db
        .insert_log("warning", "reset", "local database reset to default state", None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn clear_diagnostics(state: State<AppState>) -> CommandResult<()> {
    state.db.clear_diagnostics().map_err(map_error)?;
    state
        .db
        .insert_log("info", "debug", "cleared ingress diagnostics", None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn clear_logs(state: State<AppState>) -> CommandResult<()> {
    state.db.clear_logs().map_err(map_error)?;
    Ok(())
}

pub fn command_handlers() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool {
    generate_handler![
        refresh_index,
        start_refresh_index,
        get_import_status,
        list_albums,
        list_assets_by_date,
        list_assets_by_album,
        search_assets,
        get_asset_detail,
        get_ingress_diagnostics,
        request_thumbnail,
        request_thumbnails_batch,
        load_viewer_frame,
        get_live_photo_pair,
        get_cache_stats,
        get_recent_logs,
        record_client_log,
        reset_local_database,
        clear_diagnostics,
        clear_logs
    ]
}
