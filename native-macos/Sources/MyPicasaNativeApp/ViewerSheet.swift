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
            .background(.ultraThinMaterial)
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
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 18))
        .overlay(
            ViewerKeyboardMonitor(
                onLeft: { model.stepSelectedAsset(-1) },
                onRight: { model.stepSelectedAsset(1) },
                onEscape: { model.selectedAsset = nil }
            )
        )
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

    var body: some View {
        Group {
            if let path {
                ZoomableImageView(path: path)
            } else {
                ProgressView()
                    .controlSize(.large)
                    .tint(.white)
            }
        }
    }
}

private struct ZoomableImageView: NSViewRepresentable {
    let path: String

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> ZoomableImageScrollView {
        let scrollView = ZoomableImageScrollView()
        context.coordinator.scrollView = scrollView
        scrollView.magnificationHandler = { scrollView, recognizer in
            context.coordinator.handleMagnification(recognizer, in: scrollView)
        }
        return scrollView
    }

    func updateNSView(_ scrollView: ZoomableImageScrollView, context: Context) {
        let imageChanged = context.coordinator.loadedPath != path
        guard imageChanged else { return }

        context.coordinator.loadedPath = path
        scrollView.setImage(NSImage(contentsOfFile: path))
    }

    static func dismantleNSView(_ scrollView: ZoomableImageScrollView, coordinator: Coordinator) {
        scrollView.magnificationHandler = nil
    }

    final class Coordinator: NSObject {
        var loadedPath: String?
        weak var scrollView: ZoomableImageScrollView?
        private var initialMagnification: CGFloat = 1.0

        @MainActor
        @objc func handleMagnification(_ recognizer: NSMagnificationGestureRecognizer, in scrollView: ZoomableImageScrollView) {
            switch recognizer.state {
            case .began:
                initialMagnification = scrollView.magnification
            case .changed, .ended:
                let target = initialMagnification * (1 + recognizer.magnification)
                let location = recognizer.location(in: scrollView.documentView)
                scrollView.applyMagnification(target, centeredAt: location)
            default:
                break
            }
        }
    }
}

private final class ZoomableImageScrollView: NSScrollView {
    let containerView = NSView(frame: .zero)
    let imageView = NSImageView(frame: .zero)
    var imageSize: NSSize = .zero
    var magnificationHandler: ((ZoomableImageScrollView, NSMagnificationGestureRecognizer) -> Void)?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        configure()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configure()
    }

    private func configure() {
        drawsBackground = false
        hasHorizontalScroller = true
        hasVerticalScroller = true
        autohidesScrollers = true
        allowsMagnification = true
        minMagnification = 0.1
        maxMagnification = 8.0

        imageView.imageAlignment = .alignCenter
        imageView.imageScaling = .scaleNone
        imageView.animates = false

        containerView.addSubview(imageView)
        documentView = containerView

        let recognizer = NSMagnificationGestureRecognizer(target: self, action: #selector(handleMagnification(_:)))
        addGestureRecognizer(recognizer)
    }

    override func layout() {
        super.layout()
        updateContainerLayout()
    }

    override func reflectScrolledClipView(_ cView: NSClipView) {
        super.reflectScrolledClipView(cView)
        updateContainerLayout()
    }

    func setImage(_ image: NSImage?) {
        imageView.image = image
        imageSize = image?.size ?? .zero
        imageView.frame = NSRect(origin: .zero, size: imageSize)
        needsLayout = true

        DispatchQueue.main.async { [weak self] in
            self?.fitImageToView()
        }
    }

    func fitImageToView() {
        guard imageSize.width > 0, imageSize.height > 0 else { return }
        let contentSize = self.contentSize
        guard contentSize.width > 0, contentSize.height > 0 else { return }

        let widthScale = contentSize.width / imageSize.width
        let heightScale = contentSize.height / imageSize.height
        let fitScale = min(widthScale, heightScale)
        let centerPoint = NSPoint(x: imageSize.width / 2, y: imageSize.height / 2)
        applyMagnification(fitScale, centeredAt: centerPoint)
    }

    func applyMagnification(_ magnification: CGFloat, centeredAt point: NSPoint) {
        let clamped = min(max(magnification, minMagnification), maxMagnification)
        setMagnification(clamped, centeredAt: point)
        updateContainerLayout()
    }

    private func updateContainerLayout() {
        guard imageSize.width > 0, imageSize.height > 0 else { return }

        let safeMagnification = max(magnification, 0.0001)
        let visibleDocWidth = contentSize.width / safeMagnification
        let visibleDocHeight = contentSize.height / safeMagnification

        let containerSize = NSSize(
            width: max(imageSize.width, visibleDocWidth),
            height: max(imageSize.height, visibleDocHeight)
        )

        containerView.frame = NSRect(origin: .zero, size: containerSize)
        imageView.frame = NSRect(
            x: (containerSize.width - imageSize.width) / 2,
            y: (containerSize.height - imageSize.height) / 2,
            width: imageSize.width,
            height: imageSize.height
        )
    }

    @objc private func handleMagnification(_ recognizer: NSMagnificationGestureRecognizer) {
        magnificationHandler?(self, recognizer)
    }
}

