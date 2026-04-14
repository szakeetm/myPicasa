use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use tauri::{AppHandle, State, generate_handler, ipc::InvokeError};
use tracing::{error, info};
use walkdir::WalkDir;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::{
    app::{
        state::{
            AppState, BatchThumbnailGenerationState, BatchViewerTranscodeState, ThumbnailJob,
            ViewerTranscodeState, preview_cache_replacement_keys_for_path,
            thumbnail_cache_key_for_path,
        },
        sync_asset_protocol_scope,
    },
    db::DatabaseQueries,
    import::refresher::refresh_takeout_index,
    media::thumb::{
        VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT, clear_viewer_render_cache, generate_thumbnail,
        generate_viewer_image, generate_viewer_image_file, generate_viewer_video,
        probe_media_duration_ms, probe_primary_video_codec, probe_video_dimensions,
        thumbnail_generator_label, viewer_render_cache_stats, viewer_video_cache_path,
    },
    models::{
        AlbumSummary, AppBackupManifest, AppBackupSummary, AppSettings, AssetDetail,
        AssetListRequest, AssetListResponse, BatchThumbnailGenerationStatus,
        BatchViewerTranscodeStatus, CacheStats, CacheStorageMigrationStatus, DiagnosticEntry,
        ImportProgress, LogEntry, RefreshRequest, ThumbnailBatchItem, ViewerMediaStatus,
        ViewerPlaybackHint, ViewerPlaybackSupport,
    },
    search::query_service,
    util::time::utc_now,
};

type CommandResult<T> = Result<T, InvokeError>;
const PREVIEW_DEBUG_LOGS: bool = false;
const APP_BACKUP_MANIFEST_FILE: &str = "mypicasa-backup.json";
const APP_BACKUP_DATABASE_FILE: &str = "my_picasa.sqlite";

fn effective_preview_size_for_asset(
    state: &AppState,
    asset_id: i64,
    requested_size: u32,
) -> Option<u32> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).ok()?;
    let width = u32::try_from(detail.width?).ok()?;
    let height = u32::try_from(detail.height?).ok()?;
    Some(requested_size.min(width.min(height).max(1)))
}

fn map_error<E: std::fmt::Display>(error: E) -> InvokeError {
    InvokeError::from(error.to_string())
}

fn reveal_path_in_file_manager(path: &Path) -> Result<(), InvokeError> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(map_error)?;
        if !status.success() {
            return Err(InvokeError::from(
                "Failed to reveal asset in Finder".to_string(),
            ));
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let normalized = normalize_windows_explorer_path(path);
        let mut command = Command::new("explorer.exe");
        command.raw_arg(format!(r#"/select,"{}""#, normalized.to_string_lossy()));
        let status = command.status().map_err(map_error)?;
        if !status.success() {
            return Err(InvokeError::from(
                "Failed to reveal asset in Explorer".to_string(),
            ));
        }
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(InvokeError::from(
            "Reveal in file manager is not supported on this platform".to_string(),
        ))
    }
}

fn open_path_with_default_app(path: &Path) -> Result<(), InvokeError> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open").arg(path).status().map_err(map_error)?;
        if !status.success() {
            return Err(InvokeError::from(
                "Failed to open asset with default app".to_string(),
            ));
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let normalized = normalize_windows_explorer_path(path);
        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-Command")
            .arg("Start-Process -FilePath $env:MYPICASA_TARGET_PATH")
            .env("MYPICASA_TARGET_PATH", &normalized);
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
        let _child = command.spawn().map_err(map_error)?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(InvokeError::from(
            "Open with default app is not supported on this platform".to_string(),
        ))
    }
}

