use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{Arc, mpsc::Sender};

use parking_lot::Mutex;
use serde_json;

use crate::{
    cache::thumb_cache::ThumbnailCache,
    db::Database,
    models::{AppSettings, ImportProgress},
    util::errors::AppError,
};

pub const DEFAULT_VIEWER_PREVIEW_SIZE: u32 = 1000;

#[derive(Clone)]
pub struct ThumbnailJob {
    pub asset_id: i64,
    pub size: u32,
    pub key: String,
    pub generation: u64,
    pub use_preview_cache: bool,
}

#[derive(Clone)]
pub enum ViewerTranscodeState {
    Pending {
        started_at: std::time::Instant,
        codec: Option<String>,
        encoder: Option<String>,
        timeout_ms: u64,
        source_bytes: u64,
        temp_path: PathBuf,
    },
    Ready {
        path: PathBuf,
        codec: Option<String>,
        encoder: Option<String>,
    },
    Unavailable {
        codec: Option<String>,
        encoder: Option<String>,
        source_bytes: u64,
        output_bytes: u64,
    },
    Failed {
        message: String,
        codec: Option<String>,
        encoder: Option<String>,
        source_bytes: u64,
        output_bytes: u64,
    },
}

#[derive(Clone)]
pub struct BatchViewerTranscodeState {
    pub running: bool,
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub stop_requested: bool,
    pub current_asset_id: Option<i64>,
    pub current_filename: Option<String>,
    pub current_codec: Option<String>,
    pub current_width: Option<u32>,
    pub current_height: Option<u32>,
    pub current_duration_ms: Option<u64>,
    pub current_source_bytes: Option<u64>,
    pub current_output_bytes: Option<u64>,
    pub current_started_at: Option<std::time::Instant>,
    pub current_output_path: Option<PathBuf>,
    pub started_at: Option<std::time::Instant>,
    pub elapsed_ms: Option<u64>,
    pub message: Option<String>,
}

impl BatchViewerTranscodeState {
    pub fn idle() -> Self {
        Self {
            running: false,
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
            started_at: None,
            elapsed_ms: None,
            message: None,
        }
    }
}

#[derive(Clone)]
pub struct BatchThumbnailGenerationState {
    pub running: bool,
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub stop_requested: bool,
    pub current_asset_id: Option<i64>,
    pub current_filename: Option<String>,
    pub current_source_bytes: Option<u64>,
    pub current_started_at: Option<std::time::Instant>,
    pub started_at: Option<std::time::Instant>,
    pub elapsed_ms: Option<u64>,
    pub message: Option<String>,
}

impl BatchThumbnailGenerationState {
    pub fn idle() -> Self {
        Self {
            running: false,
            total: 0,
            completed: 0,
            failed: 0,
            skipped: 0,
            stop_requested: false,
            current_asset_id: None,
            current_filename: None,
            current_source_bytes: None,
            current_started_at: None,
            started_at: None,
            elapsed_ms: None,
            message: None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub app_data_dir: Arc<PathBuf>,
    pub settings_path: Arc<PathBuf>,
    pub app_settings: Arc<Mutex<AppSettings>>,
    pub thumbnail_worker_count: usize,
    pub import_status: Arc<Mutex<Option<ImportProgress>>>,
    pub thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
    pub preview_cache: Arc<Mutex<ThumbnailCache>>,
    pub inflight_thumbnails: Arc<Mutex<HashSet<String>>>,
    pub failed_thumbnails: Arc<Mutex<HashSet<String>>>,
    pub thumbnail_generation: Arc<AtomicU64>,
    pub thumb_backlog: Arc<AtomicUsize>,
    pub thumbnail_job_sender: Sender<ThumbnailJob>,
    pub preview_job_sender: Sender<ThumbnailJob>,
    pub viewer_video_jobs: Arc<Mutex<HashMap<String, ViewerTranscodeState>>>,
    pub batch_viewer_transcode: Arc<Mutex<BatchViewerTranscodeState>>,
    pub batch_thumbnail_generation: Arc<Mutex<BatchThumbnailGenerationState>>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            viewer_preview_size: DEFAULT_VIEWER_PREVIEW_SIZE,
        }
    }
}

impl AppSettings {
    pub fn sanitized(self) -> Self {
        Self {
            viewer_preview_size: self.viewer_preview_size.clamp(512, 4096),
        }
    }
}

pub fn app_settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

pub fn load_app_settings(settings_path: &Path) -> Result<AppSettings, AppError> {
    match fs::read_to_string(settings_path) {
        Ok(raw) => Ok(serde_json::from_str::<AppSettings>(&raw)?.sanitized()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn persist_app_settings(settings_path: &Path, settings: &AppSettings) -> Result<(), AppError> {
    fs::write(settings_path, serde_json::to_vec_pretty(settings)?)?;
    Ok(())
}

impl AppState {
    pub fn viewer_preview_size(&self) -> u32 {
        self.app_settings.lock().viewer_preview_size
    }

    pub fn app_settings_snapshot(&self) -> AppSettings {
        self.app_settings.lock().clone()
    }

    pub fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings, AppError> {
        let next = settings.sanitized();
        persist_app_settings(&self.settings_path, &next)?;
        *self.app_settings.lock() = next.clone();
        Ok(next)
    }
}