private struct NativeVideoPlayer: View {
    let path: String?

    var body: some View {
        Group {
            if let path {
                NativeVideoPlayerView(path: path)
            } else {
                ProgressView()
                    .controlSize(.large)
                    .tint(.white)
            }
        }
    }
}

private struct NativeVideoPlayerView: NSViewRepresentable {
    let path: String

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> AVPlayerView {
        let playerView = AVPlayerView()
        playerView.controlsStyle = .floating
        playerView.videoGravity = .resizeAspect
        playerView.showsSharingServiceButton = false
        playerView.allowsPictureInPicturePlayback = false
        return playerView
    }

    func updateNSView(_ playerView: AVPlayerView, context: Context) {
        guard context.coordinator.loadedPath != path else { return }

        context.coordinator.loadedPath = path
        let url = URL(fileURLWithPath: path)
        let playerItem = AVPlayerItem(url: url)
        let player = AVPlayer(playerItem: playerItem)
        player.actionAtItemEnd = .pause

        context.coordinator.player = player
        playerView.player = player
        player.play()
    }

    static func dismantleNSView(_ playerView: AVPlayerView, coordinator: Coordinator) {
        coordinator.player?.pause()
        playerView.player = nil
        coordinator.player = nil
    }

    final class Coordinator {
        var loadedPath: String?
        var player: AVPlayer?
    }
}

private struct ViewerKeyboardMonitor: NSViewRepresentable {
    let onLeft: () -> Void
    let onRight: () -> Void
    let onEscape: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onLeft: onLeft, onRight: onRight, onEscape: onEscape)
    }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        context.coordinator.installMonitor()
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.onLeft = onLeft
        context.coordinator.onRight = onRight
        context.coordinator.onEscape = onEscape
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.removeMonitor()
    }

    final class Coordinator {
        var onLeft: () -> Void
        var onRight: () -> Void
        var onEscape: () -> Void
        private var monitor: Any?

        init(onLeft: @escaping () -> Void, onRight: @escaping () -> Void, onEscape: @escaping () -> Void) {
            self.onLeft = onLeft
            self.onRight = onRight
            self.onEscape = onEscape
        }

        func installMonitor() {
            guard monitor == nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self else { return event }

                switch event.keyCode {
                case 123:
                    self.onLeft()
                    return nil
                case 124:
                    self.onRight()
                    return nil
                case 53:
                    self.onEscape()
                    return nil
                default:
                    return event
                }
            }
        }

        func removeMonitor() {
            if let monitor {
                NSEvent.removeMonitor(monitor)
                self.monitor = nil
            }
        }
    }
}