fn open_path_with_preview(path: &Path, is_video: bool) -> Result<(), InvokeError> {
    #[cfg(target_os = "macos")]
    {
        let status = if is_video {
            Command::new("osascript")
                .arg("-e")
                .arg("on run argv")
                .arg("-e")
                .arg("set mediaPath to POSIX file (item 1 of argv)")
                .arg("-e")
                .arg("tell application \"QuickTime Player\"")
                .arg("-e")
                .arg("activate")
                .arg("-e")
                .arg("set openedDocument to open mediaPath")
                .arg("-e")
                .arg("play openedDocument")
                .arg("-e")
                .arg("end tell")
                .arg("-e")
                .arg("end run")
                .arg(path)
                .status()
                .map_err(map_error)?
        } else {
            Command::new("qlmanage")
                .arg("-p")
                .arg(path)
                .status()
                .map_err(map_error)?
        };
        if !status.success() {
            return Err(InvokeError::from(
                "Failed to open asset with Quick Look".to_string(),
            ));
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let _ = is_video;
        let Some(parent) = path.parent() else {
            return Err(InvokeError::from(
                "Failed to determine asset folder".to_string(),
            ));
        };
        let normalized = normalize_windows_explorer_path(parent);
        let status = Command::new("explorer.exe")
            .arg(normalized)
            .status()
            .map_err(map_error)?;
        if !status.success() {
            return Err(InvokeError::from(
                "Failed to open asset folder in Explorer".to_string(),
            ));
        }
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (path, is_video);
        Err(InvokeError::from(
            "Preview action is not supported on this platform".to_string(),
        ))
    }
}

#[cfg(target_os = "windows")]
fn normalize_windows_explorer_path(path: &Path) -> PathBuf {
    let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut value = normalized.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix("\\\\?\\UNC\\") {
        value = format!("\\\\{stripped}");
    } else if let Some(stripped) = value.strip_prefix("\\\\?\\") {
        value = stripped.to_string();
    }
    PathBuf::from(value)
}

fn is_refresh_running(state: &AppState) -> bool {
    matches!(
        state
            .import_status
            .lock()
            .as_ref()
            .map(|item| item.status.as_str()),
        Some("running")
    )
}

fn is_refresh_cancelled_message(message: &str) -> bool {
    message.to_ascii_lowercase().contains("refresh cancelled")
}

fn wait_for_refresh_shutdown(state: &AppState, timeout: Duration) -> CommandResult<()> {
    let started = Instant::now();
    while is_refresh_running(state) {
        if started.elapsed() >= timeout {
            return Err(InvokeError::from(
                "Timed out waiting for refresh to stop".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn preview_debug_log(message: String) {
    if PREVIEW_DEBUG_LOGS {
        println!("{message}");
    }
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

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let bytes_f64 = bytes as f64;
    if bytes_f64 >= MB {
        format!("{:.1} MB", bytes_f64 / MB)
    } else if bytes_f64 >= KB {
        format!("{:.1} kB", bytes_f64 / KB)
    } else {
        format!("{bytes} B")
    }
}

fn human_elapsed_ms(elapsed_ms: u128) -> String {
    format!("{:.1}s", elapsed_ms as f64 / 1000.0)
}

fn human_duration_ms(duration_ms: u64) -> String {
    let total_seconds = duration_ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn thumb_log_kind(use_preview_cache: bool) -> &'static str {
    if use_preview_cache {
        "preview"
    } else {
        "thumb"
    }
}

fn record_thumb_log(
    state: &AppState,
    level: &str,
    asset_id: i64,
    message: String,
) -> Result<(), InvokeError> {
    state
        .db
        .insert_log(level, "thumb_gen", &message, Some(asset_id))
        .map_err(map_error)
}

fn enqueue_thumbnail_job(
    state: &AppState,
    asset_id: i64,
    source_path: &Path,
    size: u32,
    use_preview_cache: bool,
    generation: u64,
) -> Result<bool, InvokeError> {
    let key = thumbnail_cache_key_for_path(source_path, size, use_preview_cache);
    let cache = if use_preview_cache {
        &state.preview_cache
    } else {
        &state.thumbnail_cache
    };

    if cache.lock().get(&key).is_some() {
        return Ok(false);
    }

    if state.failed_thumbnails.lock().contains(&key) {
        return Ok(false);
    }

    let mut inflight = state.inflight_thumbnails.lock();
    if !inflight.insert(key.clone()) {
        return Ok(false);
    }
    drop(inflight);

    let sender = if use_preview_cache {
        &state.preview_job_sender
    } else {
        &state.thumbnail_job_sender
    };
    if !use_preview_cache {
        state.thumb_backlog.fetch_add(1, Ordering::SeqCst);
    }

    if let Err(error) = sender.send(ThumbnailJob {
        asset_id,
        size,
        key: key.clone(),
        generation,
        use_preview_cache,
    }) {
        if !use_preview_cache {
            state.thumb_backlog.fetch_sub(1, Ordering::SeqCst);
        }
        state.inflight_thumbnails.lock().remove(&key);
        state.failed_thumbnails.lock().insert(key);
        return Err(InvokeError::from(format!(
            "Failed to enqueue thumbnail job for asset {asset_id}: {error}"
        )));
    }

    Ok(true)
}

fn batch_thumbnail_generation_status_snapshot(
    state: &BatchThumbnailGenerationState,
) -> BatchThumbnailGenerationStatus {
    BatchThumbnailGenerationStatus {
        status: if state.running {
            "running".to_string()
        } else if state.total > 0 {
            "completed".to_string()
        } else {
            "idle".to_string()
        },
        total: state.total,
        completed: state.completed,
        failed: state.failed,
        skipped: state.skipped,
        stop_requested: state.stop_requested,
        current_asset_id: state.current_asset_id,
        current_filename: state.current_filename.clone(),
        current_source_bytes: state.current_source_bytes,
        current_elapsed_ms: state
            .current_started_at
            .map(|started_at| started_at.elapsed().as_millis() as u64),
        elapsed_ms: state.elapsed_ms.or_else(|| {
            state
                .started_at
                .map(|started_at| started_at.elapsed().as_millis() as u64)
        }),
        message: state.message.clone(),
    }
}

fn can_stream_original_video_bytes(path: &std::path::Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("mp4" | "m4v" | "mov" | "webm")) {
        return false;
    }

    match probe_primary_video_codec(path).ok().flatten().as_deref() {
        Some("h264" | "hevc") => true,
        Some(_) => false,
        None => false,
    }
}

#[derive(Clone)]
struct BatchThumbnailAsset {
    asset_id: i64,
    primary_path: String,
    filename: String,
    source_bytes: u64,
    needs_thumb: bool,
    needs_preview: bool,
}

enum ThumbnailBatchOutputState {
    Ready,
    Failed,
    Pending,
    Missing,
}

fn thumbnail_batch_output_state(
    state: &AppState,
    source_path: &Path,
    size: u32,
    use_preview_cache: bool,
) -> ThumbnailBatchOutputState {
    let key = thumbnail_cache_key_for_path(source_path, size, use_preview_cache);
    let cache = if use_preview_cache {
        &state.preview_cache
    } else {
        &state.thumbnail_cache
    };

    if cache.lock().get(&key).is_some() {
        ThumbnailBatchOutputState::Ready
    } else if state.failed_thumbnails.lock().contains(&key) {
        ThumbnailBatchOutputState::Failed
    } else if state.inflight_thumbnails.lock().contains(&key) {
        ThumbnailBatchOutputState::Pending
    } else {
        ThumbnailBatchOutputState::Missing
    }
}

#[tauri::command]
pub fn get_app_settings(state: State<AppState>) -> CommandResult<AppSettings> {
    Ok(state.app_settings_snapshot())
}

#[tauri::command]
pub fn update_app_settings(
    settings: AppSettings,
    app: AppHandle,
    state: State<AppState>,
) -> CommandResult<AppSettings> {
    let updated = state.update_app_settings(settings).map_err(map_error)?;
    sync_asset_protocol_scope(&app, state.inner()).map_err(map_error)?;
    Ok(updated)
}

#[tauri::command]
pub fn inspect_app_backup(backup_dir: String) -> CommandResult<AppBackupManifest> {
    load_backup_manifest(Path::new(&backup_dir))
}

#[tauri::command]
pub fn export_app_backup(
    backup_dir: String,
    state: State<AppState>,
) -> CommandResult<AppBackupSummary> {
    ensure_cache_storage_change_allowed(state.inner())?;
    let backup_dir = PathBuf::from(backup_dir);
    fs::create_dir_all(&backup_dir).map_err(map_error)?;

    let settings = state.app_settings_snapshot();
    let manifest = AppBackupManifest {
        format_version: 1,
        exported_at: utc_now(),
        settings: settings.clone(),
    };
    fs::write(
        backup_manifest_path(&backup_dir),
        serde_json::to_vec_pretty(&manifest).map_err(map_error)?,
    )
    .map_err(map_error)?;
    fs::write(
        backup_settings_path(&backup_dir),
        serde_json::to_vec_pretty(&settings).map_err(map_error)?,
    )
    .map_err(map_error)?;
    state
        .db
        .export_to(&backup_database_path(&backup_dir))
        .map_err(map_error)?;

    let cache_root = state.cache_data_dir();
    let mut cache_files = 0_u64;
    let mut cache_bytes = 0_u64;
    for subdir in cache_storage_subdirs() {
        clear_directory_contents(&backup_dir.join(subdir))?;
        let (files, bytes) =
            copy_directory_recursive(&cache_root.join(subdir), &backup_dir.join(subdir))?;
        cache_files += files;
        cache_bytes += bytes;
    }

    state
        .db
        .insert_log(
            "info",
            "backup",
            &format!(
                "exported app backup to {} with {} cache files",
                backup_dir.display(),
                cache_files
            ),
            None,
        )
        .map_err(map_error)?;

    Ok(AppBackupSummary {
        backup_dir: backup_dir.to_string_lossy().to_string(),
        settings,
        cache_files,
        cache_bytes,
    })
}

#[tauri::command]
pub fn import_app_backup(
    backup_dir: String,
    takeout_roots: Vec<String>,
    cache_storage_dir: Option<String>,
    app: AppHandle,
    state: State<AppState>,
) -> CommandResult<AppBackupSummary> {
    ensure_cache_storage_change_allowed(state.inner())?;
    let backup_dir = PathBuf::from(backup_dir);
    let manifest = load_backup_manifest(&backup_dir)?;
    let imported_roots = takeout_roots
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if imported_roots.is_empty() {
        return Err(InvokeError::from(
            "At least one Takeout root is required when importing a backup".to_string(),
        ));
    }
    if manifest.settings.indexed_roots.len() != imported_roots.len() {
        return Err(InvokeError::from(format!(
            "Backup expects {} Takeout roots, but {} were provided",
            manifest.settings.indexed_roots.len(),
            imported_roots.len()
        )));
    }

    let mut restored_settings = manifest.settings.clone();
    restored_settings.indexed_roots = imported_roots.clone();
    restored_settings.cache_storage_dir = cache_storage_dir;
    let restored_settings = state
        .apply_app_settings(restored_settings, false)
        .map_err(map_error)?;
    sync_asset_protocol_scope(&app, state.inner()).map_err(map_error)?;

    state.thumbnail_generation.fetch_add(1, Ordering::SeqCst);
    state.inflight_thumbnails.lock().clear();
    state.failed_thumbnails.lock().clear();
    state.viewer_video_jobs.lock().clear();
    *state.batch_viewer_transcode.lock() = BatchViewerTranscodeState::idle();
    *state.batch_thumbnail_generation.lock() = BatchThumbnailGenerationState::idle();
    *state.import_status.lock() = None;

    let cache_root = state.cache_data_dir();
    let mut cache_files = 0_u64;
    let mut cache_bytes = 0_u64;
    for subdir in cache_storage_subdirs() {
        let destination = cache_root.join(subdir);
        clear_directory_contents(&destination)?;
        let (files, bytes) = copy_directory_recursive(&backup_dir.join(subdir), &destination)?;
        cache_files += files;
        cache_bytes += bytes;
    }

    state
        .db
        .import_from(&backup_database_path(&backup_dir))
        .map_err(map_error)?;
    let root_mappings = manifest
        .settings
        .indexed_roots
        .iter()
        .cloned()
        .zip(imported_roots.iter().cloned())
        .collect::<Vec<_>>();
    state
        .db
        .remap_takeout_roots(&root_mappings)
        .map_err(map_error)?;

    state
        .db
        .insert_log(
            "info",
            "backup",
            &format!(
                "imported app backup from {} with {} cache files",
                backup_dir.display(),
                cache_files
            ),
            None,
        )
        .map_err(map_error)?;

    Ok(AppBackupSummary {
        backup_dir: backup_dir.to_string_lossy().to_string(),
        settings: restored_settings,
        cache_files,
        cache_bytes,
    })
}

fn cache_storage_migration_status_snapshot(state: &AppState) -> CacheStorageMigrationStatus {
    state.cache_storage_migration.lock().clone()
}

fn ensure_cache_storage_change_allowed(state: &AppState) -> CommandResult<()> {
    if matches!(
        state
            .import_status
            .lock()
            .as_ref()
            .map(|item| item.status.as_str()),
        Some("running")
    ) {
        return Err(InvokeError::from(
            "Wait for the current refresh to finish before changing app storage".to_string(),
        ));
    }
    if state.batch_viewer_transcode.lock().running {
        return Err(InvokeError::from(
            "Stop batch video transcoding before changing cache storage".to_string(),
        ));
    }
    if state.batch_thumbnail_generation.lock().running {
        return Err(InvokeError::from(
            "Stop batch thumbnail generation before changing app storage".to_string(),
        ));
    }
    if state.cache_storage_migration.lock().running {
        return Err(InvokeError::from(
            "Wait for the current cache migration to finish before changing app storage"
                .to_string(),
        ));
    }
    Ok(())
}

fn cache_storage_subdirs() -> [&'static str; 3] {
    ["thumbnail-cache", "preview-cache", "viewer-cache"]
}

fn backup_manifest_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join(APP_BACKUP_MANIFEST_FILE)
}

fn backup_database_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join(APP_BACKUP_DATABASE_FILE)
}

fn backup_settings_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join("settings.json")
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> Result<(u64, u64), InvokeError> {
    if !source.exists() {
        fs::create_dir_all(destination).map_err(map_error)?;
        return Ok((0, 0));
    }
    fs::create_dir_all(destination).map_err(map_error)?;
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    for entry in WalkDir::new(source).into_iter().flatten() {
        let Ok(relative) = entry.path().strip_prefix(source) else {
            continue;
        };
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(map_error)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(map_error)?;
        }
        fs::copy(entry.path(), &target).map_err(map_error)?;
        files += 1;
        bytes += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
    }
    Ok((files, bytes))
}

fn clear_directory_contents(path: &Path) -> Result<(), InvokeError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(map_error)?;
    }
    fs::create_dir_all(path).map_err(map_error)?;
    Ok(())
}

fn clear_app_derived_storage(state: &AppState) -> CommandResult<()> {
    state.thumbnail_generation.fetch_add(1, Ordering::SeqCst);
    state.thumbnail_cache.lock().clear();
    state.preview_cache.lock().clear();
    state.inflight_thumbnails.lock().clear();
    state.failed_thumbnails.lock().clear();
    state.viewer_video_jobs.lock().clear();
    state.inflight_viewer_playback_hints.lock().clear();
    state.refreshed_viewer_playback_hints.lock().clear();
    *state.batch_viewer_transcode.lock() = BatchViewerTranscodeState::idle();
    *state.batch_thumbnail_generation.lock() = BatchThumbnailGenerationState::idle();

    let mut cache_roots = vec![state.cache_data_dir(), state.default_cache_data_dir()];
    cache_roots.sort();
    cache_roots.dedup();
    for cache_root in cache_roots {
        for subdir in cache_storage_subdirs() {
            clear_directory_contents(&cache_root.join(subdir))?;
        }
    }
    clear_directory_contents(&state.working_dir())?;
    state
        .db
        .clear_viewer_video_transcode_statuses()
        .map_err(map_error)?;
    Ok(())
}

fn load_backup_manifest(backup_dir: &Path) -> CommandResult<AppBackupManifest> {
    let raw = fs::read_to_string(backup_manifest_path(backup_dir)).map_err(map_error)?;
    serde_json::from_str::<AppBackupManifest>(&raw).map_err(map_error)
}

fn collect_cache_copy_plan(source_root: &std::path::Path) -> Vec<(PathBuf, PathBuf, u64)> {
    let mut files = Vec::new();
    for subdir in cache_storage_subdirs() {
        let source_dir = source_root.join(subdir);
        for entry in WalkDir::new(&source_dir).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(relative_path) = entry.path().strip_prefix(source_root) else {
                continue;
            };
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            files.push((
                entry.path().to_path_buf(),
                relative_path.to_path_buf(),
                size,
            ));
        }
    }
    files
}

fn copy_file_with_progress(
    source: &std::path::Path,
    destination: &std::path::Path,
    status: &parking_lot::Mutex<CacheStorageMigrationStatus>,
    cancel_flag: &std::sync::atomic::AtomicBool,
) -> Result<bool, InvokeError> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Ok(false);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(map_error)?;
    }

    let mut reader = fs::File::open(source).map_err(map_error)?;
    let mut writer = fs::File::create(destination).map_err(map_error)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let read = reader.read(&mut buffer).map_err(map_error)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).map_err(map_error)?;
        let mut guard = status.lock();
        guard.copied_bytes = guard.copied_bytes.saturating_add(read as u64);
    }

    Ok(true)
}

