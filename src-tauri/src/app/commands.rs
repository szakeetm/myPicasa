use std::path::PathBuf;

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
        ImportProgress, LogEntry, RefreshRequest,
    },
    search::query_service,
};

type CommandResult<T> = Result<T, InvokeError>;

fn map_error<E: std::fmt::Display>(error: E) -> InvokeError {
    InvokeError::from(error.to_string())
}

#[tauri::command]
pub fn refresh_index(request: RefreshRequest, state: State<AppState>) -> CommandResult<ImportProgress> {
    info!(roots = ?request.roots, "refresh_index");
    let progress = refresh_takeout_index(&state, request).map_err(map_error)?;
    Ok(progress)
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
    let key = format!("{asset_id}:{size}");
    if let Some(bytes) = state.thumbnail_cache.lock().get(&key) {
        return Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))));
    }

    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path.clone() else {
        return Ok(None);
    };

    match generate_thumbnail(&PathBuf::from(primary_path), size) {
        Ok(Some(bytes)) => {
            state.thumbnail_cache.lock().insert(key, bytes.clone());
            Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            error!(asset_id, %error, "thumbnail generation failed");
            state
                .db
                .insert_log("error", "thumbnail", &error.to_string(), Some(asset_id))
                .map_err(map_error)?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn load_viewer_frame(asset_id: i64, state: State<AppState>) -> CommandResult<Option<String>> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Ok(None);
    };

    match generate_viewer_image(&PathBuf::from(primary_path), 2400) {
        Ok(Some(bytes)) => Ok(Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))),
        Ok(None) => Ok(None),
        Err(error) => {
            error!(asset_id, %error, "viewer frame load failed");
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

pub fn command_handlers() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool {
    generate_handler![
        refresh_index,
        get_import_status,
        list_albums,
        list_assets_by_date,
        list_assets_by_album,
        search_assets,
        get_asset_detail,
        get_ingress_diagnostics,
        request_thumbnail,
        load_viewer_frame,
        get_live_photo_pair,
        get_cache_stats,
        get_recent_logs,
        record_client_log,
        reset_local_database
    ]
}
