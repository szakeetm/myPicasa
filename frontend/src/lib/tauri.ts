import { invoke } from "@tauri-apps/api/core";

import type {
  AlbumSummary,
  AssetDetail,
  AssetListRequest,
  AssetListResponse,
  CacheStats,
  DiagnosticEntry,
  ImportProgress,
  LogEntry,
  RefreshRequest,
} from "./types";

export const api = {
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
  loadViewerFrame: (assetId: number) =>
    invoke<string | null>("load_viewer_frame", { assetId }),
  requestThumbnail: (assetId: number, size: number) =>
    invoke<string | null>("request_thumbnail", { assetId, size }),
  getDiagnostics: () =>
    invoke<DiagnosticEntry[]>("get_ingress_diagnostics"),
  getCacheStats: () => invoke<CacheStats>("get_cache_stats"),
  getRecentLogs: (limit = 150) =>
    invoke<LogEntry[]>("get_recent_logs", { limit }),
  recordClientLog: (level: string, scope: string, message: string) =>
    invoke<void>("record_client_log", { level, scope, message }),
  getLivePhotoPair: (assetId: number) =>
    invoke<string | null>("get_live_photo_pair", { assetId }),
  resetLocalDatabase: () => invoke<void>("reset_local_database"),
};
