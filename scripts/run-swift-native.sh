#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
"$ROOT_DIR/scripts/build-swift-native.sh"

export DYLD_LIBRARY_PATH="$ROOT_DIR/native-macos/NativeLib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
swift run --package-path "$ROOT_DIR/native-macos" MyPicasaNativeApp