export type RefreshRequest = { roots: string[] };

export type ImportProgress = {
  import_id: number;
  status: string;
  phase: string;
  files_scanned: number;
  processed_files: number;
  total_files: number;
  files_added: number;
  files_updated: number;
  files_deleted: number;
  assets_added: number;
  assets_updated: number;
  assets_deleted: number;
  worker_count: number;
  message?: string | null;
};

export type AssetListRequest = {
  cursor?: number;
  limit?: number;
  query?: string;
  media_kind?: string;
  date_from?: string;
  date_to?: string;
};

export type AlbumSummary = {
  id: number;
  name: string;
  source_path: string;
  asset_count: number;
  begin_taken_at_utc?: string | null;
  end_taken_at_utc?: string | null;
};

export type AssetListItem = {
  id: number;
  title?: string | null;
  media_kind: string;
  taken_at_utc?: string | null;
  duration_ms?: number | null;
  has_live_photo: boolean;
  primary_path: string;
  albums: string[];
};

export type AssetListResponse = {
  items: AssetListItem[];
  next_cursor?: number | null;
};

export type AssetDetail = {
  id: number;
  title?: string | null;
  media_kind: string;
  display_type: string;
  taken_at_utc?: string | null;
  file_size?: number | null;
  width?: number | null;
  height?: number | null;
  duration_ms?: number | null;
  gps_lat?: number | null;
  gps_lon?: number | null;
  primary_path?: string | null;
  albums: string[];
  live_photo_video_path?: string | null;
};

export type DiagnosticEntry = {
  id: number;
  import_id: number;
  severity: string;
  diagnostic_type: string;
  related_path?: string | null;
  message: string;
  created_at: string;
};

export type CacheStats = {
  thumbnail_items: number;
  thumbnail_bytes: number;
  thumbnail_budget_bytes: number;
  preview_items: number;
  preview_bytes: number;
  preview_budget_bytes: number;
  viewer_render_items: number;
  viewer_render_bytes: number;
};

export type LogEntry = {
  id: number;
  created_at: string;
  level: string;
  scope: string;
  message: string;
  asset_id?: number | null;
};

export type ThumbnailBatchItem = {
  asset_id: number;
  status: "ready" | "pending" | "unavailable";
  data_url?: string | null;
};

export type ViewerMediaStatus = {
  status: "ready" | "pending" | "unavailable";
  src?: string | null;
  source?: string | null;
  message?: string | null;
  codec?: string | null;
  encoder?: string | null;
  elapsed_ms?: number | null;
  timeout_ms?: number | null;
  source_bytes?: number | null;
  output_bytes?: number | null;
};
