use crate::{
    db::{Database, DatabaseQueries},
    models::{AlbumSummary, AssetDetail, AssetListRequest, AssetListResponse, DiagnosticEntry, LogEntry},
    util::errors::AppError,
};

pub fn list_albums(db: &Database) -> Result<Vec<AlbumSummary>, AppError> {
    db.list_albums()
}

pub fn list_assets_by_date(db: &Database, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
    db.list_assets_by_date(request)
}

pub fn list_assets_by_album(
    db: &Database,
    album_id: i64,
    request: AssetListRequest,
) -> Result<AssetListResponse, AppError> {
    db.list_assets_by_album(album_id, request)
}

pub fn search_assets(db: &Database, request: AssetListRequest) -> Result<AssetListResponse, AppError> {
    db.search_assets(request)
}

pub fn get_asset_detail(db: &Database, asset_id: i64) -> Result<AssetDetail, AppError> {
    db.get_asset_detail(asset_id)
}

pub fn get_live_photo_pair(db: &Database, asset_id: i64) -> Result<Option<String>, AppError> {
    db.get_live_photo_pair(asset_id)
}

pub fn get_ingress_diagnostics(db: &Database) -> Result<Vec<DiagnosticEntry>, AppError> {
    db.get_ingress_diagnostics()
}

pub fn get_recent_logs(db: &Database, limit: u32) -> Result<Vec<LogEntry>, AppError> {
    db.get_recent_logs(limit)
}
