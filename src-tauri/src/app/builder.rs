#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::{fs, path::PathBuf};

use parking_lot::Mutex;

use crate::app::state::{
    AppState, BatchThumbnailGenerationState, BatchViewerTranscodeState, ThumbnailJob,
    app_settings_path, load_app_settings, persist_app_settings, preview_cache_replacement_keys,
};
use crate::cache::thumb_cache::ThumbnailCache;
use crate::db::{Database, DatabaseQueries};
use crate::media::thumb::{generate_thumbnail, thumbnail_generator_label};
use crate::search::query_service;
use crate::util::errors::AppError;

const PREVIEW_DEBUG_LOGS: bool = false;

pub fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().min(4))
        .unwrap_or(4)
        .max(1)
}

pub fn build_app_state(
    app_data_dir: PathBuf,
    worker_count: Option<usize>,
) -> Result<AppState, AppError> {
    fs::create_dir_all(&app_data_dir)?;

    let db_path = app_data_dir.join("my_picasa.sqlite");
    let settings_path = app_settings_path(&app_data_dir);
    let working_dir = app_data_dir.join("working");
    let app_settings = load_app_settings(&settings_path)?;
    let cache_data_dir = app_settings
        .cache_storage_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data_dir.clone());
    let thumbnail_cache_dir = cache_data_dir.join("thumbnail-cache");
    let preview_cache_dir = cache_data_dir.join("preview-cache");
    let viewer_cache_dir = cache_data_dir.join("viewer-cache");

    fs::create_dir_all(&thumbnail_cache_dir)?;
    fs::create_dir_all(&preview_cache_dir)?;
    fs::create_dir_all(&viewer_cache_dir)?;
    fs::create_dir_all(&working_dir)?;
    persist_app_settings(&settings_path, &app_settings)?;

    let database = Database::new(&db_path)?;
    let (thumbnail_job_sender, thumbnail_job_receiver) = mpsc::channel::<ThumbnailJob>();
    let (preview_job_sender, preview_job_receiver) = mpsc::channel::<ThumbnailJob>();
    let thumbnail_job_receiver = Arc::new(Mutex::new(thumbnail_job_receiver));
    let preview_job_receiver = Arc::new(Mutex::new(preview_job_receiver));
    let thumbnail_cache = Arc::new(Mutex::new(ThumbnailCache::new(
        thumbnail_cache_dir,
        256 * 1024 * 1024,
    )));
    let preview_cache = Arc::new(Mutex::new(ThumbnailCache::new(
        preview_cache_dir,
        512 * 1024 * 1024,
    )));
    let inflight_thumbnails = Arc::new(Mutex::new(HashSet::new()));
    let failed_thumbnails = Arc::new(Mutex::new(HashSet::new()));
    let thumbnail_generation = Arc::new(AtomicU64::new(0));
    let worker_count = worker_count.unwrap_or_else(default_worker_count);
    let thumb_backlog = Arc::new(AtomicUsize::new(0));
    let active_thumb_workers = Arc::new(AtomicUsize::new(0));

    for worker_index in 0..worker_count {
        spawn_thumbnail_worker(
            worker_index,
            thumbnail_job_receiver.clone(),
            preview_job_receiver.clone(),
            db_path.clone(),
            thumbnail_cache.clone(),
            preview_cache.clone(),
            inflight_thumbnails.clone(),
            failed_thumbnails.clone(),
            thumbnail_generation.clone(),
            working_dir.clone(),
            thumb_backlog.clone(),
            active_thumb_workers.clone(),
        );
    }

    let state = AppState {
        db: Arc::new(database),
        app_data_dir: Arc::new(app_data_dir),
        cache_data_dir: Arc::new(Mutex::new(cache_data_dir)),
        settings_path: Arc::new(settings_path),
        app_settings: Arc::new(Mutex::new(app_settings)),
        thumbnail_worker_count: worker_count,
        import_status: Arc::new(Mutex::new(None)),
        refresh_cancel: Arc::new(AtomicBool::new(false)),
        thumbnail_cache,
        preview_cache,
        inflight_thumbnails,
        failed_thumbnails,
        thumbnail_generation,
        thumb_backlog,
        thumbnail_job_sender,
        preview_job_sender,
        viewer_video_jobs: Arc::new(Mutex::new(HashMap::new())),
        batch_viewer_transcode: Arc::new(Mutex::new(BatchViewerTranscodeState::idle())),
        batch_thumbnail_generation: Arc::new(Mutex::new(BatchThumbnailGenerationState::idle())),
        cache_storage_migration: Arc::new(Mutex::new(
            crate::models::CacheStorageMigrationStatus::idle(),
        )),
        cache_storage_migration_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    state
        .db
        .insert_log("info", "bootstrap", "backend initialized", None)?;

    Ok(state)
}

fn preview_debug_log(message: String) {
    if PREVIEW_DEBUG_LOGS {
        println!("{message}");
    }
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

fn thumb_log_kind(use_preview_cache: bool) -> &'static str {
    if use_preview_cache {
        "preview"
    } else {
        "thumb"
    }
}

