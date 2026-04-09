use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{Arc, mpsc::Sender};

use parking_lot::Mutex;

use crate::{cache::thumb_cache::ThumbnailCache, db::Database, models::ImportProgress};

#[derive(Clone)]
pub struct ThumbnailJob {
    pub asset_id: i64,
    pub size: u32,
    pub key: String,
    pub generation: u64,
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
    pub current_asset_id: Option<i64>,
    pub current_filename: Option<String>,
    pub current_source_bytes: Option<u64>,
    pub current_output_bytes: Option<u64>,
    pub started_at: Option<std::time::Instant>,
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
            current_asset_id: None,
            current_filename: None,
            current_source_bytes: None,
            current_output_bytes: None,
            started_at: None,
            message: None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub app_data_dir: Arc<PathBuf>,
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
}