fn finish_cache_storage_migration(
    state: &AppState,
    status_value: &str,
    message: String,
) -> CacheStorageMigrationStatus {
    let mut status = state.cache_storage_migration.lock();
    status.status = status_value.to_string();
    status.running = false;
    status.message = Some(message);
    status.current_path = None;
    status.clone()
}

#[tauri::command]
pub fn get_cache_storage_migration_status(
    state: State<AppState>,
) -> CommandResult<CacheStorageMigrationStatus> {
    Ok(cache_storage_migration_status_snapshot(state.inner()))
}

#[tauri::command]
pub fn cancel_cache_storage_migration(
    state: State<AppState>,
) -> CommandResult<CacheStorageMigrationStatus> {
    let mut status = state.cache_storage_migration.lock();
    if status.running {
        status.stop_requested = true;
        status.message = Some("Stopping after the current file...".to_string());
        state
            .cache_storage_migration_cancel
            .store(true, Ordering::SeqCst);
    }
    Ok(status.clone())
}

#[tauri::command]
pub fn start_cache_storage_migration(
    cache_storage_dir: Option<String>,
    copy_existing: bool,
    app: AppHandle,
    state: State<AppState>,
) -> CommandResult<CacheStorageMigrationStatus> {
    ensure_cache_storage_change_allowed(state.inner())?;
    {
        let status = state.cache_storage_migration.lock();
        if status.running {
            return Ok(status.clone());
        }
    }

    let current_settings = state.app_settings_snapshot();
    let next_settings = AppSettings {
        viewer_preview_size: current_settings.viewer_preview_size,
        cache_storage_dir,
        indexed_roots: current_settings.indexed_roots.clone(),
    }
    .sanitized();
    let source_root = state.cache_data_dir();
    let destination_root = state.resolve_cache_data_dir(&next_settings);

    if source_root == destination_root {
        return Ok(finish_cache_storage_migration(
            state.inner(),
            "completed",
            "Cache storage location is already active.".to_string(),
        ));
    }

    if !copy_existing {
        let applied = state
            .apply_app_settings(next_settings, false)
            .map_err(map_error)?;
        sync_asset_protocol_scope(&app, state.inner()).map_err(map_error)?;
        state
            .db
            .insert_log(
                "info",
                "settings",
                &format!(
                    "cache storage moved without copy to {}",
                    destination_root.display()
                ),
                None,
            )
            .map_err(map_error)?;
        {
            let mut status = state.cache_storage_migration.lock();
            *status = CacheStorageMigrationStatus {
                status: "completed".to_string(),
                running: false,
                stop_requested: false,
                copy_existing: false,
                source_dir: Some(source_root.to_string_lossy().to_string()),
                destination_dir: Some(destination_root.to_string_lossy().to_string()),
                total_files: 0,
                copied_files: 0,
                total_bytes: 0,
                copied_bytes: 0,
                current_path: None,
                message: Some(
                    "Switched cache storage. New assets will be rendered there.".to_string(),
                ),
            };
        }
        let _ = applied;
        return Ok(cache_storage_migration_status_snapshot(state.inner()));
    }

    let files = collect_cache_copy_plan(&source_root);
    let total_files = files.len() as u64;
    let total_bytes = files.iter().map(|(_, _, size)| *size).sum::<u64>();
    state
        .cache_storage_migration_cancel
        .store(false, Ordering::SeqCst);
    {
        let mut status = state.cache_storage_migration.lock();
        *status = CacheStorageMigrationStatus {
            status: "running".to_string(),
            running: true,
            stop_requested: false,
            copy_existing: true,
            source_dir: Some(source_root.to_string_lossy().to_string()),
            destination_dir: Some(destination_root.to_string_lossy().to_string()),
            total_files,
            copied_files: 0,
            total_bytes,
            copied_bytes: 0,
            current_path: None,
            message: Some(if total_files == 0 {
                "No existing cache files found. Preparing destination...".to_string()
            } else {
                format!("Copying {total_files} cache files")
            }),
        };
    }

    let state = state.inner().clone();
    let worker_state = state.clone();
    let app_handle = app.clone();
    thread::spawn(move || {
        let copy_result = (|| -> CommandResult<()> {
            fs::create_dir_all(&destination_root).map_err(map_error)?;
            for subdir in cache_storage_subdirs() {
                fs::create_dir_all(destination_root.join(subdir)).map_err(map_error)?;
            }

            for (source_path, relative_path, _) in files {
                if worker_state
                    .cache_storage_migration_cancel
                    .load(Ordering::SeqCst)
                {
                    return Ok(());
                }
                {
                    let mut status = worker_state.cache_storage_migration.lock();
                    status.current_path = Some(relative_path.to_string_lossy().to_string());
                    status.message = Some(format!("Copying {}", relative_path.to_string_lossy()));
                }

                let destination_path = destination_root.join(&relative_path);
                let copied = copy_file_with_progress(
                    &source_path,
                    &destination_path,
                    &worker_state.cache_storage_migration,
                    &worker_state.cache_storage_migration_cancel,
                )?;
                if !copied {
                    return Ok(());
                }

                let mut status = worker_state.cache_storage_migration.lock();
                status.copied_files = status.copied_files.saturating_add(1);
            }

            worker_state
                .apply_app_settings(next_settings, true)
                .map_err(map_error)?;
            sync_asset_protocol_scope(&app_handle, &worker_state).map_err(map_error)?;
            worker_state
                .db
                .insert_log(
                    "info",
                    "settings",
                    &format!(
                        "cache storage migrated from {} to {}",
                        source_root.display(),
                        destination_root.display()
                    ),
                    None,
                )
                .map_err(map_error)?;
            Ok(())
        })();

        if worker_state
            .cache_storage_migration_cancel
            .load(Ordering::SeqCst)
        {
            let _ = finish_cache_storage_migration(
                &worker_state,
                "cancelled",
                "Copy interrupted. The active cache location was left unchanged.".to_string(),
            );
            worker_state
                .cache_storage_migration_cancel
                .store(false, Ordering::SeqCst);
            return;
        }

        match copy_result {
            Ok(()) => {
                let message = if total_files == 0 {
                    "Cache location changed. No existing cache files needed copying.".to_string()
                } else {
                    format!("Copied {total_files} cache files and switched to the new location.")
                };
                let _ = finish_cache_storage_migration(&worker_state, "completed", message);
            }
            Err(error) => {
                let message = format!("Cache copy failed: {error:?}");
                let _ = finish_cache_storage_migration(&worker_state, "failed", message.clone());
                let _ = worker_state
                    .db
                    .insert_log("error", "settings", &message, None);
            }
        }
    });

    Ok(cache_storage_migration_status_snapshot(&state))
}

fn collect_all_media_assets(state: &AppState) -> Result<Vec<BatchThumbnailAsset>, InvokeError> {
    state
        .db
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.media_kind, f.path, f.file_size
                 FROM assets a
                 JOIN file_entries f ON f.id = a.primary_file_id
                 WHERE a.is_deleted = 0
                   AND f.is_deleted = 0
                   AND a.media_kind IN ('photo', 'video', 'live_photo')
                   AND NOT EXISTS (
                     SELECT 1
                     FROM asset_files af_hidden
                     WHERE af_hidden.file_id = a.primary_file_id
                       AND af_hidden.role = 'live_photo_video'
                   )
                 ORDER BY COALESCE(a.taken_at_utc, f.mtime_utc) DESC, a.id DESC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows
                .into_iter()
                .map(
                    |(asset_id, media_kind, path, file_size)| BatchThumbnailAsset {
                        asset_id,
                        primary_path: path.clone(),
                        filename: PathBuf::from(&path)
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or(&path)
                            .to_string(),
                        source_bytes: file_size.max(0) as u64,
                        needs_thumb: true,
                        needs_preview: media_kind != "video",
                    },
                )
                .collect())
        })
        .map_err(map_error)
}

fn finish_batch_thumbnail_generation(state: &AppState, message: String) {
    let mut status = state.batch_thumbnail_generation.lock();
    status.running = false;
    status.current_asset_id = None;
    status.current_filename = None;
    status.current_source_bytes = None;
    status.current_started_at = None;
    status.elapsed_ms = status
        .started_at
        .map(|started_at| started_at.elapsed().as_millis() as u64);
    status.message = Some(message);
}

fn open_in_system_browser(target: &str) -> Result<(), InvokeError> {
    let status = {
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(target)
                .status()
                .map_err(map_error)?
        }
        #[cfg(target_os = "windows")]
        {
            Command::new("cmd")
                .args(["/C", "start", "", target])
                .status()
                .map_err(map_error)?
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            Command::new("xdg-open")
                .arg(target)
                .status()
                .map_err(map_error)?
        }
    };

    if !status.success() {
        return Err(InvokeError::from(
            "Failed to open URL in the system browser".to_string(),
        ));
    }

    Ok(())
}

fn video_mime_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("mov") => "video/quicktime",
        Some("mpg" | "mpeg") => "video/mpeg",
        Some("webm") => "video/webm",
        _ => "video/mp4",
    }
}

fn primary_asset_path(state: &AppState, asset_id: i64) -> Result<PathBuf, InvokeError> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Err(InvokeError::from("Asset file path unavailable".to_string()));
    };
    Ok(PathBuf::from(primary_path))
}

fn image_mime_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        _ => "image/jpeg",
    }
}

fn image_requires_backend_orientation(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("jpg" | "jpeg" | "heic" | "heif" | "tif" | "tiff")
    )
}

fn ready_viewer_media_status(
    src: String,
    source: String,
    codec: Option<String>,
    encoder: Option<String>,
) -> ViewerMediaStatus {
    ViewerMediaStatus {
        status: "ready".to_string(),
        src: Some(src),
        source: Some(source),
        message: None,
        codec,
        encoder,
        elapsed_ms: None,
        timeout_ms: None,
        source_bytes: None,
        output_bytes: None,
    }
}

fn pending_viewer_media_status(
    message: &str,
    codec: Option<String>,
    encoder: Option<String>,
    elapsed_ms: Option<u64>,
    timeout_ms: Option<u64>,
) -> ViewerMediaStatus {
    ViewerMediaStatus {
        status: "pending".to_string(),
        src: None,
        source: None,
        message: Some(message.to_string()),
        codec,
        encoder,
        elapsed_ms,
        timeout_ms,
        source_bytes: None,
        output_bytes: None,
    }
}

