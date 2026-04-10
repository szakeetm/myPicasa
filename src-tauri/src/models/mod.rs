use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub viewer_preview_size: u32,
    pub cache_storage_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStorageMigrationStatus {
    pub status: String,
    pub running: bool,
    pub stop_requested: bool,
    pub copy_existing: bool,
    pub source_dir: Option<String>,
    pub destination_dir: Option<String>,
    pub total_files: u64,
    pub copied_files: u64,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub current_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportProgress {
    pub import_id: i64,
    pub status: String,
    pub phase: String,
    pub files_scanned: u32,
    pub processed_files: u32,
    pub total_files: u32,
    pub files_added: u32,
    pub files_updated: u32,
    pub files_deleted: u32,
    pub assets_added: u32,
    pub assets_updated: u32,
    pub assets_deleted: u32,
    pub worker_count: u32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileScanRecord {
    pub path: String,
    pub root_path: String,
    pub parent_path: String,
    pub filename: String,
    pub extension: Option<String>,
    pub detected_format: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: i64,
    pub mtime_utc: String,
    pub ctime_utc: Option<String>,
    pub candidate_type: String,
    pub json_kind: Option<String>,
    pub quick_hash: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSidecar {
    pub json_raw: String,
    pub title_hint: Option<String>,
    pub photo_taken_time_utc: Option<String>,
    pub geo_lat: Option<f64>,
    pub geo_lon: Option<f64>,
    pub geo_alt: Option<f64>,
    pub people_json: Option<String>,
    pub google_photos_origin: Option<String>,
    pub google_photos_url: Option<String>,
    pub json_kind: String,
    pub guessed_target_stem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub id: i64,
    pub name: String,
    pub source_path: String,
    pub asset_count: u32,
    pub begin_taken_at_utc: Option<String>,
    pub end_taken_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListRequest {
    pub cursor: Option<u32>,
    pub limit: Option<u32>,
    pub query: Option<String>,
    pub media_kind: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListResponse {
    pub items: Vec<AssetListItem>,
    pub next_cursor: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetListItem {
    pub id: i64,
    pub title: Option<String>,
    pub media_kind: String,
    pub taken_at_utc: Option<String>,
    pub duration_ms: Option<i64>,
    pub has_live_photo: bool,
    pub primary_path: String,
    pub albums: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDetail {
    pub id: i64,
    pub title: Option<String>,
    pub media_kind: String,
    pub display_type: String,
    pub taken_at_utc: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub primary_path: Option<String>,
    pub albums: Vec<String>,
    pub live_photo_video_path: Option<String>,
    pub google_photos_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerPlaybackHint {
    pub asset_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEntry {
    pub id: i64,
    pub import_id: i64,
    pub severity: String,
    pub diagnostic_type: String,
    pub related_path: Option<String>,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub thumbnail_items: u32,
    pub thumbnail_bytes: u64,
    pub thumbnail_budget_bytes: u64,
    pub preview_items: u32,
    pub preview_bytes: u64,
    pub preview_budget_bytes: u64,
    pub viewer_render_items: u32,
    pub viewer_render_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: i64,
    pub created_at: String,
    pub level: String,
    pub scope: String,
    pub message: String,
    pub asset_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailBatchItem {
    pub asset_id: i64,
    pub status: String,
    pub data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerMediaStatus {
    pub status: String,
    pub src: Option<String>,
    pub source: Option<String>,
    pub message: Option<String>,
    pub codec: Option<String>,
    pub encoder: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub source_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchViewerTranscodeStatus {
    pub status: String,
    pub total: u32,
    pub completed: u32,
    pub succeeded: u32,
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
    pub current_elapsed_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchThumbnailGenerationStatus {
    pub status: String,
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub stop_requested: bool,
    pub current_asset_id: Option<i64>,
    pub current_filename: Option<String>,
    pub current_source_bytes: Option<u64>,
    pub current_elapsed_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerPlaybackSupport {
    pub mp4_h264: bool,
    pub mp4_hevc: bool,
    pub mov_h264: bool,
    pub mov_hevc: bool,
    pub webm: bool,
}
