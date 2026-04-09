// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "MyPicasaNativeApp",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .executable(name: "MyPicasaNativeApp", targets: ["MyPicasaNativeApp"]),
    ],
    targets: [
        .systemLibrary(
            name: "my_picasaFFI",
            path: "Sources/my_picasaFFI"
        ),
        .executableTarget(
            name: "MyPicasaNativeApp",
            dependencies: ["my_picasaFFI"],
            path: "Sources/MyPicasaNativeApp",
            swiftSettings: [
                .unsafeFlags(["-Xfrontend", "-enable-experimental-feature", "-Xfrontend", "StrictConcurrency=complete"]),
            ],
            linkerSettings: [
                .unsafeFlags(["-L", "NativeLib", "-lmy_picasa"]),
            ]
        ),
    ]
)