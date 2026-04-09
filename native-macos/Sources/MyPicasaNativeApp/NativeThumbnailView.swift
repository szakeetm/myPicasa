import AppKit
import QuickLookThumbnailing
import SwiftUI

struct NativeThumbnailView: View {
    let path: String

    @State private var thumbnail: NSImage?

    var body: some View {
        GeometryReader { proxy in
            let side = min(proxy.size.width, proxy.size.height)

            ZStack {
                RoundedRectangle(cornerRadius: 14)
                    .fill(Color(nsColor: .windowBackgroundColor))

                if let thumbnail {
                    Image(nsImage: thumbnail)
                        .resizable()
                        .scaledToFill()
                        .frame(width: side, height: side)
                        .clipped()
                } else {
                    ProgressView()
                        .controlSize(.small)
                        .frame(width: side, height: side)
                }
            }
            .frame(width: side, height: side)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
            .clipShape(RoundedRectangle(cornerRadius: 14))
        }
        .task(id: path) {
            await loadThumbnail()
        }
    }

    @MainActor
    private func loadThumbnail() async {
        let url = URL(fileURLWithPath: path)
        let request = QLThumbnailGenerator.Request(
            fileAt: url,
            size: CGSize(width: 360, height: 360),
            scale: NSScreen.main?.backingScaleFactor ?? 2,
            representationTypes: [.thumbnail, .icon]
        )

        do {
            let representation = try await QLThumbnailGenerator.shared.generateBestRepresentation(for: request)
            thumbnail = representation.nsImage
        } catch {
            thumbnail = NSWorkspace.shared.icon(forFile: path)
        }
    }
}