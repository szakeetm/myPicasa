use std::collections::HashSet;
use std::sync::{Arc, mpsc::Sender};

use parking_lot::Mutex;

use crate::{cache::thumb_cache::ThumbnailCache, db::Database, models::ImportProgress};

#[derive(Clone)]
pub struct ThumbnailJob {
    pub asset_id: i64,
    pub size: u32,
    pub key: String,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub import_status: Arc<Mutex<Option<ImportProgress>>>,
    pub thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
    pub inflight_thumbnails: Arc<Mutex<HashSet<String>>>,
    pub failed_thumbnails: Arc<Mutex<HashSet<String>>>,
    pub thumbnail_job_sender: Sender<ThumbnailJob>,
    pub thumbnail_worker_count: usize,
}