fn process_thumbnail_job(
    job: ThumbnailJob,
    worker_index: usize,
    db_path: &PathBuf,
    thumbnail_cache: &Arc<Mutex<ThumbnailCache>>,
    preview_cache: &Arc<Mutex<ThumbnailCache>>,
    inflight: &Arc<Mutex<HashSet<String>>>,
    failed: &Arc<Mutex<HashSet<String>>>,
    generation: &Arc<AtomicU64>,
    working_dir: &PathBuf,
) {
    let result = (|| -> Result<Option<Vec<u8>>, String> {
        let db = Database::new(db_path).map_err(|error| error.to_string())?;
        let detail =
            query_service::get_asset_detail(&db, job.asset_id).map_err(|error| error.to_string())?;
        let Some(primary_path) = detail.primary_path else {
            return Ok(None);
        };
        let primary_path_buf = PathBuf::from(&primary_path);
        let filename = primary_path_buf
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or(&primary_path)
            .to_string();
        let file_size = fs::metadata(&primary_path).map(|meta| meta.len()).unwrap_or(0);
        let kind = thumb_log_kind(job.use_preview_cache);
        let generator = thumbnail_generator_label(&primary_path_buf);
        let _ = db.insert_log(
            "info",
            "thumb_gen",
            &format!(
                "kind={kind} generator={generator} status=start worker={} asset_id={} size={}px filename=\"{}\" file_size={}",
                worker_index,
                job.asset_id,
                job.size,
                filename,
                human_size(file_size),
            ),
            Some(job.asset_id),
        );
        preview_debug_log(format!(
            "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=start size={}",
            worker_index,
            job.asset_id,
            filename,
            file_size,
            job.size
        ));
        let generated =
            generate_thumbnail(&primary_path_buf, job.size, !job.use_preview_cache, working_dir)
                .map_err(|error| error.to_string())?;
        match &generated.bytes {
            Some(bytes) => {
                let _ = db.insert_log(
                    "info",
                    "thumb_gen",
                    &format!(
                        "kind={kind} generator={generator} status=success worker={} asset_id={} size={}px filename=\"{}\" file_size={} generated_size={}",
                        worker_index,
                        job.asset_id,
                        job.size,
                        filename,
                        human_size(file_size),
                        human_size(bytes.len() as u64),
                    ),
                    Some(job.asset_id),
                );
            }
            None => {
                let _ = db.insert_log(
                    "warning",
                    "thumb_gen",
                    &format!(
                        "kind={kind} generator={generator} status=unavailable worker={} asset_id={} size={}px filename=\"{}\" file_size={}",
                        worker_index,
                        job.asset_id,
                        job.size,
                        filename,
                        human_size(file_size),
                    ),
                    Some(job.asset_id),
                );
            }
        }
        Ok(generated.bytes)
    })();

    match result {
        Ok(Some(bytes)) => {
            if generation.load(Ordering::SeqCst) == job.generation {
                let cache = if job.use_preview_cache {
                    preview_cache
                } else {
                    thumbnail_cache
                };
                let mut cache = cache.lock();
                if job.use_preview_cache {
                    for replacement_key in preview_cache_replacement_keys(job.asset_id, job.size) {
                        cache.remove(&replacement_key);
                    }
                }
                cache.insert(job.key.clone(), bytes);
                failed.lock().remove(&job.key);
            }
        }
        Ok(None) => {
            if generation.load(Ordering::SeqCst) == job.generation {
                failed.lock().insert(job.key.clone());
            }
        }
        Err(error) => {
            if let Ok(db) = Database::new(db_path) {
                let _ = db.insert_log(
                    "error",
                    "thumb_gen",
                    &format!(
                        "kind={} status=failed worker={} asset_id={} size={}px error={error}",
                        thumb_log_kind(job.use_preview_cache),
                        worker_index,
                        job.asset_id,
                        job.size,
                    ),
                    Some(job.asset_id),
                );
            }
            if generation.load(Ordering::SeqCst) == job.generation {
                failed.lock().insert(job.key.clone());
            }
        }
    }

    inflight.lock().remove(&job.key);
}

fn spawn_thumbnail_worker(
    worker_index: usize,
    thumbnail_receiver: Arc<Mutex<Receiver<ThumbnailJob>>>,
    preview_receiver: Arc<Mutex<Receiver<ThumbnailJob>>>,
    db_path: PathBuf,
    thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
    preview_cache: Arc<Mutex<ThumbnailCache>>,
    inflight: Arc<Mutex<HashSet<String>>>,
    failed: Arc<Mutex<HashSet<String>>>,
    generation: Arc<AtomicU64>,
    working_dir: PathBuf,
    thumb_backlog: Arc<AtomicUsize>,
    active_thumb_workers: Arc<AtomicUsize>,
) {
    thread::spawn(move || loop {
        let thumb_job = {
            let receiver = thumbnail_receiver.lock();
            match receiver.recv_timeout(Duration::from_millis(40)) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };

        if let Some(job) = thumb_job {
            active_thumb_workers.fetch_add(1, Ordering::SeqCst);
            process_thumbnail_job(
                job,
                worker_index,
                &db_path,
                &thumbnail_cache,
                &preview_cache,
                &inflight,
                &failed,
                &generation,
                &working_dir,
            );
            active_thumb_workers.fetch_sub(1, Ordering::SeqCst);
            thumb_backlog.fetch_sub(1, Ordering::SeqCst);
            continue;
        }

        if thumb_backlog.load(Ordering::SeqCst) > 0
            || active_thumb_workers.load(Ordering::SeqCst) > 0
        {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        let job = {
            let receiver = preview_receiver.lock();
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };

        process_thumbnail_job(
            job,
            worker_index,
            &db_path,
            &thumbnail_cache,
            &preview_cache,
            &inflight,
            &failed,
            &generation,
            &working_dir,
        );
    });
}
