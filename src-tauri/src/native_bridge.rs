use std::sync::Arc;
use std::thread;

use crate::app::builder::build_app_state;
use crate::app::state::{AppState, BatchThumbnailGenerationState, BatchViewerTranscodeState};
use crate::db::DatabaseQueries;
use crate::import::refresher::refresh_takeout_index;
use crate::media::thumb::{clear_viewer_render_cache, viewer_render_cache_stats};
use crate::models::{AssetListRequest, ImportProgress, RefreshRequest};
use crate::search::query_service;
use crate::util::errors::AppError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum NativeBridgeError {
    #[error("{reason}")]
    Message { reason: String },
}

impl From<AppError> for NativeBridgeError {
    fn from(value: AppError) -> Self {
        Self::Message {
            reason: value.to_string(),
        }
    }
}

impl From<serde_json::Error> for NativeBridgeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Message {
            reason: value.to_string(),
        }
    }
}

type BridgeResult<T> = Result<T, NativeBridgeError>;

#[derive(uniffi::Object)]
pub struct NativeAppBridge {
    state: AppState,
}

#[uniffi::export]
impl NativeAppBridge {
    #[uniffi::constructor]
    pub fn new(app_data_dir: String) -> BridgeResult<Arc<Self>> {
        Ok(Arc::new(Self {
            state: build_app_state(app_data_dir.into(), None)?,
        }))
    }

    pub fn list_albums_json(&self) -> BridgeResult<String> {
        Ok(serde_json::to_string(&query_service::list_albums(&self.state.db)?)?)
    }

    pub fn list_assets_by_date_json(&self, request_json: String) -> BridgeResult<String> {
        let request: AssetListRequest = serde_json::from_str(&request_json)?;
        Ok(serde_json::to_string(&query_service::list_assets_by_date(
            &self.state.db,
            request,
        )?)?)
    }

    pub fn list_assets_by_album_json(
        &self,
        album_id: i64,
        request_json: String,
    ) -> BridgeResult<String> {
        let request: AssetListRequest = serde_json::from_str(&request_json)?;
        Ok(serde_json::to_string(&query_service::list_assets_by_album(
            &self.state.db,
            album_id,
            request,
        )?)?)
    }

    pub fn search_assets_json(&self, request_json: String) -> BridgeResult<String> {
        let request: AssetListRequest = serde_json::from_str(&request_json)?;
        Ok(serde_json::to_string(&query_service::search_assets(
            &self.state.db,
            request,
        )?)?)
    }

    pub fn get_asset_detail_json(&self, asset_id: i64) -> BridgeResult<String> {
        Ok(serde_json::to_string(&query_service::get_asset_detail(
            &self.state.db,
            asset_id,
        )?)?)
    }

    pub fn get_recent_logs_json(&self, limit: u32) -> BridgeResult<String> {
        Ok(serde_json::to_string(&query_service::get_recent_logs(
            &self.state.db,
            limit,
        )?)?)
    }

    pub fn get_ingress_diagnostics_json(&self) -> BridgeResult<String> {
        Ok(serde_json::to_string(&query_service::get_ingress_diagnostics(
            &self.state.db,
        )?)?)
    }

    pub fn get_import_status_json(&self) -> BridgeResult<String> {
        Ok(serde_json::to_string(&self.state.import_status.lock().clone())?)
    }

    pub fn get_cache_stats_json(&self) -> BridgeResult<String> {
        let mut stats = self.state.thumbnail_cache.lock().stats();
        let preview_stats = self.state.preview_cache.lock().stats();
        stats.preview_items = preview_stats.thumbnail_items;
        stats.preview_bytes = preview_stats.thumbnail_bytes;
        stats.preview_budget_bytes = preview_stats.thumbnail_budget_bytes;
        let (viewer_render_items, viewer_render_bytes) =
            viewer_render_cache_stats(&self.state.viewer_cache_dir())?;
        stats.viewer_render_items = viewer_render_items;
        stats.viewer_render_bytes = viewer_render_bytes;
        Ok(serde_json::to_string(&stats)?)
    }

    pub fn start_refresh_index(&self, roots: Vec<String>) -> BridgeResult<()> {
        if matches!(
            self.state
                .import_status
                .lock()
                .as_ref()
                .map(|item| item.status.as_str()),
            Some("running")
        ) {
            return Err(NativeBridgeError::Message {
                reason: "an import is already running".to_string(),
            });
        }

        let state = self.state.clone();
        thread::spawn(move || {
            if let Err(error) = refresh_takeout_index(&state, RefreshRequest { roots }) {
                let message = error.to_string();
                *state.import_status.lock() = Some(ImportProgress {
                    import_id: 0,
                    status: "failed".to_string(),
                    phase: "failed".to_string(),
                    files_scanned: 0,
                    processed_files: 0,
                    total_files: 0,
                    files_added: 0,
                    files_updated: 0,
                    files_deleted: 0,
                    assets_added: 0,
                    assets_updated: 0,
                    assets_deleted: 0,
                    worker_count: 0,
                    message: Some(message.clone()),
                });
                let _ = state.db.insert_log(
                    "error",
                    "import",
                    &format!("background refresh failed: {message}"),
                    None,
                );
            }
        });

        Ok(())
    }

    pub fn clear_thumbnail_cache(&self) -> BridgeResult<()> {
        self.state.thumbnail_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.state.thumbnail_cache.lock().clear();
        self.state.preview_cache.lock().clear();
        self.state.inflight_thumbnails.lock().clear();
        self.state.failed_thumbnails.lock().clear();
        self.state.viewer_video_jobs.lock().clear();
        *self.state.batch_thumbnail_generation.lock() = BatchThumbnailGenerationState::idle();
        self.state
            .db
            .insert_log("info", "thumbnail", "cleared thumbnail and preview caches", None)?;
        Ok(())
    }

    pub fn clear_viewer_render_cache(&self) -> BridgeResult<()> {
        clear_viewer_render_cache(&self.state.viewer_cache_dir())?;
        self.state.viewer_video_jobs.lock().clear();
        self.state
            .db
            .insert_log("info", "viewer", "cleared viewer render cache", None)?;
        Ok(())
    }

    pub fn clear_diagnostics(&self) -> BridgeResult<()> {
        self.state.db.clear_diagnostics()?;
        self.state
            .db
            .insert_log("info", "debug", "cleared ingress diagnostics", None)?;
        Ok(())
    }

    pub fn clear_logs(&self) -> BridgeResult<()> {
        self.state.db.clear_logs()?;
        Ok(())
    }

    pub fn reset_local_database(&self) -> BridgeResult<()> {
        self.state.db.reset()?;
        *self.state.import_status.lock() = None;
        self.state.thumbnail_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.state.thumbnail_cache.lock().clear();
        self.state.preview_cache.lock().clear();
        self.state.inflight_thumbnails.lock().clear();
        self.state.failed_thumbnails.lock().clear();
        self.state.viewer_video_jobs.lock().clear();
        *self.state.batch_viewer_transcode.lock() = BatchViewerTranscodeState::idle();
        *self.state.batch_thumbnail_generation.lock() = BatchThumbnailGenerationState::idle();
        clear_viewer_render_cache(&self.state.viewer_cache_dir())?;
        self.state.db.insert_log(
            "warning",
            "reset",
            "local database reset to default state",
            None,
        )?;
        Ok(())
    }
}
