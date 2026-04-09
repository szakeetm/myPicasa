import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: NativeAppModel

    private let columns = [GridItem(.adaptive(minimum: 180, maximum: 240), spacing: 12)]

    var body: some View {
        ZStack {
            NavigationSplitView {
                sidebar
            } content: {
                contentPanel
            } detail: {
                debugPanel
            }

            if let asset = model.selectedAsset {
                viewerOverlay(asset: asset)
                    .transition(.opacity)
                    .zIndex(1)
            }
        }
        .alert("Native UI Error", isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) {
                model.errorMessage = nil
            }
        } message: {
            Text(model.errorMessage ?? "Unknown error")
        }
    }

    private func viewerOverlay(asset: NativeAssetDetail) -> some View {
        ZStack {
            Color.black.opacity(0.5)
                .ignoresSafeArea()
                .onTapGesture {
                    model.selectedAsset = nil
                }

            ViewerSheet(asset: asset)
                .environmentObject(model)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(12)
        }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            libraryPanel

            List {
                Section("Navigation") {
                    Button("Timeline") {
                        model.selectAlbum(nil)
                    }
                    .foregroundStyle(model.selectedAlbumId == nil ? Color.accentColor : Color.primary)

                    ForEach(model.albums) { album in
                        Button(album.name) {
                            model.selectAlbum(album.id)
                        }
                        .foregroundStyle(model.selectedAlbumId == album.id ? Color.accentColor : Color.primary)
                    }
                }
            }
        }
        .navigationSplitViewColumnWidth(min: 280, ideal: 320)
    }

    private var libraryPanel: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Library")
                .font(.headline)

            TextField("/path/to/Takeout/Google Photos", text: $model.rootsInput, axis: .vertical)
                .textFieldStyle(.roundedBorder)

            HStack(spacing: 10) {
                Button("Browse…") {
                    model.browseForTakeoutRoots()
                }

                Button("Refresh Index") {
                    model.refreshIndex()
                }

                Button("Reset DB", role: .destructive) {
                    model.resetDatabase()
                }
            }

            if let importStatus = model.importStatus {
                VStack(alignment: .leading, spacing: 4) {
                    Text(importStatus.status.capitalized)
                        .font(.subheadline.weight(.semibold))
                    Text(importStatus.message ?? importStatus.phase)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(10)
                .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .padding(16)
    }

    private var contentPanel: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                TextField("Search title or filename", text: $model.query)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit {
                        Task {
                            try? await model.refreshAssets(resetCursor: true)
                        }
                    }

                Picker("Media", selection: $model.mediaKind) {
                    ForEach(NativeMediaKind.allCases) { kind in
                        Text(kind.title).tag(kind)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 280)
                .onChange(of: model.mediaKind) { _, _ in
                    Task {
                        try? await model.refreshAssets(resetCursor: true)
                    }
                }

                Spacer()

                if let timelineLabel = model.timelineLabel, model.viewMode == .timeline {
                    Text(timelineLabel)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(16)

            Divider()

            ScrollView {
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(model.assets) { asset in
                        Button {
                            model.openAsset(asset)
                        } label: {
                            VStack(alignment: .leading, spacing: 8) {
                                ZStack(alignment: .topTrailing) {
                                    NativeThumbnailView(path: asset.primary_path)
                                    if asset.has_live_photo {
                                        Text("Live")
                                            .font(.caption2.weight(.bold))
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 6)
                                            .background(.ultraThinMaterial, in: Capsule())
                                            .padding(10)
                                    } else if asset.media_kind == "video" {
                                        Text(asset.duration_ms.map(formatDuration) ?? "Video")
                                            .font(.caption2.weight(.bold))
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 6)
                                            .background(.ultraThinMaterial, in: Capsule())
                                            .padding(10)
                                    }
                                }
                                .frame(maxWidth: .infinity, alignment: .center)
                                .aspectRatio(1, contentMode: .fit)

                                Text(asset.title ?? "Untitled asset")
                                    .font(.headline)
                                    .lineLimit(2)
                                    .multilineTextAlignment(.leading)
                                Text(asset.taken_at_utc ?? "Unknown date")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                            .padding(12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 18))
                        }
                        .buttonStyle(.plain)
                        .onAppear {
                            model.loadMoreAssetsIfNeeded(current: asset)
                        }
                    }
                }
                .padding(16)

                if model.isLoadingMore {
                    ProgressView()
                        .padding(.bottom, 16)
                }
            }
        }
        .navigationSplitViewColumnWidth(min: 760, ideal: 920)
    }

    private var debugPanel: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let cacheStats = model.cacheStats {
                    GroupBox("Caches") {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Thumbnails: \(cacheStats.thumbnail_items) • \(ByteCountFormatter.string(fromByteCount: Int64(cacheStats.thumbnail_bytes), countStyle: .file))")
                            Text("Previews: \(cacheStats.preview_items) • \(ByteCountFormatter.string(fromByteCount: Int64(cacheStats.preview_bytes), countStyle: .file))")
                            Text("Rendered media: \(cacheStats.viewer_render_items) • \(ByteCountFormatter.string(fromByteCount: Int64(cacheStats.viewer_render_bytes), countStyle: .file))")
                            HStack {
                                Button("Clear Thumbnails") { model.clearThumbnails() }
                                Button("Clear Rendered") { model.clearRenderedMedia() }
                            }
                        }
                    }
                }

                GroupBox("Ingress Diagnostics") {
                    VStack(alignment: .leading, spacing: 8) {
                        Button("Clear Diagnostics") { model.clearDiagnostics() }
                        ForEach(model.diagnostics.prefix(8)) { diagnostic in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(diagnostic.diagnostic_type)
                                    .font(.subheadline.weight(.semibold))
                                Text(diagnostic.message)
                                    .font(.caption)
                                if let relatedPath = diagnostic.related_path {
                                    Text(relatedPath)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                        }
                    }
                }

                GroupBox("Recent Logs") {
                    VStack(alignment: .leading, spacing: 8) {
                        Button("Clear Logs") { model.clearLogs() }
                        ForEach(model.logs.prefix(20)) { entry in
                            VStack(alignment: .leading, spacing: 4) {
                                Text("\(entry.level) • \(entry.scope)")
                                    .font(.subheadline.weight(.semibold))
                                Text(entry.message)
                                    .font(.caption)
                                Text(entry.created_at)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                        }
                    }
                }
            }
            .padding(16)
        }
        .navigationSplitViewColumnWidth(min: 320, ideal: 360)
    }

    private func formatDuration(_ durationMs: Int) -> String {
        let totalSeconds = max(0, Int(round(Double(durationMs) / 1000.0)))
        let minutes = totalSeconds / 60
        let seconds = totalSeconds % 60
        return "\(minutes):\(String(format: "%02d", seconds))"
    }
}