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

use std::{fs, path::PathBuf, sync::Arc};

use app::{commands::command_handlers, state::AppState};
use cache::thumb_cache::ThumbnailCache;
use db::{Database, DatabaseQueries};
use parking_lot::Mutex;
use tauri::Manager;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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
            let state = AppState {
                db: Arc::new(database),
                import_status: Arc::new(Mutex::new(None)),
                thumbnail_cache: Arc::new(Mutex::new(ThumbnailCache::new(256 * 1024 * 1024))),
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
