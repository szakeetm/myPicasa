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
    Pending,
    Ready { path: PathBuf },
    Unavailable,
    Failed { message: String },
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
}
