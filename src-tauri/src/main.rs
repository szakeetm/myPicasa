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

use std::path::PathBuf;

use app::{builder::build_app_state, commands::command_handlers};
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

            let state = build_app_state(app_data_dir, None)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(command_handlers())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
