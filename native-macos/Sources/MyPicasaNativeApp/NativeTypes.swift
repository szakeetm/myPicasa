import Foundation

struct NativeRefreshRequest: Codable {
    var roots: [String]
}

struct NativeAssetListRequest: Codable {
    var cursor: Int?
    var limit: Int?
    var query: String?
    var media_kind: String?
    var date_from: String?
    var date_to: String?
}

struct NativeAlbumSummary: Codable, Identifiable, Hashable {
    let id: Int64
    let name: String
    let source_path: String
    let asset_count: Int
    let begin_taken_at_utc: String?
    let end_taken_at_utc: String?
}

struct NativeAssetListItem: Codable, Identifiable, Hashable {
    let id: Int64
    let title: String?
    let media_kind: String
    let taken_at_utc: String?
    let duration_ms: Int?
    let has_live_photo: Bool
    let primary_path: String
    let albums: [String]
}

struct NativeAssetListResponse: Codable {
    let items: [NativeAssetListItem]
    let next_cursor: Int?
}

struct NativeAssetDetail: Codable, Identifiable, Hashable {
    let id: Int64
    let title: String?
    let media_kind: String
    let display_type: String
    let taken_at_utc: String?
    let file_size: Int?
    let width: Int?
    let height: Int?
    let duration_ms: Int?
    let gps_lat: Double?
    let gps_lon: Double?
    let primary_path: String?
    let albums: [String]
    let live_photo_video_path: String?
    let google_photos_url: String?
}

struct NativeDiagnosticEntry: Codable, Identifiable, Hashable {
    let id: Int64
    let import_id: Int64
    let severity: String
    let diagnostic_type: String
    let related_path: String?
    let message: String
    let created_at: String
}

struct NativeLogEntry: Codable, Identifiable, Hashable {
    let id: Int64
    let created_at: String
    let level: String
    let scope: String
    let message: String
    let asset_id: Int64?
}

struct NativeCacheStats: Codable {
    let thumbnail_items: Int
    let thumbnail_bytes: UInt64
    let thumbnail_budget_bytes: UInt64
    let preview_items: Int
    let preview_bytes: UInt64
    let preview_budget_bytes: UInt64
    let viewer_render_items: Int
    let viewer_render_bytes: UInt64
}

struct NativeImportProgress: Codable {
    let import_id: Int64
    let status: String
    let phase: String
    let files_scanned: Int
    let processed_files: Int
    let total_files: Int
    let files_added: Int
    let files_updated: Int
    let files_deleted: Int
    let assets_added: Int
    let assets_updated: Int
    let assets_deleted: Int
    let worker_count: Int
    let message: String?
}

enum NativeViewMode: String, CaseIterable {
    case timeline
    case album
}

enum NativeMediaKind: String, CaseIterable, Identifiable {
    case all = ""
    case photo
    case video

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: return "All media"
        case .photo: return "Photos"
        case .video: return "Videos"
        }
    }
}