fn unavailable_viewer_media_status(
    message: &str,
    codec: Option<String>,
    encoder: Option<String>,
) -> ViewerMediaStatus {
    ViewerMediaStatus {
        status: "unavailable".to_string(),
        src: None,
        source: None,
        message: Some(message.to_string()),
        codec,
        encoder,
        elapsed_ms: None,
        timeout_ms: None,
        source_bytes: None,
        output_bytes: None,
    }
}

fn load_cached_transcoded_video(
    path: &std::path::Path,
    codec: Option<String>,
    encoder: Option<String>,
) -> Result<ViewerMediaStatus, InvokeError> {
    Ok(ready_viewer_media_status(
        path.to_string_lossy().to_string(),
        "transcoded_mp4".to_string(),
        codec,
        encoder,
    ))
}

fn output_bytes_for_path(path: &std::path::Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn queue_viewer_video_transcode(
    asset_id: i64,
    source_path: PathBuf,
    state: &AppState,
    log_scope: &'static str,
) -> CommandResult<ViewerMediaStatus> {
    let codec = probe_primary_video_codec(&source_path).map_err(map_error)?;
    let duration_ms = probe_media_duration_ms(&source_path)
        .map_err(map_error)?
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0);
    let timeout_ms = duration_ms.max(VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT.as_millis() as u64);
    let source_bytes = fs::metadata(&source_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let Some(output_path) =
        viewer_video_cache_path(&source_path, &state.viewer_cache_dir()).map_err(map_error)?
    else {
        return Ok(unavailable_viewer_media_status(
            "Video playback unavailable",
            codec,
            None,
        ));
    };
    let temp_output_path = output_path.with_extension("tmp.mp4");

    if output_path.is_file() {
        let path_string = output_path.to_string_lossy().to_string();
        let _ = state
            .db
            .set_viewer_video_transcode_status(asset_id, "ready", Some(&path_string));
        let mut status = load_cached_transcoded_video(&output_path, codec, None)?;
        status.source_bytes = Some(source_bytes);
        status.output_bytes = Some(output_bytes_for_path(&output_path));
        return Ok(status);
    }

    let job_key = source_path.to_string_lossy().to_string();
    {
        let jobs = state.viewer_video_jobs.lock();
        if let Some(job) = jobs.get(&job_key) {
            match job {
                ViewerTranscodeState::Pending {
                    started_at,
                    codec,
                    encoder,
                    timeout_ms,
                    source_bytes,
                    temp_path,
                } => {
                    let mut status = pending_viewer_media_status(
                        "Transcoding video in background...",
                        codec.clone(),
                        encoder.clone(),
                        Some(started_at.elapsed().as_millis() as u64),
                        Some(*timeout_ms),
                    );
                    status.source_bytes = Some(*source_bytes);
                    status.output_bytes = Some(output_bytes_for_path(temp_path));
                    return Ok(status);
                }
                ViewerTranscodeState::Ready {
                    path,
                    codec,
                    encoder,
                } if path.is_file() => {
                    let mut status =
                        load_cached_transcoded_video(path, codec.clone(), encoder.clone())?;
                    status.source_bytes = Some(source_bytes);
                    status.output_bytes = Some(output_bytes_for_path(path));
                    return Ok(status);
                }
                ViewerTranscodeState::Unavailable {
                    codec,
                    encoder,
                    source_bytes,
                    output_bytes,
                } => {
                    let mut status = unavailable_viewer_media_status(
                        "Video transcoding unavailable",
                        codec.clone(),
                        encoder.clone(),
                    );
                    status.source_bytes = Some(*source_bytes);
                    status.output_bytes = Some(*output_bytes);
                    return Ok(status);
                }
                ViewerTranscodeState::Failed {
                    message,
                    codec,
                    encoder,
                    source_bytes,
                    output_bytes,
                } => {
                    let mut status =
                        unavailable_viewer_media_status(message, codec.clone(), encoder.clone());
                    status.source_bytes = Some(*source_bytes);
                    status.output_bytes = Some(*output_bytes);
                    return Ok(status);
                }
                ViewerTranscodeState::Ready { .. } => {}
            }
        }
    }

    state.viewer_video_jobs.lock().insert(
        job_key.clone(),
        ViewerTranscodeState::Pending {
            started_at: Instant::now(),
            codec: codec.clone(),
            encoder: None,
            timeout_ms,
            source_bytes,
            temp_path: temp_output_path.clone(),
        },
    );

    let state = state.clone();
    let codec_for_job = codec.clone();
    let temp_output_path_for_job = temp_output_path.clone();
    thread::spawn(move || {
        let (filename, file_size) = media_debug_info(&job_key);
        let viewer_cache_dir = state.viewer_cache_dir();
        let result = generate_viewer_video(
            &source_path,
            &viewer_cache_dir,
            Duration::from_millis(timeout_ms),
        );

        match result {
            Ok(Some((path, cache_hit, encoder_used))) => {
                state.viewer_video_jobs.lock().insert(
                    job_key.clone(),
                    ViewerTranscodeState::Ready {
                        path: path.clone(),
                        codec: codec_for_job.clone(),
                        encoder: Some(encoder_used.clone()),
                    },
                );
                let path_string = path.to_string_lossy().to_string();
                let _ = state.db.set_viewer_video_transcode_status(
                    asset_id,
                    "ready",
                    Some(&path_string),
                );
                let generated_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                let _ = state.db.insert_log(
                    "info",
                    log_scope,
                    &format!(
                        "asset_id={asset_id} filename=\"{filename}\" source={} encoder={} input_bytes={file_size} output_bytes={generated_bytes} output_path={}",
                        if cache_hit { "cache_hit" } else { "transcoded" },
                        encoder_used,
                        path.display(),
                    ),
                    Some(asset_id),
                );
            }
            Ok(None) => {
                state.viewer_video_jobs.lock().insert(
                    job_key.clone(),
                    ViewerTranscodeState::Unavailable {
                        codec: codec_for_job.clone(),
                        encoder: None,
                        source_bytes,
                        output_bytes: output_bytes_for_path(&temp_output_path_for_job),
                    },
                );
                let _ = state.db.insert_log(
                    "warning",
                    log_scope,
                    &format!("asset_id={asset_id} filename=\"{filename}\" transcode unavailable"),
                    Some(asset_id),
                );
            }
            Err(error) => {
                error!(asset_id, %error, "viewer background transcode failed");
                state.viewer_video_jobs.lock().insert(
                    job_key.clone(),
                    ViewerTranscodeState::Failed {
                        message: error.to_string(),
                        codec: codec_for_job.clone(),
                        encoder: None,
                        source_bytes,
                        output_bytes: output_bytes_for_path(&temp_output_path_for_job),
                    },
                );
                let _ = state
                    .db
                    .insert_log("error", log_scope, &error.to_string(), Some(asset_id));
            }
        }
    });

    let mut status = pending_viewer_media_status(
        "Transcoding video in background...",
        codec,
        None,
        Some(0),
        Some(timeout_ms),
    );
    status.source_bytes = Some(source_bytes);
    status.output_bytes = Some(output_bytes_for_path(&temp_output_path));
    Ok(status)
}

fn collect_all_video_assets(state: &AppState) -> Result<Vec<(i64, String, u64)>, InvokeError> {
    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let response = query_service::list_assets_by_date(
            &state.db,
            AssetListRequest {
                cursor,
                limit: Some(500),
                query: None,
                media_kind: Some("video".to_string()),
                date_from: None,
                date_to: None,
            },
        )
        .map_err(map_error)?;
        for asset in response.items {
            let file_size = fs::metadata(&asset.primary_path)
                .map(|meta| meta.len())
                .unwrap_or(0);
            items.push((asset.id, asset.primary_path, file_size));
        }
        if response.next_cursor.is_none() {
            break;
        }
        cursor = response.next_cursor;
    }
    Ok(items)
}

