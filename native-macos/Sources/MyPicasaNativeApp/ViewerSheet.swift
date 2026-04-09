import AVKit
import AppKit
import SwiftUI

struct ViewerSheet: View {
    @EnvironmentObject private var model: NativeAppModel
    let asset: NativeAssetDetail

    @State private var showLiveMotion = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(asset.title ?? "Untitled asset")
                        .font(.headline)
                    Text(metadataLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if let googlePhotosURL = asset.google_photos_url,
                   let url = URL(string: googlePhotosURL) {
                    Button("View On Google Photos") {
                        NSWorkspace.shared.open(url)
                    }
                }
                if asset.live_photo_video_path != nil {
                    Button(showLiveMotion ? "Show Photo" : "Play Live Photo") {
                        showLiveMotion.toggle()
                    }
                }
                Button("Show In Finder") {
                    revealInFinder()
                }
                Button("Open In Default App") {
                    openDefaultApp()
                }
                Button(asset.media_kind == "video" ? "Open In QuickTime" : "Open In Quick Look") {
                    openQuickLookOrQuickTime()
                }
                Button("Previous") { model.stepSelectedAsset(-1) }
                Button("Next") { model.stepSelectedAsset(1) }
                Button("Close") { model.selectedAsset = nil }
                    .keyboardShortcut(.cancelAction)
            }
            .padding(16)
            Divider()

            ZStack(alignment: .topTrailing) {
                Color.black.opacity(0.92)

                if asset.media_kind == "video" || showLiveMotion {
                    NativeVideoPlayer(path: showLiveMotion ? asset.live_photo_video_path : asset.primary_path)
                } else {
                    NativePhotoView(path: asset.primary_path)
                }

                if asset.live_photo_video_path != nil, !showLiveMotion {
                    Text("Live photo")
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(.ultraThinMaterial, in: Capsule())
                        .padding(18)
                }
            }
        }
        .frame(minWidth: 960, minHeight: 720)
    }

    private var metadataLine: String {
        [asset.taken_at_utc, fileSizeLabel].compactMap { $0 }.joined(separator: " • ")
    }

    private var fileSizeLabel: String? {
        guard let fileSize = asset.file_size else { return nil }
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: Int64(fileSize))
    }

    private func revealInFinder() {
        guard let path = asset.primary_path else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    private func openDefaultApp() {
        guard let path = asset.primary_path else { return }
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    private func openQuickLookOrQuickTime() {
        guard let path = asset.primary_path else { return }
        let process = Process()
        if asset.media_kind == "video" {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
            process.arguments = ["-a", "QuickTime Player", path]
        } else {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/qlmanage")
            process.arguments = ["-p", path]
        }
        try? process.run()
    }
}

private struct NativePhotoView: View {
    let path: String?
    @State private var image: NSImage?

    var body: some View {
        Group {
            if let image {
                GeometryReader { proxy in
                    ScrollView([.horizontal, .vertical]) {
                        Image(nsImage: image)
                            .resizable()
                            .scaledToFit()
                            .frame(minWidth: proxy.size.width, minHeight: proxy.size.height)
                    }
                }
            } else {
                ProgressView()
                    .controlSize(.large)
                    .tint(.white)
            }
        }
        .task(id: path) {
            guard let path else { return }
            image = NSImage(contentsOfFile: path)
        }
    }
}

private struct NativeVideoPlayer: View {
    let path: String?
    @State private var player: AVPlayer?

    var body: some View {
        Group {
            if let player {
                VideoPlayer(player: player)
                    .onAppear { player.play() }
                    .onDisappear { player.pause() }
            } else {
                ProgressView()
                    .controlSize(.large)
                    .tint(.white)
            }
        }
        .task(id: path) {
            guard let path else { return }
            player = AVPlayer(url: URL(fileURLWithPath: path))
        }
    }
}