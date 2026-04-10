use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, mpsc::Sender};

use parking_lot::Mutex;
use serde_json;

use crate::{
    cache::thumb_cache::ThumbnailCache,
    db::{Database, DatabaseQueries},
    models::{AppSettings, CacheStorageMigrationStatus, ImportProgress},
    util::errors::AppError,
};

pub const DEFAULT_VIEWER_PREVIEW_SIZE: u32 = 1000;
const VIEWER_PREVIEW_SIZE_OPTIONS: [u32; 4] = [1000, 1280, 1600, 2048];

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
    pub cache_data_dir: Arc<Mutex<PathBuf>>,
    pub settings_path: Arc<PathBuf>,
    pub app_settings: Arc<Mutex<AppSettings>>,
    pub thumbnail_worker_count: usize,
    pub import_status: Arc<Mutex<Option<ImportProgress>>>,
    pub refresh_cancel: Arc<AtomicBool>,
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
    pub cache_storage_migration: Arc<Mutex<CacheStorageMigrationStatus>>,
    pub cache_storage_migration_cancel: Arc<AtomicBool>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            viewer_preview_size: DEFAULT_VIEWER_PREVIEW_SIZE,
            cache_storage_dir: None,
            indexed_roots: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn sanitized(self) -> Self {
        Self {
            viewer_preview_size: self.viewer_preview_size.clamp(512, 4096),
            cache_storage_dir: self
                .cache_storage_dir
                .and_then(|value| {
                    let trimmed = value.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                }),
            indexed_roots: self
                .indexed_roots
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
        }
    }
}

impl CacheStorageMigrationStatus {
    pub fn idle() -> Self {
        Self {
            status: "idle".to_string(),
            running: false,
            stop_requested: false,
            copy_existing: false,
            source_dir: None,
            destination_dir: None,
            total_files: 0,
            copied_files: 0,
            total_bytes: 0,
            copied_bytes: 0,
            current_path: None,
            message: None,
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

pub fn preview_cache_replacement_keys(asset_id: i64, keep_size: u32) -> Vec<String> {
    VIEWER_PREVIEW_SIZE_OPTIONS
        .into_iter()
        .filter(|size| *size != keep_size)
        .map(|size| format!("pv2:{asset_id}:{size}"))
        .collect()
}

impl AppState {
    pub fn viewer_preview_size(&self) -> u32 {
        self.app_settings.lock().viewer_preview_size
    }

    pub fn default_cache_data_dir(&self) -> PathBuf {
        (*self.app_data_dir).clone()
    }

    pub fn resolve_cache_data_dir(&self, settings: &AppSettings) -> PathBuf {
        settings
            .cache_storage_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_cache_data_dir())
    }

    pub fn cache_data_dir(&self) -> PathBuf {
        self.cache_data_dir.lock().clone()
    }

    pub fn viewer_cache_dir(&self) -> PathBuf {
        self.cache_data_dir().join("viewer-cache")
    }

    pub fn working_dir(&self) -> PathBuf {
        self.app_data_dir.join("working")
    }

    pub fn app_settings_snapshot(&self) -> AppSettings {
        self.app_settings.lock().clone()
    }

    pub fn switch_cache_data_dir(
        &self,
        cache_data_dir: PathBuf,
        copied_existing_assets: bool,
    ) -> Result<(), AppError> {
        fs::create_dir_all(&cache_data_dir)?;
        fs::create_dir_all(cache_data_dir.join("thumbnail-cache"))?;
        fs::create_dir_all(cache_data_dir.join("preview-cache"))?;
        fs::create_dir_all(cache_data_dir.join("viewer-cache"))?;

        self.thumbnail_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inflight_thumbnails.lock().clear();
        self.failed_thumbnails.lock().clear();
        *self.thumbnail_cache.lock() =
            ThumbnailCache::new(cache_data_dir.join("thumbnail-cache"), 256 * 1024 * 1024);
        *self.preview_cache.lock() =
            ThumbnailCache::new(cache_data_dir.join("preview-cache"), 512 * 1024 * 1024);
        self.viewer_video_jobs.lock().clear();
        if !copied_existing_assets {
            self.db.clear_viewer_video_transcode_statuses()?;
        }
        *self.cache_data_dir.lock() = cache_data_dir;
        Ok(())
    }

    pub fn apply_app_settings(
        &self,
        settings: AppSettings,
        copied_existing_assets: bool,
    ) -> Result<AppSettings, AppError> {
        let next = settings.sanitized();
        let previous = self.app_settings_snapshot();
        let previous_cache_dir = self.resolve_cache_data_dir(&previous);
        let next_cache_dir = self.resolve_cache_data_dir(&next);
        if previous_cache_dir != next_cache_dir {
            self.switch_cache_data_dir(next_cache_dir, copied_existing_assets)?;
        }
        persist_app_settings(&self.settings_path, &next)?;
        *self.app_settings.lock() = next.clone();
        Ok(next)
    }

    pub fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings, AppError> {
        self.apply_app_settings(settings, false)
    }
}
