import AppKit
import Foundation

@MainActor
final class NativeAppModel: ObservableObject {
    @Published var rootsInput = ""
    @Published var query = ""
    @Published var mediaKind: NativeMediaKind = .all
    @Published var viewMode: NativeViewMode = .timeline
    @Published var selectedAlbumId: Int64?
    @Published var albums: [NativeAlbumSummary] = []
    @Published var assets: [NativeAssetListItem] = []
    @Published var selectedAsset: NativeAssetDetail?
    @Published var diagnostics: [NativeDiagnosticEntry] = []
    @Published var logs: [NativeLogEntry] = []
    @Published var cacheStats: NativeCacheStats?
    @Published var importStatus: NativeImportProgress?
    @Published var timelineLabel: String?
    @Published var nextCursor: Int?
    @Published var isLoadingMore = false
    @Published var errorMessage: String?

    private let bridge: NativeAppBridge
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()
    private var importPollingTask: Task<Void, Never>?

    init() {
        let defaults = UserDefaults.standard
        rootsInput = defaults.string(forKey: "native.rootsInput") ?? ""

        do {
            bridge = try NativeAppBridge(appDataDir: Self.defaultAppDataDirectory.path)
        } catch {
            fatalError("Failed to initialize Rust bridge: \(error)")
        }

        Task {
            await refreshAllSurfaces(resetAssets: true)
        }
    }

    deinit {
        importPollingTask?.cancel()
    }

