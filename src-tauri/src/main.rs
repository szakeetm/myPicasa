#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cache;
mod db;
mod hash;
mod import;
mod media;
mod models;
mod search;
mod util;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::{fs, path::PathBuf, sync::Arc, thread};
use std::time::Duration;

use app::{
    commands::command_handlers,
    state::{AppState, ThumbnailJob},
};
use cache::thumb_cache::ThumbnailCache;
use db::{Database, DatabaseQueries};
use media::thumb::{generate_thumbnail, thumbnail_generator_label};
use parking_lot::Mutex;
use search::query_service;
use tauri::Manager;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const PREVIEW_DEBUG_LOGS: bool = false;
const VIEWER_PREVIEW_SIZE: u32 = 1024;
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
        let primary_path_buf = std::path::PathBuf::from(&primary_path);
        let filename = primary_path_buf
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or(&primary_path)
            .to_string();
        let file_size = fs::metadata(&primary_path).map(|meta| meta.len()).unwrap_or(0);
        let started = std::time::Instant::now();
        let kind = thumb_log_kind(job.size);
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
                human_size(file_size)
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
            generate_thumbnail(&primary_path_buf, job.size, working_dir).map_err(|error| error.to_string())?;
        match &generated {
            Some(bytes) => {
                let elapsed_ms = started.elapsed().as_millis();
                let _ = db.insert_log(
                    "info",
                    "thumb_gen",
                    &format!(
                        "kind={kind} generator={generator} status=success worker={} asset_id={} size={}px filename=\"{}\" file_size={} generated_size={} elapsed={}",
                        worker_index,
                        job.asset_id,
                        job.size,
                        filename,
                        human_size(file_size),
                        human_size(bytes.len() as u64),
                        human_elapsed_ms(elapsed_ms),
                    ),
                    Some(job.asset_id),
                );
                preview_debug_log(format!(
                    "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=success generated_bytes={} elapsed_ms={elapsed_ms}",
                    worker_index,
                    job.asset_id,
                    filename,
                    file_size,
                    bytes.len(),
                ));
            }
            None => {
                let elapsed_ms = started.elapsed().as_millis();
                let _ = db.insert_log(
                    "warning",
                    "thumb_gen",
                    &format!(
                        "kind={kind} generator={generator} status=unavailable worker={} asset_id={} size={}px filename=\"{}\" file_size={} elapsed={}",
                        worker_index,
                        job.asset_id,
                        job.size,
                        filename,
                        human_size(file_size),
                        human_elapsed_ms(elapsed_ms),
                    ),
                    Some(job.asset_id),
                );
                preview_debug_log(format!(
                    "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=unavailable elapsed_ms={elapsed_ms}",
                    worker_index,
                    job.asset_id,
                    filename,
                    file_size,
                ));
            }
        }
        Ok(generated)
    })();

    match result {
        Ok(Some(bytes)) => {
            if generation.load(Ordering::SeqCst) == job.generation {
                let cache = if job.size >= VIEWER_PREVIEW_SIZE {
                    preview_cache
                } else {
                    thumbnail_cache
                };
                cache.lock().insert(job.key.clone(), bytes);
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
                        thumb_log_kind(job.size),
                        worker_index,
                        job.asset_id,
                        job.size,
                    ),
                    Some(job.asset_id),
                );
                let _ = db.insert_log("error", "thumbnail_worker", &error, Some(job.asset_id));
            }
            preview_debug_log(format!(
                "thumbnail_worker={} asset_id={} status=failed error={error}",
                worker_index, job.asset_id
            ));
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

        if thumb_backlog.load(Ordering::SeqCst) > 0 || active_thumb_workers.load(Ordering::SeqCst) > 0 {
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

fn main() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("my_picasa=debug,tauri=info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from(".myPicasa"));

            fs::create_dir_all(&app_data_dir)?;

            let db_path = app_data_dir.join("my_picasa.sqlite");
            let thumbnail_cache_dir = app_data_dir.join("thumbnail-cache");
            let preview_cache_dir = app_data_dir.join("preview-cache");
            let viewer_cache_dir = app_data_dir.join("viewer-cache");
            let working_dir = app_data_dir.join("working");
            fs::create_dir_all(&preview_cache_dir)?;
            fs::create_dir_all(&viewer_cache_dir)?;
            fs::create_dir_all(&working_dir)?;
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
            let worker_count = std::thread::available_parallelism()
                .map(|count| count.get().min(4))
                .unwrap_or(4)
                .max(1);
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
                import_status: Arc::new(Mutex::new(None)),
                thumbnail_cache,
                preview_cache,
                inflight_thumbnails,
                failed_thumbnails,
                thumbnail_generation,
                thumb_backlog,
                thumbnail_job_sender,
                preview_job_sender,
                viewer_video_jobs: Arc::new(Mutex::new(HashMap::new())),
            };

            state
                .db
                .insert_log("info", "bootstrap", "backend initialized", None)?;

            app.manage(state);
            Ok(())
        })
        .invoke_handler(command_handlers())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
