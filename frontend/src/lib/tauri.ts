import { invoke } from "@tauri-apps/api/core";

import type {
  AlbumSummary,
  AssetDetail,
  AssetListRequest,
  AssetListResponse,
  BatchViewerTranscodeStatus,
  CacheStats,
  DiagnosticEntry,
  ImportProgress,
  LogEntry,
  RefreshRequest,
  ThumbnailBatchItem,
  ViewerMediaStatus,
  ViewerPlaybackSupport,
} from "./types";

export const api = {
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
  requestThumbnailsBatch: (assetIds: number[], size: number) =>
    invoke<ThumbnailBatchItem[]>("request_thumbnails_batch", { assetIds, size }),
  requestThumbnail: (assetId: number, size: number) =>
    invoke<string | null>("request_thumbnail", { assetId, size }),
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
  getRecentLogs: (limit = 150) =>
    invoke<LogEntry[]>("get_recent_logs", { limit }),
  getThumbGenerationLogs: (limit = 400) =>
    invoke<LogEntry[]>("get_thumb_generation_logs", { limit }),
  getBatchViewerTranscodeLogs: (limit = 400) =>
    invoke<LogEntry[]>("get_batch_viewer_transcode_logs", { limit }),
  clearThumbGenerationLogs: () => invoke<void>("clear_thumb_generation_logs"),
  clearBatchViewerTranscodeLogs: () => invoke<void>("clear_batch_viewer_transcode_logs"),
  recordClientLog: (level: string, scope: string, message: string) =>
    invoke<void>("record_client_log", { level, scope, message }),
  clearDiagnostics: () => invoke<void>("clear_diagnostics"),
  clearLogs: () => invoke<void>("clear_logs"),
  getLivePhotoPair: (assetId: number) =>
    invoke<string | null>("get_live_photo_pair", { assetId }),
  resetLocalDatabase: () => invoke<void>("reset_local_database"),
};