fn source_is_natively_playable(
    source_path: &str,
    codec: Option<&str>,
    support: &ViewerPlaybackSupport,
) -> bool {
    let extension = PathBuf::from(source_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    match (extension.as_deref(), codec) {
        (Some("mp4" | "m4v"), Some("h264")) => support.mp4_h264,
        (Some("mp4" | "m4v"), Some("hevc")) => support.mp4_hevc,
        (Some("mov"), Some("h264")) => support.mov_h264,
        (Some("mov"), Some("hevc")) => support.mov_hevc,
        (Some("webm"), Some(_)) => support.webm,
        _ => false,
    }
}

fn populate_missing_viewer_playback_statuses(
    asset_ids: &[i64],
    support: &ViewerPlaybackSupport,
    state: &AppState,
    known_statuses: &mut HashMap<i64, String>,
) {
    let mut queued_assets = Vec::new();

    for asset_id in asset_ids {
        if matches!(
            known_statuses.get(asset_id).map(String::as_str),
            Some("ready" | "native" | "requires_transcode")
        ) {
            continue;
        }

        if state
            .refreshed_viewer_playback_hints
            .lock()
            .contains(asset_id)
        {
            continue;
        }

        let mut inflight = state.inflight_viewer_playback_hints.lock();
        if !inflight.insert(*asset_id) {
            continue;
        }
        drop(inflight);
        queued_assets.push(*asset_id);
    }

    for asset_id in queued_assets {
        let state = state.clone();
        let support = support.clone();
        thread::spawn(move || {
            let result = (|| -> Result<Option<String>, InvokeError> {
                let detail =
                    query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
                if detail.media_kind != "video" {
                    return Ok(None);
                }
                let Some(primary_path) = detail.primary_path else {
                    return Ok(None);
                };

                let codec =
                    probe_primary_video_codec(&PathBuf::from(&primary_path)).map_err(map_error)?;
                let status =
                    if source_is_natively_playable(&primary_path, codec.as_deref(), &support) {
                        "native"
                    } else {
                        "requires_transcode"
                    };
                state
                    .db
                    .set_viewer_video_transcode_status(asset_id, status, None)
                    .map_err(map_error)?;
                Ok(Some(status.to_string()))
            })();

            state
                .inflight_viewer_playback_hints
                .lock()
                .remove(&asset_id);
            state
                .refreshed_viewer_playback_hints
                .lock()
                .insert(asset_id);

            if let Err(error) = result {
                let _ = state.db.insert_log(
                    "error",
                    "viewer_video",
                    &format!("asset_id={asset_id} playback hint refresh failed: {error:?}"),
                    Some(asset_id),
                );
            }
        });
    }
}

fn batch_viewer_transcode_status_snapshot(
    state: &BatchViewerTranscodeState,
) -> BatchViewerTranscodeStatus {
    let current_output_bytes = state
        .current_output_path
        .as_deref()
        .map(output_bytes_for_path);
    let current_elapsed_ms = state
        .current_started_at
        .map(|started_at| started_at.elapsed().as_millis() as u64);
    BatchViewerTranscodeStatus {
        status: if state.running {
            "running".to_string()
        } else if state.total > 0 {
            "completed".to_string()
        } else {
            "idle".to_string()
        },
        total: state.total,
        completed: state.completed,
        succeeded: state.completed.saturating_sub(state.skipped),
        failed: state.failed,
        skipped: state.skipped,
        stop_requested: state.stop_requested,
        current_asset_id: state.current_asset_id,
        current_filename: state.current_filename.clone(),
        current_codec: state.current_codec.clone(),
        current_width: state.current_width,
        current_height: state.current_height,
        current_duration_ms: state.current_duration_ms,
        current_source_bytes: state.current_source_bytes,
        current_output_bytes: current_output_bytes.or(state.current_output_bytes),
        current_elapsed_ms,
        elapsed_ms: state.elapsed_ms.or_else(|| {
            state
                .started_at
                .map(|started_at| started_at.elapsed().as_millis() as u64)
        }),
        message: state.message.clone(),
    }
}

#[tauri::command]
pub fn get_batch_viewer_transcode_status(
    state: State<AppState>,
) -> CommandResult<BatchViewerTranscodeStatus> {
    Ok(batch_viewer_transcode_status_snapshot(
        &state.batch_viewer_transcode.lock(),
    ))
}

#[tauri::command]
pub fn start_batch_viewer_transcode(
    state: State<AppState>,
    support: ViewerPlaybackSupport,
) -> CommandResult<BatchViewerTranscodeStatus> {
    {
        let status = state.batch_viewer_transcode.lock();
        if status.running {
            return Ok(batch_viewer_transcode_status_snapshot(&status));
        }
    }

    {
        let mut status = state.batch_viewer_transcode.lock();
        *status = BatchViewerTranscodeState {
            running: true,
            total: 0,
            completed: 0,
            failed: 0,
            skipped: 0,
            stop_requested: false,
            current_asset_id: None,
            current_filename: None,
            current_codec: None,
            current_width: None,
            current_height: None,
            current_duration_ms: None,
            current_source_bytes: None,
            current_output_bytes: None,
            current_started_at: None,
            current_output_path: None,
            started_at: Some(Instant::now()),
            elapsed_ms: None,
            message: Some("Discovering videos...".to_string()),
        };
    }

    state
        .db
        .clear_logs_by_scope(&["batch_viewer_transcode"])
        .map_err(map_error)?;

    let state = state.inner().clone();
    let worker_state = state.clone();
    thread::spawn(move || {
        let videos = match collect_all_video_assets(&worker_state) {
            Ok(videos) => videos,
            Err(error) => {
                let error_message = format!("{error:?}");
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.running = false;
                status.message = Some(format!("Failed to discover videos: {error_message}"));
                let _ = worker_state.db.insert_log(
                    "error",
                    "batch_viewer_transcode",
                    &format!("failed to discover videos: {error_message}"),
                    None,
                );
                return;
            }
        };

        {
            let mut status = worker_state.batch_viewer_transcode.lock();
            status.total = videos.len() as u32;
            status.message = Some(format!("Preparing {} videos", videos.len()));
        }

        let viewer_cache_dir = worker_state.viewer_cache_dir();
        for (index, (asset_id, source_path, source_bytes)) in videos.into_iter().enumerate() {
            {
                let status = worker_state.batch_viewer_transcode.lock();
                if status.stop_requested {
                    drop(status);
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.running = false;
                    status.current_asset_id = None;
                    status.current_filename = None;
                    status.current_codec = None;
                    status.current_width = None;
                    status.current_height = None;
                    status.current_duration_ms = None;
                    status.current_source_bytes = None;
                    status.current_output_bytes = None;
                    status.current_started_at = None;
                    status.current_output_path = None;
                    status.elapsed_ms = status
                        .started_at
                        .map(|started_at| started_at.elapsed().as_millis() as u64);
                    status.message = Some(format!(
                        "Stopped after {} processed videos",
                        status.completed + status.failed
                    ));
                    return;
                }
            }

            let filename = PathBuf::from(&source_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(&source_path)
                .to_string();
            let codec = probe_primary_video_codec(&PathBuf::from(&source_path))
                .ok()
                .flatten();
            let total = {
                let status = worker_state.batch_viewer_transcode.lock();
                status.total
            };
            let dimensions = probe_video_dimensions(&PathBuf::from(&source_path))
                .ok()
                .flatten();
            let resolution = dimensions
                .map(|(width, height)| format!("{width}x{height}"))
                .unwrap_or_else(|| "unknown".to_string());
            {
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.current_asset_id = Some(asset_id);
                status.current_filename = Some(filename.clone());
                status.current_codec = codec.clone();
                status.current_width = dimensions.map(|(width, _)| width);
                status.current_height = dimensions.map(|(_, height)| height);
                status.current_duration_ms = None;
                status.current_source_bytes = Some(source_bytes);
                status.current_output_bytes = None;
                status.current_started_at = None;
                status.current_output_path = None;
                status.message = Some(format!("Transcoding {} of {}", index + 1, total));
            }

            if source_is_natively_playable(&source_path, codec.as_deref(), &support) {
                let _ = worker_state
                    .db
                    .set_viewer_video_transcode_status(asset_id, "native", None);
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.completed += 1;
                status.skipped += 1;
                status.current_output_bytes = None;
                let _ = worker_state.db.insert_log(
                    "info",
                    "batch_viewer_transcode",
                    &format!(
                        "[skipped-native] filename=\"{filename}\" source_codec={} resolution={} source_size={}",
                        codec.clone().unwrap_or_else(|| "unknown".to_string()),
                        resolution,
                        human_size(source_bytes)
                    ),
                    Some(asset_id),
                );
                continue;
            }

            let duration_ms = probe_media_duration_ms(&PathBuf::from(&source_path))
                .ok()
                .flatten()
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(0);
            let temp_output_path =
                viewer_video_cache_path(&PathBuf::from(&source_path), &viewer_cache_dir)
                    .ok()
                    .flatten()
                    .map(|path| path.with_extension("tmp.mp4"));
            let timeout_ms = duration_ms.max(VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT.as_millis() as u64);
            let transcode_started_at = Instant::now();
            {
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.current_duration_ms = Some(duration_ms);
                status.current_started_at = Some(transcode_started_at);
                status.current_output_path = temp_output_path.clone();
                status.current_output_bytes =
                    temp_output_path.as_deref().map(output_bytes_for_path);
            }
            match generate_viewer_video(
                &PathBuf::from(&source_path),
                &viewer_cache_dir,
                Duration::from_millis(timeout_ms),
            ) {
                Ok(Some((path, cache_hit, encoder))) => {
                    let path_string = path.to_string_lossy().to_string();
                    let _ = worker_state.db.set_viewer_video_transcode_status(
                        asset_id,
                        "ready",
                        Some(&path_string),
                    );
                    let output_bytes = output_bytes_for_path(&path);
                    let transcode_elapsed =
                        human_elapsed_ms(transcode_started_at.elapsed().as_millis());
                    let video_duration = human_duration_ms(duration_ms);
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.completed += 1;
                    if cache_hit {
                        status.skipped += 1;
                    }
                    status.current_duration_ms = Some(duration_ms);
                    status.current_output_bytes = Some(output_bytes);
                    let status_label = if cache_hit {
                        "skipped-present"
                    } else {
                        "success"
                    };
                    let log_message = if cache_hit {
                        format!(
                            "[{status_label}] filename=\"{filename}\" source_codec={} encoder={} resolution={} video_duration={} output_size={}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string()),
                            encoder,
                            resolution,
                            video_duration,
                            human_size(output_bytes),
                        )
                    } else {
                        format!(
                            "[{status_label}] filename=\"{filename}\" source_codec={} encoder={} resolution={} video_duration={} transcode_duration={} output_size={}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string()),
                            encoder,
                            resolution,
                            video_duration,
                            transcode_elapsed,
                            human_size(output_bytes),
                        )
                    };
                    let _ = worker_state.db.insert_log(
                        "info",
                        "batch_viewer_transcode",
                        &log_message,
                        Some(asset_id),
                    );
                }
                Ok(None) => {
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.failed += 1;
                    status.current_duration_ms = Some(duration_ms);
                    status.current_output_bytes =
                        temp_output_path.as_deref().map(output_bytes_for_path);
                    let _ = worker_state.db.insert_log(
                        "warning",
                        "batch_viewer_transcode",
                        &format!(
                            "[unavailable] filename=\"{filename}\" source_codec={} resolution={}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string()),
                            resolution
                        ),
                        Some(asset_id),
                    );
                }
                Err(error) => {
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.failed += 1;
                    status.current_duration_ms = Some(duration_ms);
                    status.current_output_bytes =
                        temp_output_path.as_deref().map(output_bytes_for_path);
                    let _ = worker_state.db.insert_log(
                        "error",
                        "batch_viewer_transcode",
                        &format!(
                            "[failed] filename=\"{filename}\" source_codec={} resolution={} error={error}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string())
                            ,
                            resolution
                        ),
                        Some(asset_id),
                    );
                }
            }
        }

        let mut status = worker_state.batch_viewer_transcode.lock();
        status.running = false;
        status.current_asset_id = None;
        status.current_filename = None;
        status.current_codec = None;
        status.current_width = None;
        status.current_height = None;
        status.current_duration_ms = None;
        status.current_source_bytes = None;
        status.current_output_bytes = None;
        status.current_started_at = None;
        status.current_output_path = None;
        status.elapsed_ms = status
            .started_at
            .map(|started_at| started_at.elapsed().as_millis() as u64);
        status.message = Some(format!(
            "Finished {} videos with {} failures",
            status.completed, status.failed
        ));
    });

    Ok(batch_viewer_transcode_status_snapshot(
        &state.batch_viewer_transcode.lock(),
    ))
}

#[tauri::command]
pub fn stop_batch_viewer_transcode(
    state: State<AppState>,
) -> CommandResult<BatchViewerTranscodeStatus> {
    let mut status = state.batch_viewer_transcode.lock();
    status.stop_requested = true;
    if status.running {
        status.message = Some("Will stop after current file".to_string());
    }
    Ok(batch_viewer_transcode_status_snapshot(&status))
}

#[tauri::command]
pub fn refresh_index(
    request: RefreshRequest,
    state: State<AppState>,
) -> CommandResult<ImportProgress> {
    info!(roots = ?request.roots, "refresh_index");
    state.refresh_cancel.store(false, Ordering::SeqCst);
    let progress = refresh_takeout_index(&state, request).map_err(map_error)?;
    Ok(progress)
}

#[tauri::command]
pub fn start_refresh_index(request: RefreshRequest, state: State<AppState>) -> CommandResult<()> {
    if is_refresh_running(state.inner()) {
        return Err(map_error("an import is already running"));
    }

    let state = state.inner().clone();
    state.refresh_cancel.store(false, Ordering::SeqCst);
    thread::spawn(move || {
        if let Err(error) = refresh_takeout_index(&state, request) {
            let message = error.to_string();
            if is_refresh_cancelled_message(&message) {
                return;
            }
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
            let _ = state.db.insert_log(
                "error",
                "import",
                &format!("background refresh failed: {message}"),
                None,
            );
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
    prefer_preview_cache: Option<bool>,
    state: State<AppState>,
) -> CommandResult<Option<String>> {
    let use_preview_cache = prefer_preview_cache.unwrap_or(false);
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path.clone() else {
        return Ok(None);
    };
    let primary_path_buf = PathBuf::from(&primary_path);
    let key = thumbnail_cache_key_for_path(&primary_path_buf, size, use_preview_cache);
    let cache = if use_preview_cache {
        &state.preview_cache
    } else {
        &state.thumbnail_cache
    };
    if let Some(path) = cache.lock().cached_path(&key) {
        return Ok(Some(path.to_string_lossy().to_string()));
    }
    let (filename, file_size) = media_debug_info(&primary_path);
    let kind = thumb_log_kind(use_preview_cache);
    let generator = thumbnail_generator_label(&primary_path_buf);
    record_thumb_log(
        state.inner(),
        "info",
        asset_id,
        format!(
            "kind={kind} generator={generator} status=start mode=direct size={size}px filename=\"{filename}\" file_size={}",
            human_size(file_size)
        ),
    )?;

    let working_dir = state.working_dir();
    match generate_thumbnail(&primary_path_buf, size, !use_preview_cache, &working_dir) {
        Ok(result) => {
            let generated = result.bytes;
            if let Some(bytes) = generated {
                record_thumb_log(
                    state.inner(),
                    "info",
                    asset_id,
                    format!(
                        "kind={kind} generator={generator} status=success mode=direct size={size}px filename=\"{filename}\" file_size={} generated_size={}",
                        human_size(file_size),
                        human_size(bytes.len() as u64),
                    ),
                )?;
                let mut cache = cache.lock();
                if use_preview_cache {
                    for replacement_key in
                        preview_cache_replacement_keys_for_path(&primary_path_buf, size)
                    {
                        cache.remove(&replacement_key);
                    }
                }
                cache.insert(key.clone(), bytes);
                Ok(cache
                    .cached_path(&key)
                    .map(|path| path.to_string_lossy().to_string()))
            } else {
                record_thumb_log(
                    state.inner(),
                    "warning",
                    asset_id,
                    format!(
                        "kind={kind} generator={generator} status=unavailable mode=direct size={size}px filename=\"{filename}\" file_size={}",
                        human_size(file_size),
                    ),
                )?;
                Ok(None)
            }
        }
        Err(error) => {
            error!(asset_id, %error, "thumbnail generation failed");
            preview_debug_log(format!(
                "thumbnail asset_id={asset_id} filename=\"{filename}\" file_size={} failed error={error}",
                file_size
            ));
            record_thumb_log(
                state.inner(),
                "error",
                asset_id,
                format!(
                    "kind={kind} generator={generator} status=failed mode=direct size={size}px filename=\"{filename}\" file_size={} error={error}",
                    human_size(file_size)
                ),
            )?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn request_thumbnails_batch(
    asset_ids: Vec<i64>,
    size: u32,
    prefer_preview_cache: Option<bool>,
    check_cache_only: Option<bool>,
    state: State<AppState>,
) -> CommandResult<Vec<ThumbnailBatchItem>> {
    let generation = state.thumbnail_generation.load(Ordering::SeqCst);
    let use_preview_cache = prefer_preview_cache.unwrap_or(false);
    let check_cache_only = check_cache_only.unwrap_or(false);
    let cache = if use_preview_cache {
        state.preview_cache.clone()
    } else {
        state.thumbnail_cache.clone()
    };

    let items = asset_ids
        .into_iter()
        .map(|asset_id| {
            let detail = match query_service::get_asset_detail(&state.db, asset_id) {
                Ok(detail) => detail,
                Err(_) => {
                    return ThumbnailBatchItem {
                        asset_id,
                        status: "unavailable".to_string(),
                        data_url: None,
                    };
                }
            };
            let Some(primary_path) = detail.primary_path else {
                return ThumbnailBatchItem {
                    asset_id,
                    status: "unavailable".to_string(),
                    data_url: None,
                };
            };
            let primary_path_buf = PathBuf::from(primary_path);
            let key = thumbnail_cache_key_for_path(&primary_path_buf, size, use_preview_cache);

            if use_preview_cache {
                let cached_path = {
                    let cache_guard = cache.lock();
                    cache_guard.cached_path(&key)
                };
                if let Some(cached_path) = cached_path {
                    preview_debug_log(format!(
                        "thumbnail_batch_item asset_id={} size={} status=ready source=cache_path_only",
                        asset_id,
                        size
                    ));
                    return ThumbnailBatchItem {
                        asset_id,
                        status: "ready".to_string(),
                        data_url: Some(cached_path.to_string_lossy().to_string()),
                    };
                }
            } else {
                let cached_path = {
                    let mut cache_guard = cache.lock();
                    cache_guard
                        .get(&key)
                        .map(|bytes| (bytes.len(), cache_guard.cached_path(&key)))
                };
                if let Some((bytes_len, cached_path)) = cached_path {
                    preview_debug_log(format!(
                        "thumbnail_batch_item asset_id={} size={} status=ready source=cache bytes={}",
                        asset_id,
                        size,
                        bytes_len
                    ));
                    return ThumbnailBatchItem {
                        asset_id,
                        status: "ready".to_string(),
                        data_url: cached_path.map(|path| path.to_string_lossy().to_string()),
                    };
                }
            }

            if state.failed_thumbnails.lock().contains(&key) {
                preview_debug_log(format!(
                    "thumbnail_batch_item asset_id={} size={} status=unavailable source=failed_cache",
                    asset_id, size
                ));
                return ThumbnailBatchItem {
                    asset_id,
                    status: "unavailable".to_string(),
                    data_url: None,
                };
            }

            if check_cache_only {
                if state.inflight_thumbnails.lock().contains(&key) {
                    preview_debug_log(format!(
                        "thumbnail_batch_item asset_id={} size={} status=pending source=inflight_check_only",
                        asset_id, size
                    ));
                    return ThumbnailBatchItem {
                        asset_id,
                        status: "pending".to_string(),
                        data_url: None,
                    };
                }

                if use_preview_cache {
                    let effective_target_size =
                        effective_preview_size_for_asset(state.inner(), asset_id, size)
                            .unwrap_or(size);
                    let has_equivalent_variant = {
                        let cache_guard = cache.lock();
                        preview_cache_replacement_keys_for_path(&primary_path_buf, size)
                            .into_iter()
                            .filter_map(|replacement_key| {
                                let alt_size = replacement_key.rsplit(':').next()?.parse::<u32>().ok()?;
                                cache_guard.cached_path(&replacement_key).is_some().then_some(alt_size)
                            })
                            .any(|alt_size| alt_size >= effective_target_size)
                    };
                    if has_equivalent_variant {
                        preview_debug_log(format!(
                            "thumbnail_batch_item asset_id={} size={} status=ready source=equivalent_preview_variant effective_target_size={}",
                            asset_id, size, effective_target_size
                        ));
                        return ThumbnailBatchItem {
                            asset_id,
                            status: "ready".to_string(),
                            data_url: None,
                        };
                    }

                    let has_stale_variant = {
                        let cache_guard = cache.lock();
                        preview_cache_replacement_keys_for_path(&primary_path_buf, size)
                            .into_iter()
                            .any(|replacement_key| cache_guard.cached_path(&replacement_key).is_some())
                    };
                    if has_stale_variant {
                        preview_debug_log(format!(
                            "thumbnail_batch_item asset_id={} size={} status=stale source=alternate_preview_variant",
                            asset_id, size
                        ));
                        return ThumbnailBatchItem {
                            asset_id,
                            status: "stale".to_string(),
                            data_url: None,
                        };
                    }
                }

                preview_debug_log(format!(
                    "thumbnail_batch_item asset_id={} size={} status=missing source=cache_check_only",
                    asset_id, size
                ));
                return ThumbnailBatchItem {
                    asset_id,
                    status: "missing".to_string(),
                    data_url: None,
                };
            }

            let mut inflight = state.inflight_thumbnails.lock();
            if inflight.insert(key.clone()) {
                drop(inflight);
                preview_debug_log(format!(
                    "thumbnail_batch_item asset_id={} size={} status=pending enqueue=start",
                    asset_id, size
                ));
                let sender = if use_preview_cache {
                    &state.preview_job_sender
                } else {
                    &state.thumbnail_job_sender
                };
                if !use_preview_cache {
                    state.thumb_backlog.fetch_add(1, Ordering::SeqCst);
                }
                if let Err(error) = sender.send(ThumbnailJob {
                    asset_id,
                    size,
                    key: key.clone(),
                    generation,
                    use_preview_cache,
                }) {
                    if !use_preview_cache {
                        state.thumb_backlog.fetch_sub(1, Ordering::SeqCst);
                    }
                    state.inflight_thumbnails.lock().remove(&key);
                    state.failed_thumbnails.lock().insert(key.clone());
                    let _ = state.db.insert_log(
                        "error",
                        "thumbnail_batch",
                        &format!("failed to enqueue thumbnail job: {error}"),
                        Some(asset_id),
                    );
                    preview_debug_log(format!(
                        "thumbnail_batch_item asset_id={} size={} status=unavailable enqueue=failed error={error}",
                        asset_id, size
                    ));
                } else {
                    preview_debug_log(format!(
                        "thumbnail_batch_item asset_id={} size={} status=pending enqueue=ok",
                        asset_id, size
                    ));
                }
            } else {
                drop(inflight);
                preview_debug_log(format!(
                    "thumbnail_batch_item asset_id={} size={} status=pending source=inflight",
                    asset_id, size
                ));
            }

            ThumbnailBatchItem {
                asset_id,
                status: "pending".to_string(),
                data_url: None,
            }
        })
        .collect::<Vec<_>>();

    if use_preview_cache {
        let ready = items.iter().filter(|item| item.status == "ready").count();
        let pending = items.iter().filter(|item| item.status == "pending").count();
        let unavailable = items
            .iter()
            .filter(|item| item.status == "unavailable")
            .count();
        let _ = state.db.insert_log(
            "info",
            "thumbnail_batch",
            &format!(
                "kind=preview status=batch requested={} ready={} pending={} unavailable={}",
                items.len(),
                ready,
                pending,
                unavailable,
            ),
            None,
        );
    }

    Ok(items)
}

#[tauri::command]
pub fn get_batch_thumbnail_generation_status(
    state: State<AppState>,
) -> CommandResult<BatchThumbnailGenerationStatus> {
    Ok(batch_thumbnail_generation_status_snapshot(
        &state.batch_thumbnail_generation.lock(),
    ))
}

#[tauri::command]
pub fn start_batch_thumbnail_generation(
    state: State<AppState>,
) -> CommandResult<BatchThumbnailGenerationStatus> {
    {
        let status = state.batch_thumbnail_generation.lock();
        if status.running {
            return Ok(batch_thumbnail_generation_status_snapshot(&status));
        }
    }

    {
        let mut status = state.batch_thumbnail_generation.lock();
        *status = BatchThumbnailGenerationState {
            running: true,
            total: 0,
            completed: 0,
            failed: 0,
            skipped: 0,
            stop_requested: false,
            current_asset_id: None,
            current_filename: None,
            current_source_bytes: None,
            current_started_at: None,
            started_at: Some(Instant::now()),
            elapsed_ms: None,
            message: Some("Discovering media...".to_string()),
        };
    }

    state
        .db
        .clear_logs_by_scope(&["thumb_gen"])
        .map_err(map_error)?;

    let worker_state = state.inner().clone();
    thread::spawn(move || {
        const THUMB_SIZE: u32 = 210;
        let preview_size = worker_state.viewer_preview_size();
        let assets = match collect_all_media_assets(&worker_state) {
            Ok(assets) => assets,
            Err(error) => {
                let error_message = format!("{error:?}");
                let mut status = worker_state.batch_thumbnail_generation.lock();
                status.running = false;
                status.message = Some(format!("Failed to discover media: {error_message}"));
                let _ = worker_state.db.insert_log(
                    "error",
                    "thumb_gen",
                    &format!("kind=batch status=failed error={error_message}"),
                    None,
                );
                return;
            }
        };

        {
            let mut status = worker_state.batch_thumbnail_generation.lock();
            status.total = assets.len() as u32;
            status.message = Some(format!("Preparing {} assets", assets.len()));
        }

        let mut pending = VecDeque::from(assets);
        let mut active = HashMap::<i64, BatchThumbnailAsset>::new();

        loop {
            let stop_requested = worker_state
                .batch_thumbnail_generation
                .lock()
                .stop_requested;

            while !stop_requested && active.len() < worker_state.thumbnail_worker_count {
                let Some(asset) = pending.pop_front() else {
                    break;
                };
                let primary_path = PathBuf::from(&asset.primary_path);

                let thumb_state =
                    thumbnail_batch_output_state(&worker_state, &primary_path, THUMB_SIZE, false);
                let preview_state = if asset.needs_preview {
                    thumbnail_batch_output_state(&worker_state, &primary_path, preview_size, true)
                } else {
                    ThumbnailBatchOutputState::Ready
                };

                let thumb_terminal = matches!(
                    thumb_state,
                    ThumbnailBatchOutputState::Ready | ThumbnailBatchOutputState::Failed
                );
                let preview_terminal = matches!(
                    preview_state,
                    ThumbnailBatchOutputState::Ready | ThumbnailBatchOutputState::Failed
                );

                if thumb_terminal && preview_terminal {
                    let mut status = worker_state.batch_thumbnail_generation.lock();
                    if matches!(thumb_state, ThumbnailBatchOutputState::Failed)
                        || matches!(preview_state, ThumbnailBatchOutputState::Failed)
                    {
                        status.failed += 1;
                    } else {
                        status.completed += 1;
                        status.skipped += 1;
                        let preview_suffix = if asset.needs_preview {
                            format!(" preview_size={}px", preview_size)
                        } else {
                            "".to_string()
                        };
                        let _ = worker_state.db.insert_log(
                            "info",
                            "thumb_gen",
                            &format!(
                                "[skipped] filename=\"{}\" source_size={} reason=existing thumb_size={}px{}",
                                asset.filename,
                                human_size(asset.source_bytes),
                                THUMB_SIZE,
                                preview_suffix
                            ),
                            Some(asset.asset_id),
                        );
                    }
                    continue;
                }

                if asset.needs_thumb && matches!(thumb_state, ThumbnailBatchOutputState::Missing) {
                    let _ = enqueue_thumbnail_job(
                        &worker_state,
                        asset.asset_id,
                        &primary_path,
                        THUMB_SIZE,
                        false,
                        worker_state.thumbnail_generation.load(Ordering::SeqCst),
                    );
                }
                if asset.needs_preview
                    && matches!(preview_state, ThumbnailBatchOutputState::Missing)
                {
                    let _ = enqueue_thumbnail_job(
                        &worker_state,
                        asset.asset_id,
                        &primary_path,
                        preview_size,
                        true,
                        worker_state.thumbnail_generation.load(Ordering::SeqCst),
                    );
                }

                active.insert(asset.asset_id, asset);
            }

            let mut finished_ids = Vec::new();
            for (asset_id, asset) in &active {
                let primary_path = PathBuf::from(&asset.primary_path);
                let thumb_state =
                    thumbnail_batch_output_state(&worker_state, &primary_path, THUMB_SIZE, false);
                let preview_state = if asset.needs_preview {
                    thumbnail_batch_output_state(&worker_state, &primary_path, preview_size, true)
                } else {
                    ThumbnailBatchOutputState::Ready
                };

                let thumb_terminal = matches!(
                    thumb_state,
                    ThumbnailBatchOutputState::Ready | ThumbnailBatchOutputState::Failed
                );
                let preview_terminal = matches!(
                    preview_state,
                    ThumbnailBatchOutputState::Ready | ThumbnailBatchOutputState::Failed
                );

                if thumb_terminal && preview_terminal {
                    let mut status = worker_state.batch_thumbnail_generation.lock();
                    if matches!(thumb_state, ThumbnailBatchOutputState::Failed)
                        || matches!(preview_state, ThumbnailBatchOutputState::Failed)
                    {
                        status.failed += 1;
                    } else {
                        status.completed += 1;
                    }
                    finished_ids.push(*asset_id);
                }
            }

            for asset_id in finished_ids {
                active.remove(&asset_id);
            }

            {
                let mut status = worker_state.batch_thumbnail_generation.lock();
                let processed = status.completed + status.failed;
                status.message = Some(if stop_requested {
                    format!(
                        "Finishing {} active assets • {processed}/{} processed",
                        active.len(),
                        status.total
                    )
                } else {
                    format!(
                        "{} active • {} queued • {processed}/{} processed",
                        active.len(),
                        pending.len(),
                        status.total
                    )
                });
            }

            if stop_requested && active.is_empty() {
                let processed = {
                    let status = worker_state.batch_thumbnail_generation.lock();
                    status.completed + status.failed
                };
                finish_batch_thumbnail_generation(
                    &worker_state,
                    format!("Stopped after {processed} processed assets"),
                );
                return;
            }

            if active.is_empty() && pending.is_empty() {
                let (completed, failed) = {
                    let status = worker_state.batch_thumbnail_generation.lock();
                    (status.completed, status.failed)
                };
                finish_batch_thumbnail_generation(
                    &worker_state,
                    format!("Finished {completed} assets with {failed} failures"),
                );
                return;
            }

            thread::sleep(Duration::from_millis(40));
        }
    });

    Ok(batch_thumbnail_generation_status_snapshot(
        &state.batch_thumbnail_generation.lock(),
    ))
}

#[tauri::command]
pub fn stop_batch_thumbnail_generation(
    state: State<AppState>,
) -> CommandResult<BatchThumbnailGenerationStatus> {
    let mut status = state.batch_thumbnail_generation.lock();
    status.stop_requested = true;
    if status.running {
        status.message = Some("Will stop after current asset".to_string());
    }
    Ok(batch_thumbnail_generation_status_snapshot(&status))
}

#[tauri::command]
pub fn get_viewer_playback_hints(
    asset_ids: Vec<i64>,
    support: ViewerPlaybackSupport,
    state: State<AppState>,
) -> CommandResult<Vec<ViewerPlaybackHint>> {
    let mut statuses = state
        .db
        .get_viewer_video_playback_statuses(&asset_ids)
        .map_err(map_error)?;
    populate_missing_viewer_playback_statuses(&asset_ids, &support, state.inner(), &mut statuses);

    Ok(asset_ids
        .into_iter()
        .map(|asset_id| ViewerPlaybackHint {
            asset_id,
            status: match statuses.get(&asset_id).map(String::as_str) {
                Some("ready") => "transcoded".to_string(),
                Some("native") => "native".to_string(),
                _ => "none".to_string(),
            },
        })
        .collect())
}

#[tauri::command]
pub fn load_viewer_frame(
    asset_id: i64,
    prefer_original: Option<bool>,
    state: State<AppState>,
) -> CommandResult<Option<String>> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Ok(None);
    };
    let (filename, file_size) = media_debug_info(&primary_path);
    let source_path = PathBuf::from(&primary_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original {
        if image_requires_backend_orientation(&source_path) {
            let working_dir = state.working_dir();
            if let Some(bytes) =
                generate_viewer_image(&source_path, u32::MAX, &working_dir).map_err(map_error)?
            {
                return Ok(Some(format!(
                    "data:image/jpeg;base64,{}",
                    STANDARD.encode(bytes)
                )));
            }
        }
        let bytes = fs::read(&source_path).map_err(map_error)?;
        return Ok(Some(format!(
            "data:{};base64,{}",
            image_mime_type(&source_path),
            STANDARD.encode(bytes)
        )));
    }

    let viewer_cache_dir = state.viewer_cache_dir();
    let working_dir = state.working_dir();
    match generate_viewer_image_file(&source_path, 2400, &viewer_cache_dir, &working_dir) {
        Ok(Some(path)) => {
            let generated_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            preview_debug_log(format!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} generated_bytes={}",
                file_size, generated_bytes
            ));
            Ok(Some(path.to_string_lossy().to_string()))
        }
        Ok(None) => {
            preview_debug_log(format!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} unavailable",
                file_size
            ));
            Ok(None)
        }
        Err(error) => {
            error!(asset_id, %error, "viewer frame load failed");
            preview_debug_log(format!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} failed error={error}",
                file_size
            ));
            state
                .db
                .insert_log("error", "viewer", &error.to_string(), Some(asset_id))
                .map_err(map_error)?;
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn load_viewer_video(
    asset_id: i64,
    prefer_original: Option<bool>,
    state: State<AppState>,
) -> CommandResult<ViewerMediaStatus> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Ok(unavailable_viewer_media_status(
            "Video playback unavailable",
            None,
            None,
        ));
    };
    let (filename, file_size) = media_debug_info(&primary_path);
    let source_path = PathBuf::from(&primary_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original && can_stream_original_video_bytes(&source_path) {
        let codec = probe_primary_video_codec(&source_path).map_err(map_error)?;
        state
            .db
            .set_viewer_video_transcode_status(asset_id, "native", None)
            .map_err(map_error)?;
        state
            .db
            .insert_log(
                "info",
                "viewer_video",
                &format!(
                    "asset_id={asset_id} filename=\"{filename}\" source=original_bytes input_bytes={file_size}"
                ),
                Some(asset_id),
            )
            .map_err(map_error)?;
        return Ok(ready_viewer_media_status(
            source_path.to_string_lossy().to_string(),
            match video_mime_type(&source_path) {
                "video/quicktime" => "original_quicktime".to_string(),
                "video/webm" => "original_webm".to_string(),
                _ => "original_mp4".to_string(),
            },
            codec,
            None,
        ));
    }

    let _ = (filename, file_size);
    queue_viewer_video_transcode(asset_id, source_path, state.inner(), "viewer_video")
}

#[tauri::command]
pub fn load_live_photo_motion(
    asset_id: i64,
    prefer_original: Option<bool>,
    state: State<AppState>,
) -> CommandResult<ViewerMediaStatus> {
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(motion_path) = detail.live_photo_video_path else {
        return Ok(unavailable_viewer_media_status(
            "Live photo playback unavailable",
            None,
            None,
        ));
    };
    let (filename, file_size) = media_debug_info(&motion_path);
    let source_path = PathBuf::from(&motion_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original && can_stream_original_video_bytes(&source_path) {
        let codec = probe_primary_video_codec(&source_path).map_err(map_error)?;
        state
            .db
            .insert_log(
                "info",
                "live_photo",
                &format!(
                    "asset_id={asset_id} filename=\"{filename}\" source=original_bytes input_bytes={file_size}"
                ),
                Some(asset_id),
            )
            .map_err(map_error)?;
        return Ok(ready_viewer_media_status(
            source_path.to_string_lossy().to_string(),
            match video_mime_type(&source_path) {
                "video/quicktime" => "original_quicktime".to_string(),
                "video/webm" => "original_webm".to_string(),
                _ => "original_mp4".to_string(),
            },
            codec,
            None,
        ));
    }

    let _ = (filename, file_size);
    queue_viewer_video_transcode(asset_id, source_path, state.inner(), "live_photo")
}

#[tauri::command]
pub fn get_live_photo_pair(asset_id: i64, state: State<AppState>) -> CommandResult<Option<String>> {
    query_service::get_live_photo_pair(&state.db, asset_id).map_err(map_error)
}

#[tauri::command]
pub fn get_cache_stats(state: State<AppState>) -> CommandResult<CacheStats> {
    let mut stats = state.thumbnail_cache.lock().stats();
    let preview_stats = state.preview_cache.lock().stats();
    stats.preview_items = preview_stats.thumbnail_items;
    stats.preview_bytes = preview_stats.thumbnail_bytes;
    stats.preview_budget_bytes = preview_stats.thumbnail_budget_bytes;
    let (viewer_render_items, viewer_render_bytes) =
        viewer_render_cache_stats(&state.viewer_cache_dir()).map_err(map_error)?;
    stats.viewer_render_items = viewer_render_items;
    stats.viewer_render_bytes = viewer_render_bytes;
    Ok(stats)
}

#[tauri::command]
pub fn clear_thumbnail_cache(state: State<AppState>) -> CommandResult<()> {
    state.thumbnail_generation.fetch_add(1, Ordering::SeqCst);
    state.thumbnail_cache.lock().clear();
    state.preview_cache.lock().clear();
    state.inflight_thumbnails.lock().clear();
    state.failed_thumbnails.lock().clear();
    state.viewer_video_jobs.lock().clear();
    *state.batch_thumbnail_generation.lock() = BatchThumbnailGenerationState::idle();
    state
        .db
        .insert_log(
            "info",
            "thumbnail",
            "cleared thumbnail and preview caches",
            None,
        )
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn clear_viewer_render_cache_command(state: State<AppState>) -> CommandResult<()> {
    clear_viewer_render_cache(&state.viewer_cache_dir()).map_err(map_error)?;
    state.viewer_video_jobs.lock().clear();
    state
        .db
        .clear_viewer_video_transcode_statuses()
        .map_err(map_error)?;
    state
        .db
        .insert_log("info", "viewer", "cleared viewer render cache", None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn get_recent_logs(limit: Option<u32>, state: State<AppState>) -> CommandResult<Vec<LogEntry>> {
    query_service::get_recent_logs(&state.db, limit.unwrap_or(10_000)).map_err(map_error)
}

#[tauri::command]
pub fn get_thumb_generation_logs(
    limit: Option<u32>,
    state: State<AppState>,
) -> CommandResult<Vec<LogEntry>> {
    query_service::get_logs_by_scope(&state.db, &["thumb_gen"], limit.unwrap_or(10_000))
        .map_err(map_error)
}

#[tauri::command]
pub fn get_batch_viewer_transcode_logs(
    limit: Option<u32>,
    state: State<AppState>,
) -> CommandResult<Vec<LogEntry>> {
    query_service::get_logs_by_scope(
        &state.db,
        &["batch_viewer_transcode"],
        limit.unwrap_or(10_000),
    )
    .map_err(map_error)
}

#[tauri::command]
pub fn clear_thumb_generation_logs(state: State<AppState>) -> CommandResult<()> {
    state
        .db
        .clear_logs_by_scope(&["thumb_gen"])
        .map_err(map_error)
}

#[tauri::command]
pub fn clear_batch_viewer_transcode_logs(state: State<AppState>) -> CommandResult<()> {
    state
        .db
        .clear_logs_by_scope(&["batch_viewer_transcode"])
        .map_err(map_error)
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
    if is_refresh_running(state.inner()) {
        state.refresh_cancel.store(true, Ordering::SeqCst);
        {
            let mut status = state.import_status.lock();
            if let Some(progress) = status.as_mut() {
                progress.message = Some("stopping refresh before clearing database".to_string());
            }
        }
        wait_for_refresh_shutdown(state.inner(), Duration::from_secs(30))?;
    }

    state.db.reset().map_err(map_error)?;
    state.refresh_cancel.store(false, Ordering::SeqCst);
    *state.import_status.lock() = None;
    clear_app_derived_storage(state.inner())?;
    state
        .db
        .insert_log(
            "warning",
            "reset",
            "local database reset to default state",
            None,
        )
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

#[tauri::command]
pub fn reveal_asset_in_file_manager(asset_id: i64, state: State<AppState>) -> CommandResult<()> {
    let path = primary_asset_path(state.inner(), asset_id)?;
    reveal_path_in_file_manager(&path)
}

#[tauri::command]
pub fn open_asset_with_default_app(asset_id: i64, state: State<AppState>) -> CommandResult<()> {
    let started = Instant::now();
    let path = primary_asset_path(state.inner(), asset_id)?;
    let lookup_elapsed_ms = started.elapsed().as_millis();
    let launch_started = Instant::now();
    let result = open_path_with_default_app(&path);
    let launch_elapsed_ms = launch_started.elapsed().as_millis();
    let total_elapsed_ms = started.elapsed().as_millis();
    let message = match &result {
        Ok(()) => format!(
            "open_default_app asset_id={asset_id} status=ok lookup_ms={lookup_elapsed_ms} launch_ms={launch_elapsed_ms} total_ms={total_elapsed_ms} path=\"{}\"",
            path.display()
        ),
        Err(error) => format!(
            "open_default_app asset_id={asset_id} status=failed lookup_ms={lookup_elapsed_ms} launch_ms={launch_elapsed_ms} total_ms={total_elapsed_ms} path=\"{}\" error={error:?}",
            path.display()
        ),
    };
    let _ = state.db.insert_log(
        if result.is_ok() { "info" } else { "error" },
        "shell",
        &message,
        Some(asset_id),
    );
    result
}

#[tauri::command]
pub fn open_asset_preview(asset_id: i64, state: State<AppState>) -> CommandResult<()> {
    let path = primary_asset_path(state.inner(), asset_id)?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let is_video = matches!(
        extension.as_deref(),
        Some("mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" | "mts" | "m2ts" | "3gp")
    );
    open_path_with_preview(&path, is_video)
}

#[tauri::command]
pub fn open_url_in_browser(url: String) -> CommandResult<()> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(InvokeError::from(
            "Only http(s) URLs can be opened in the browser".to_string(),
        ));
    }
    open_in_system_browser(trimmed)
}

pub fn command_handlers() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool {
    generate_handler![
        get_app_settings,
        update_app_settings,
        inspect_app_backup,
        export_app_backup,
        import_app_backup,
        get_cache_storage_migration_status,
        start_cache_storage_migration,
        cancel_cache_storage_migration,
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
        get_batch_thumbnail_generation_status,
        start_batch_thumbnail_generation,
        stop_batch_thumbnail_generation,
        get_viewer_playback_hints,
        load_viewer_frame,
        load_viewer_video,
        load_live_photo_motion,
        get_live_photo_pair,
        get_cache_stats,
        get_batch_viewer_transcode_status,
        start_batch_viewer_transcode,
        stop_batch_viewer_transcode,
        clear_thumbnail_cache,
        clear_viewer_render_cache_command,
        get_recent_logs,
        get_thumb_generation_logs,
        get_batch_viewer_transcode_logs,
        clear_thumb_generation_logs,
        clear_batch_viewer_transcode_logs,
        record_client_log,
        reveal_asset_in_file_manager,
        open_asset_with_default_app,
        open_asset_preview,
        open_url_in_browser,
        reset_local_database,
        clear_diagnostics,
        clear_logs
    ]
}
