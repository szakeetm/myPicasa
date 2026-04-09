import SwiftUI

@main
struct MyPicasaNativeApp: App {
    @StateObject private var model = NativeAppModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                .frame(minWidth: 1320, minHeight: 860)
        }
        .windowResizability(.contentMinSize)
    }
}