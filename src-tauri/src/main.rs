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

use std::collections::HashSet;
use std::sync::mpsc;
use std::{fs, path::PathBuf, sync::Arc, thread};

use app::{
    commands::command_handlers,
    state::{AppState, ThumbnailJob},
};
use cache::thumb_cache::ThumbnailCache;
use db::{Database, DatabaseQueries};
use media::thumb::generate_thumbnail;
use parking_lot::Mutex;
use search::query_service;
use tauri::Manager;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

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
            let database = Database::new(&db_path)?;
            let (thumbnail_job_sender, thumbnail_job_receiver) = mpsc::channel::<ThumbnailJob>();
            let thumbnail_job_receiver = Arc::new(Mutex::new(thumbnail_job_receiver));
            let thumbnail_cache = Arc::new(Mutex::new(ThumbnailCache::new(256 * 1024 * 1024)));
            let inflight_thumbnails = Arc::new(Mutex::new(HashSet::new()));
            let failed_thumbnails = Arc::new(Mutex::new(HashSet::new()));
            let worker_count = std::thread::available_parallelism()
                .map(|count| count.get().min(4))
                .unwrap_or(4)
                .max(1);

            for worker_index in 0..worker_count {
                let receiver = thumbnail_job_receiver.clone();
                let db = Arc::new(Database::new(&db_path)?);
                let cache = thumbnail_cache.clone();
                let inflight = inflight_thumbnails.clone();
                let failed = failed_thumbnails.clone();
                thread::spawn(move || loop {
                    let job = {
                        let receiver = receiver.lock();
                        match receiver.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        }
                    };

                    let result = (|| -> Result<Option<Vec<u8>>, String> {
                        let detail =
                            query_service::get_asset_detail(&db, job.asset_id).map_err(|error| error.to_string())?;
                        let Some(primary_path) = detail.primary_path else {
                            return Ok(None);
                        };
                        let filename = std::path::PathBuf::from(&primary_path)
                            .file_name()
                            .and_then(|item| item.to_str())
                            .unwrap_or(&primary_path)
                            .to_string();
                        let file_size = fs::metadata(&primary_path).map(|meta| meta.len()).unwrap_or(0);
                        let started = std::time::Instant::now();
                        println!(
                            "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=start size={}",
                            worker_index,
                            job.asset_id,
                            filename,
                            file_size,
                            job.size
                        );
                        let generated =
                            generate_thumbnail(&std::path::PathBuf::from(primary_path), job.size)
                                .map_err(|error| error.to_string())?;
                        match &generated {
                            Some(bytes) => println!(
                                "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=success generated_bytes={} elapsed_ms={}",
                                worker_index,
                                job.asset_id,
                                filename,
                                file_size,
                                bytes.len(),
                                started.elapsed().as_millis()
                            ),
                            None => println!(
                                "thumbnail_worker={} asset_id={} filename=\"{}\" file_size={} status=unavailable elapsed_ms={}",
                                worker_index,
                                job.asset_id,
                                filename,
                                file_size,
                                started.elapsed().as_millis()
                            ),
                        }
                        Ok(generated)
                    })();

                    match result {
                        Ok(Some(bytes)) => {
                            cache.lock().insert(job.key.clone(), bytes);
                            failed.lock().remove(&job.key);
                        }
                        Ok(None) => {
                            failed.lock().insert(job.key.clone());
                        }
                        Err(error) => {
                            println!(
                                "thumbnail_worker={} asset_id={} status=failed error={error}",
                                worker_index, job.asset_id
                            );
                            let _ = db.insert_log("error", "thumbnail_worker", &error, Some(job.asset_id));
                            failed.lock().insert(job.key.clone());
                        }
                    }

                    inflight.lock().remove(&job.key);
                });
            }

            let state = AppState {
                db: Arc::new(database),
                import_status: Arc::new(Mutex::new(None)),
                thumbnail_cache,
                inflight_thumbnails,
                failed_thumbnails,
                thumbnail_job_sender,
                thumbnail_worker_count: worker_count,
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
