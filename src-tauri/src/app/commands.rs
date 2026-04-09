use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use tauri::{State, generate_handler, ipc::InvokeError};
use tracing::{error, info};

use crate::{
    app::state::{AppState, BatchViewerTranscodeState, ThumbnailJob, ViewerTranscodeState},
    db::DatabaseQueries,
    import::refresher::refresh_takeout_index,
    media::thumb::{
        clear_viewer_render_cache, generate_thumbnail, generate_viewer_image,
        generate_viewer_image_file, generate_viewer_video, probe_media_duration_ms,
        probe_primary_video_codec,
        thumbnail_generator_label, viewer_render_cache_stats, viewer_video_cache_path,
        VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT,
    },
    models::{
        AlbumSummary, AssetDetail, AssetListRequest, AssetListResponse, BatchViewerTranscodeStatus,
        CacheStats, DiagnosticEntry, ImportProgress, LogEntry, RefreshRequest,
        ThumbnailBatchItem, ViewerMediaStatus, ViewerPlaybackSupport,
    },
    search::query_service,
};

type CommandResult<T> = Result<T, InvokeError>;
const PREVIEW_DEBUG_LOGS: bool = false;
const VIEWER_PREVIEW_SIZE: u32 = 1024;

fn map_error<E: std::fmt::Display>(error: E) -> InvokeError {
    InvokeError::from(error.to_string())
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

fn thumb_log_kind(size: u32) -> &'static str {
    if size >= VIEWER_PREVIEW_SIZE {
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

fn video_mime_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        _ => "video/mp4",
    }
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
    let bytes = fs::read(path).map_err(map_error)?;
    Ok(ready_viewer_media_status(
        format!("data:video/mp4;base64,{}", STANDARD.encode(bytes)),
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
    let source_bytes = fs::metadata(&source_path).map(|meta| meta.len()).unwrap_or(0);
    let Some(output_path) = viewer_video_cache_path(&source_path, &state.app_data_dir.join("viewer-cache"))
        .map_err(map_error)?
    else {
        return Ok(unavailable_viewer_media_status("Video playback unavailable", codec, None));
    };
    let temp_output_path = output_path.with_extension("tmp.mp4");

    if output_path.is_file() {
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
                ViewerTranscodeState::Ready { path, codec, encoder } if path.is_file() => {
                    let mut status = load_cached_transcoded_video(
                        path,
                        codec.clone(),
                        encoder.clone(),
                    )?;
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
                    let mut status =
                        unavailable_viewer_media_status("Video transcoding unavailable", codec.clone(), encoder.clone());
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
                    let mut status = unavailable_viewer_media_status(message, codec.clone(), encoder.clone());
                    status.source_bytes = Some(*source_bytes);
                    status.output_bytes = Some(*output_bytes);
                    return Ok(status);
                }
                ViewerTranscodeState::Ready { .. } => {}
            }
        }
    }

    state
        .viewer_video_jobs
        .lock()
        .insert(
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
        let viewer_cache_dir = state.app_data_dir.join("viewer-cache");
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
                state
                    .viewer_video_jobs
                    .lock()
                    .insert(
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
            let file_size = fs::metadata(&asset.primary_path).map(|meta| meta.len()).unwrap_or(0);
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

fn batch_viewer_transcode_status_snapshot(
    state: &BatchViewerTranscodeState,
) -> BatchViewerTranscodeStatus {
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
        failed: state.failed,
        skipped: state.skipped,
        stop_requested: state.stop_requested,
        current_asset_id: state.current_asset_id,
        current_filename: state.current_filename.clone(),
        current_codec: state.current_codec.clone(),
        current_source_bytes: state.current_source_bytes,
        current_output_bytes: state.current_output_bytes,
        elapsed_ms: state
            .started_at
            .map(|started_at| started_at.elapsed().as_millis() as u64),
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
            current_source_bytes: None,
            current_output_bytes: None,
            started_at: Some(Instant::now()),
            message: Some("Discovering videos...".to_string()),
        };
    }

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

        let viewer_cache_dir = worker_state.app_data_dir.join("viewer-cache");
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
                    status.current_source_bytes = None;
                    status.current_output_bytes = None;
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
            {
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.current_asset_id = Some(asset_id);
                status.current_filename = Some(filename.clone());
                status.current_codec = codec.clone();
                status.current_source_bytes = Some(source_bytes);
                status.current_output_bytes = None;
                status.message = Some(format!("Transcoding {} of {}", index + 1, total));
            }

            if source_is_natively_playable(&source_path, codec.as_deref(), &support) {
                let mut status = worker_state.batch_viewer_transcode.lock();
                status.completed += 1;
                status.skipped += 1;
                status.current_output_bytes = None;
                    let _ = worker_state.db.insert_log(
                        "info",
                        "batch_viewer_transcode",
                        &format!(
                            "asset_id={asset_id} filename=\"{filename}\" status=skipped reason=native_format source_codec={} source_size={}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string()),
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
            let timeout_ms = duration_ms.max(VIEWER_VIDEO_TRANSCODE_MIN_TIMEOUT.as_millis() as u64);
            match generate_viewer_video(
                &PathBuf::from(&source_path),
                &viewer_cache_dir,
                Duration::from_millis(timeout_ms),
            ) {
                Ok(Some((path, cache_hit, encoder))) => {
                    let output_bytes = output_bytes_for_path(&path);
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.completed += 1;
                    if cache_hit {
                        status.skipped += 1;
                    }
                    status.current_output_bytes = Some(output_bytes);
                    let _ = worker_state.db.insert_log(
                        "info",
                        "batch_viewer_transcode",
                        &format!(
                            "asset_id={asset_id} filename=\"{filename}\" status=success source={} source_codec={} encoder={} output_size={}",
                            if cache_hit { "cache_hit" } else { "transcoded" },
                            codec.clone().unwrap_or_else(|| "unknown".to_string()),
                            encoder,
                            human_size(output_bytes),
                        ),
                        Some(asset_id),
                    );
                }
                Ok(None) => {
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.failed += 1;
                    let _ = worker_state.db.insert_log(
                        "warning",
                        "batch_viewer_transcode",
                        &format!(
                            "asset_id={asset_id} filename=\"{filename}\" status=unavailable source_codec={}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string())
                        ),
                        Some(asset_id),
                    );
                }
                Err(error) => {
                    let mut status = worker_state.batch_viewer_transcode.lock();
                    status.failed += 1;
                    let _ = worker_state.db.insert_log(
                        "error",
                        "batch_viewer_transcode",
                        &format!(
                            "asset_id={asset_id} filename=\"{filename}\" status=failed source_codec={} error={error}",
                            codec.clone().unwrap_or_else(|| "unknown".to_string())
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
        status.current_source_bytes = None;
        status.current_output_bytes = None;
        status.message = Some(format!(
            "Finished {} videos with {} failures",
            status.completed,
            status.failed
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
    let progress = refresh_takeout_index(&state, request).map_err(map_error)?;
    Ok(progress)
}

#[tauri::command]
pub fn start_refresh_index(request: RefreshRequest, state: State<AppState>) -> CommandResult<()> {
    if matches!(
        state
            .import_status
            .lock()
            .as_ref()
            .map(|item| item.status.as_str()),
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
    state: State<AppState>,
) -> CommandResult<Option<String>> {
    let key = format!("{asset_id}:{size}");
    let use_preview_cache = size >= VIEWER_PREVIEW_SIZE;
    let cache = if use_preview_cache {
        &state.preview_cache
    } else {
        &state.thumbnail_cache
    };
    if let Some(bytes) = cache.lock().get(&key) {
        return Ok(Some(format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(bytes)
        )));
    }

    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path.clone() else {
        return Ok(None);
    };
    let (filename, file_size) = media_debug_info(&primary_path);
    let started = Instant::now();
    let kind = thumb_log_kind(size);
    let generator = thumbnail_generator_label(&PathBuf::from(&primary_path));
    record_thumb_log(
        state.inner(),
        "info",
        asset_id,
        format!(
            "kind={kind} generator={generator} status=start mode=direct size={size}px filename=\"{filename}\" file_size={}",
            human_size(file_size)
        ),
    )?;

    let working_dir = state.app_data_dir.join("working");
    match generate_thumbnail(&PathBuf::from(primary_path), size, &working_dir) {
        Ok(Some(bytes)) => {
            record_thumb_log(
                state.inner(),
                "info",
                asset_id,
                format!(
                    "kind={kind} generator={generator} status=success mode=direct size={size}px filename=\"{filename}\" file_size={} generated_size={} elapsed={}",
                    human_size(file_size),
                    human_size(bytes.len() as u64),
                    human_elapsed_ms(started.elapsed().as_millis())
                ),
            )?;
            cache.lock().insert(key, bytes.clone());
            Ok(Some(format!(
                "data:image/jpeg;base64,{}",
                STANDARD.encode(bytes)
            )))
        }
        Ok(None) => {
            record_thumb_log(
                state.inner(),
                "warning",
                asset_id,
                format!(
                    "kind={kind} generator={generator} status=unavailable mode=direct size={size}px filename=\"{filename}\" file_size={} elapsed={}",
                    human_size(file_size),
                    human_elapsed_ms(started.elapsed().as_millis())
                ),
            )?;
            Ok(None)
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
                    "kind={kind} generator={generator} status=failed mode=direct size={size}px filename=\"{filename}\" file_size={} elapsed={} error={error}",
                    human_size(file_size),
                    human_elapsed_ms(started.elapsed().as_millis())
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
    state: State<AppState>,
) -> CommandResult<Vec<ThumbnailBatchItem>> {
    let generation = state.thumbnail_generation.load(Ordering::SeqCst);
    let use_preview_cache = size >= VIEWER_PREVIEW_SIZE;
    let cache = if use_preview_cache {
        state.preview_cache.clone()
    } else {
        state.thumbnail_cache.clone()
    };

    let items = asset_ids
        .into_iter()
        .map(|asset_id| {
            let key = format!("{asset_id}:{size}");

            if let Some(bytes) = cache.lock().get(&key) {
                preview_debug_log(format!(
                    "thumbnail_batch_item asset_id={} size={} status=ready source=cache bytes={}",
                    asset_id,
                    size,
                    bytes.len()
                ));
                return ThumbnailBatchItem {
                    asset_id,
                    status: "ready".to_string(),
                    data_url: Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))),
                };
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

    Ok(items)
}

#[tauri::command]
pub fn load_viewer_frame(
    asset_id: i64,
    prefer_original: Option<bool>,
    state: State<AppState>,
) -> CommandResult<Option<String>> {
    let started = Instant::now();
    let detail = query_service::get_asset_detail(&state.db, asset_id).map_err(map_error)?;
    let Some(primary_path) = detail.primary_path else {
        return Ok(None);
    };
    let (filename, file_size) = media_debug_info(&primary_path);
    let source_path = PathBuf::from(&primary_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original {
        if image_requires_backend_orientation(&source_path) {
            let working_dir = state.app_data_dir.join("working");
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

    let viewer_cache_dir = state.app_data_dir.join("viewer-cache");
    let working_dir = state.app_data_dir.join("working");
    match generate_viewer_image_file(
        &source_path,
        2400,
        &viewer_cache_dir,
        &working_dir,
    ) {
        Ok(Some(path)) => {
            let elapsed = started.elapsed().as_millis();
            let bytes = fs::read(&path).map_err(map_error)?;
            let generated_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            preview_debug_log(format!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} generated_bytes={} elapsed_ms={elapsed}",
                file_size,
                generated_bytes
            ));
            Ok(Some(format!(
                "data:image/jpeg;base64,{}",
                STANDARD.encode(bytes)
            )))
        }
        Ok(None) => {
            let elapsed = started.elapsed().as_millis();
            preview_debug_log(format!(
                "viewer asset_id={asset_id} filename=\"{filename}\" file_size={} unavailable elapsed_ms={elapsed}",
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
        return Ok(unavailable_viewer_media_status("Video playback unavailable", None, None));
    };
    let (filename, file_size) = media_debug_info(&primary_path);
    let source_path = PathBuf::from(&primary_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original && can_stream_original_video_bytes(&source_path) {
        let codec = probe_primary_video_codec(&source_path).map_err(map_error)?;
        let bytes = fs::read(&source_path).map_err(map_error)?;
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
            format!(
                "data:{};base64,{}",
                video_mime_type(&source_path),
                STANDARD.encode(bytes)
            ),
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
        return Ok(unavailable_viewer_media_status("Live photo playback unavailable", None, None));
    };
    let (filename, file_size) = media_debug_info(&motion_path);
    let source_path = PathBuf::from(&motion_path);
    let prefer_original = prefer_original.unwrap_or(false);

    if prefer_original && can_stream_original_video_bytes(&source_path) {
        let codec = probe_primary_video_codec(&source_path).map_err(map_error)?;
        let bytes = fs::read(&source_path).map_err(map_error)?;
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
            format!(
                "data:{};base64,{}",
                video_mime_type(&source_path),
                STANDARD.encode(bytes)
            ),
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
        viewer_render_cache_stats(&state.app_data_dir.join("viewer-cache")).map_err(map_error)?;
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
    state
        .db
        .insert_log("info", "thumbnail", "cleared thumbnail and preview caches", None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn clear_viewer_render_cache_command(state: State<AppState>) -> CommandResult<()> {
    clear_viewer_render_cache(&state.app_data_dir.join("viewer-cache")).map_err(map_error)?;
    state.viewer_video_jobs.lock().clear();
    state
        .db
        .insert_log("info", "viewer", "cleared viewer render cache", None)
        .map_err(map_error)?;
    Ok(())
}

#[tauri::command]
pub fn get_recent_logs(limit: Option<u32>, state: State<AppState>) -> CommandResult<Vec<LogEntry>> {
    query_service::get_recent_logs(&state.db, limit.unwrap_or(300)).map_err(map_error)
}

#[tauri::command]
pub fn get_thumb_generation_logs(
    limit: Option<u32>,
    state: State<AppState>,
) -> CommandResult<Vec<LogEntry>> {
    query_service::get_logs_by_scope(&state.db, &["thumb_gen"], limit.unwrap_or(400))
        .map_err(map_error)
}

#[tauri::command]
pub fn clear_thumb_generation_logs(state: State<AppState>) -> CommandResult<()> {
    state.db.clear_logs_by_scope(&["thumb_gen"]).map_err(map_error)
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
    state.thumbnail_generation.fetch_add(1, Ordering::SeqCst);
    state.thumbnail_cache.lock().clear();
    state.preview_cache.lock().clear();
    state.inflight_thumbnails.lock().clear();
    state.failed_thumbnails.lock().clear();
    state.viewer_video_jobs.lock().clear();
    *state.batch_viewer_transcode.lock() = BatchViewerTranscodeState::idle();
    clear_viewer_render_cache(&state.app_data_dir.join("viewer-cache")).map_err(map_error)?;
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
        clear_thumb_generation_logs,
        record_client_log,
        reset_local_database,
        clear_diagnostics,
        clear_logs
    ]
}
