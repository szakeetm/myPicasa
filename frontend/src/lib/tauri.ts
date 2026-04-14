import { invoke } from "@tauri-apps/api/core";

import type {
  AlbumSummary,
  AppBackupManifest,
  AppBackupSummary,
  AppSettings,
  AssetDetail,
  AssetListRequest,
  AssetListResponse,
  BatchThumbnailGenerationStatus,
  BatchViewerTranscodeStatus,
  CacheStats,
  CacheStorageMigrationStatus,
  DiagnosticEntry,
  ImportProgress,
  LogEntry,
  RefreshRequest,
  ThumbnailBatchItem,
  ViewerMediaStatus,
  ViewerPlaybackHint,
  ViewerPlaybackSupport,
} from "./types";

export const api = {
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { settings }),
  inspectAppBackup: (backupDir: string) =>
    invoke<AppBackupManifest>("inspect_app_backup", { backupDir }),
  exportAppBackup: (backupDir: string) =>
    invoke<AppBackupSummary>("export_app_backup", { backupDir }),
  importAppBackup: (backupDir: string, takeoutRoots: string[], cacheStorageDir?: string | null) =>
    invoke<AppBackupSummary>("import_app_backup", { backupDir, takeoutRoots, cacheStorageDir }),
  getCacheStorageMigrationStatus: () =>
    invoke<CacheStorageMigrationStatus>("get_cache_storage_migration_status"),
  startCacheStorageMigration: (cacheStorageDir?: string | null, copyExisting = true) =>
    invoke<CacheStorageMigrationStatus>("start_cache_storage_migration", {
      cacheStorageDir,
      copyExisting,
    }),
  cancelCacheStorageMigration: () =>
    invoke<CacheStorageMigrationStatus>("cancel_cache_storage_migration"),
  startRefreshIndex: (request: RefreshRequest) =>
    invoke<void>("start_refresh_index", { request }),
  refreshIndex: (request: RefreshRequest) =>
    invoke<ImportProgress>("refresh_index", { request }),
  getImportStatus: () => invoke<ImportProgress | null>("get_import_status"),
  listAlbums: () => invoke<AlbumSummary[]>("list_albums"),
  listAssetsByDate: (request: AssetListRequest) =>
    invoke<AssetListResponse>("list_assets_by_date", { request }),
  listAssetsByAlbum: (albumId: number, request: AssetListRequest) =>
    invoke<AssetListResponse>("list_assets_by_album", { albumId, request }),
  searchAssets: (request: AssetListRequest) =>
    invoke<AssetListResponse>("search_assets", { request }),
  getAssetDetail: (assetId: number) =>
    invoke<AssetDetail>("get_asset_detail", { assetId }),
  loadViewerFrame: (assetId: number, preferOriginal?: boolean) =>
    invoke<string | null>("load_viewer_frame", { assetId, preferOriginal }),
  loadViewerVideo: (assetId: number, preferOriginal?: boolean) =>
    invoke<ViewerMediaStatus>("load_viewer_video", { assetId, preferOriginal }),
  loadLivePhotoMotion: (assetId: number, preferOriginal?: boolean) =>
    invoke<ViewerMediaStatus>("load_live_photo_motion", { assetId, preferOriginal }),
  requestThumbnailsBatch: (
    assetIds: number[],
    size: number,
    preferPreviewCache = false,
    checkCacheOnly = false,
  ) =>
    invoke<ThumbnailBatchItem[]>("request_thumbnails_batch", {
      assetIds,
      size,
      preferPreviewCache,
      checkCacheOnly,
    }),
  requestThumbnail: (assetId: number, size: number, preferPreviewCache = false) =>
    invoke<string | null>("request_thumbnail", { assetId, size, preferPreviewCache }),
  getBatchThumbnailGenerationStatus: () =>
    invoke<BatchThumbnailGenerationStatus>("get_batch_thumbnail_generation_status"),
  startBatchThumbnailGeneration: () =>
    invoke<BatchThumbnailGenerationStatus>("start_batch_thumbnail_generation"),
  stopBatchThumbnailGeneration: () =>
    invoke<BatchThumbnailGenerationStatus>("stop_batch_thumbnail_generation"),
  getViewerPlaybackHints: (assetIds: number[], support: ViewerPlaybackSupport) =>
    invoke<ViewerPlaybackHint[]>("get_viewer_playback_hints", { assetIds, support }),
  getDiagnostics: () =>
    invoke<DiagnosticEntry[]>("get_ingress_diagnostics"),
  getCacheStats: () => invoke<CacheStats>("get_cache_stats"),
  getBatchViewerTranscodeStatus: () =>
    invoke<BatchViewerTranscodeStatus>("get_batch_viewer_transcode_status"),
  startBatchViewerTranscode: (support: ViewerPlaybackSupport) =>
    invoke<BatchViewerTranscodeStatus>("start_batch_viewer_transcode", { support }),
  stopBatchViewerTranscode: () =>
    invoke<BatchViewerTranscodeStatus>("stop_batch_viewer_transcode"),
  clearThumbnailCache: () => invoke<void>("clear_thumbnail_cache"),
  clearViewerRenderCache: () => invoke<void>("clear_viewer_render_cache_command"),
  getRecentLogs: (limit = 10_000) =>
    invoke<LogEntry[]>("get_recent_logs", { limit }),
  getThumbGenerationLogs: (limit = 10_000) =>
    invoke<LogEntry[]>("get_thumb_generation_logs", { limit }),
  getBatchViewerTranscodeLogs: (limit = 10_000) =>
    invoke<LogEntry[]>("get_batch_viewer_transcode_logs", { limit }),
  clearThumbGenerationLogs: () => invoke<void>("clear_thumb_generation_logs"),
  clearBatchViewerTranscodeLogs: () => invoke<void>("clear_batch_viewer_transcode_logs"),
  recordClientLog: (level: string, scope: string, message: string) =>
    invoke<void>("record_client_log", { level, scope, message }),
  revealAssetInFileManager: (assetId: number) =>
    invoke<void>("reveal_asset_in_file_manager", { assetId }),
  openAssetWithDefaultApp: (assetId: number) =>
    invoke<void>("open_asset_with_default_app", { assetId }),
  openAssetPreview: (assetId: number) => invoke<void>("open_asset_preview", { assetId }),
  openUrlInBrowser: (url: string) => invoke<void>("open_url_in_browser", { url }),
  clearDiagnostics: () => invoke<void>("clear_diagnostics"),
  clearLogs: () => invoke<void>("clear_logs"),
  getLivePhotoPair: (assetId: number) =>
    invoke<string | null>("get_live_photo_pair", { assetId }),
  resetLocalDatabase: () => invoke<void>("reset_local_database"),
};
