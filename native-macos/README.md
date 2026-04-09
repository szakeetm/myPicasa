# Native macOS UI

This directory contains a parallel SwiftUI macOS app that talks to the Rust backend through UniFFI. It does not replace the existing Tauri app.

## Prerequisites

- Rust toolchain installed and working
- Xcode Command Line Tools or full Xcode installed
- `ffmpeg` available on `PATH` for video thumbnail and probe support

On macOS, install `ffmpeg` with Homebrew if needed:

```bash
brew install ffmpeg
```

## Build the Rust bridge and generate Swift bindings

```bash
./scripts/build-swift-native.sh
```

## Run the SwiftUI app from the command line

```bash
./scripts/run-swift-native.sh
```

This script rebuilds the Rust dylib, regenerates UniFFI bindings, sets `DYLD_LIBRARY_PATH`, and launches the SwiftUI app.

## Build only the Swift package

```bash
./scripts/build-swift-native.sh
swift build --package-path native-macos
```

## Open in Xcode

```bash
open native-macos/Package.swift
```

Then run the `MyPicasaNativeApp` scheme from Xcode. If you start it outside the helper script, make sure the Rust dylib is available via `DYLD_LIBRARY_PATH` or copied into the app runtime location.

## Storage

The native app keeps its own local app data under macOS Application Support in `myPicasa-native`. It reads your Takeout media in place and does not modify source files.