    func refreshAllSurfaces(resetAssets: Bool) async {
        do {
            try await refreshDebugSurfaces()
            if resetAssets {
                try await refreshAssets(resetCursor: true)
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func refreshAssets(resetCursor: Bool) async throws {
        let request = NativeAssetListRequest(
            cursor: resetCursor ? nil : nextCursor,
            limit: 200,
            query: query.isEmpty ? nil : query,
            media_kind: mediaKind.rawValue.isEmpty ? nil : mediaKind.rawValue,
            date_from: nil,
            date_to: nil
        )

        let requestJSON = try String(data: encoder.encode(request), encoding: .utf8).unwrapOrThrow("request encoding failed")
        let response: NativeAssetListResponse
        if !query.isEmpty {
            response = try decode(NativeAssetListResponse.self, from: try bridge.searchAssetsJson(requestJson: requestJSON))
        } else if viewMode == .album, let albumId = selectedAlbumId {
            response = try decode(
                NativeAssetListResponse.self,
                from: try bridge.listAssetsByAlbumJson(albumId: albumId, requestJson: requestJSON)
            )
        } else {
            response = try decode(NativeAssetListResponse.self, from: try bridge.listAssetsByDateJson(requestJson: requestJSON))
        }

        if resetCursor {
            assets = response.items
        } else {
            let seen = Set(assets.map(\ .id))
            assets.append(contentsOf: response.items.filter { !seen.contains($0.id) })
        }
        nextCursor = response.next_cursor
        timelineLabel = Self.formatTimelineLabel(from: assets.first?.taken_at_utc)
    }

    func loadMoreAssetsIfNeeded(current asset: NativeAssetListItem?) {
        guard let asset, let nextCursor, !isLoadingMore else {
            return
        }
        let thresholdIndex = assets.index(assets.endIndex, offsetBy: -12, limitedBy: assets.startIndex) ?? assets.startIndex
        guard assets.firstIndex(where: { $0.id == asset.id }) == thresholdIndex else {
            return
        }

        isLoadingMore = true
        Task {
            defer { isLoadingMore = false }
            do {
                self.nextCursor = nextCursor
                try await refreshAssets(resetCursor: false)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func refreshDebugSurfaces() async throws {
        albums = try decode([NativeAlbumSummary].self, from: try bridge.listAlbumsJson())
        diagnostics = try decode([NativeDiagnosticEntry].self, from: try bridge.getIngressDiagnosticsJson())
        logs = try decode([NativeLogEntry].self, from: try bridge.getRecentLogsJson(limit: 300))
        cacheStats = try decode(NativeCacheStats.self, from: try bridge.getCacheStatsJson())
        importStatus = try decode(Optional<NativeImportProgress>.self, from: try bridge.getImportStatusJson())
    }

    func selectAlbum(_ albumId: Int64?) {
        selectedAlbumId = albumId
        viewMode = albumId == nil ? .timeline : .album
        Task {
            do {
                try await refreshAssets(resetCursor: true)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func refreshIndex() {
        let roots = rootsInput
            .split(separator: ";")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }

        guard !roots.isEmpty else {
            errorMessage = "Add at least one Takeout root first."
            return
        }

        UserDefaults.standard.set(rootsInput, forKey: "native.rootsInput")

        do {
            try bridge.startRefreshIndex(roots: roots)
            startImportPolling()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    func browseForTakeoutRoots() {
        let panel = NSOpenPanel()
        panel.title = "Choose Google Photos Takeout folders"
        panel.message = "Select one or more folders that contain your Takeout exports."
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.canCreateDirectories = false

        guard panel.runModal() == .OK else {
            return
        }

        let selectedPaths = panel.urls.map(\ .path)
        guard !selectedPaths.isEmpty else {
            return
        }

        let existingPaths = rootsInput
            .split(separator: ";")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }

        var mergedPaths: [String] = []
        for path in existingPaths + selectedPaths {
            if !mergedPaths.contains(path) {
                mergedPaths.append(path)
            }
        }

        rootsInput = mergedPaths.joined(separator: "; ")
    }

    func startImportPolling() {
        importPollingTask?.cancel()
        importPollingTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                do {
                    self.importStatus = try self.decode(Optional<NativeImportProgress>.self, from: try self.bridge.getImportStatusJson())
                    if self.importStatus?.status == "completed" || self.importStatus?.status == "failed" {
                        try await self.refreshDebugSurfaces()
                        try await self.refreshAssets(resetCursor: true)
                        return
                    }
                } catch {
                    self.errorMessage = error.localizedDescription
                    return
                }
                try? await Task.sleep(for: .milliseconds(450))
            }
        }
    }

    func openAsset(_ asset: NativeAssetListItem) {
        Task {
            do {
                selectedAsset = try await loadAssetDetail(assetId: asset.id)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func stepSelectedAsset(_ direction: Int) {
        guard let selectedAsset,
              let index = assets.firstIndex(where: { $0.id == selectedAsset.id }) else {
            return
        }
        let nextIndex = index + direction
        guard assets.indices.contains(nextIndex) else {
            return
        }
        openAsset(assets[nextIndex])
    }

    private func loadAssetDetail(assetId: Int64) async throws -> NativeAssetDetail {
        let bridge = self.bridge
        let json = try await Task.detached(priority: .userInitiated) {
            try bridge.getAssetDetailJson(assetId: assetId)
        }.value
        return try decode(NativeAssetDetail.self, from: json)
    }

    func clearThumbnails() {
        runDestructiveAction(
            title: "Clear thumbnails?",
            text: "This clears generated thumbnails and previews from the native backend cache."
        ) {
            try self.bridge.clearThumbnailCache()
            try await self.refreshDebugSurfaces()
        }
    }

    func clearRenderedMedia() {
        runDestructiveAction(
            title: "Clear rendered media?",
            text: "This deletes cached rendered viewer media and video transcodes for the native app."
        ) {
            try self.bridge.clearViewerRenderCache()
            try await self.refreshDebugSurfaces()
        }
    }

    func clearDiagnostics() {
        Task {
            do {
                try bridge.clearDiagnostics()
                try await refreshDebugSurfaces()
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func clearLogs() {
        Task {
            do {
                try bridge.clearLogs()
                try await refreshDebugSurfaces()
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    func resetDatabase() {
        runDestructiveAction(
            title: "Reset local database?",
            text: "This clears the native app's local index, logs, diagnostics, and cache data. Original Takeout files remain untouched."
        ) {
            try self.bridge.resetLocalDatabase()
            self.selectedAlbumId = nil
            self.selectedAsset = nil
            self.viewMode = .timeline
            await self.refreshAllSurfaces(resetAssets: true)
        }
    }

    private func runDestructiveAction(title: String, text: String, operation: @escaping () async throws -> Void) {
        guard Self.confirm(title: title, text: text) else { return }
        Task {
            do {
                try await operation()
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
        try decoder.decode(type, from: Data(json.utf8))
    }

    static var defaultAppDataDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dir = base.appendingPathComponent("myPicasa-native", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static func formatTimelineLabel(from value: String?) -> String? {
        guard let value else { return nil }
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: value) {
            let out = DateFormatter()
            out.dateFormat = "MMMM yyyy"
            return out.string(from: date)
        }
        return nil
    }

    static func confirm(title: String, text: String) -> Bool {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Continue")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }
}

private extension Optional where Wrapped == String {
    func unwrapOrThrow(_ message: String) throws -> String {
        guard let self else {
            throw NSError(domain: "MyPicasaNativeApp", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
        }
        return self
    }
}