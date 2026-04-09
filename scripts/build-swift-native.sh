#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="$ROOT_DIR/src-tauri"
SWIFT_DIR="$ROOT_DIR/native-macos"
GENERATED_DIR="$SWIFT_DIR/Sources/MyPicasaNativeApp/Generated"
LIB_DIR="$SWIFT_DIR/NativeLib"
FFI_DIR="$SWIFT_DIR/Sources/my_picasaFFI"

mkdir -p "$GENERATED_DIR" "$LIB_DIR" "$FFI_DIR"

pushd "$RUST_DIR" >/dev/null
cargo build --lib
LIB_PATH="$RUST_DIR/target/debug/libmy_picasa.dylib"

cargo run --bin uniffi-bindgen generate \
  --library "$LIB_PATH" \
  --language swift \
  --out-dir "$GENERATED_DIR"
popd >/dev/null

cp "$RUST_DIR/target/debug/libmy_picasa.dylib" "$LIB_DIR/libmy_picasa.dylib"
cp "$GENERATED_DIR/my_picasaFFI.h" "$FFI_DIR/my_picasaFFI.h"
cp "$GENERATED_DIR/my_picasaFFI.modulemap" "$FFI_DIR/module.modulemap"

echo "Swift bindings generated at $GENERATED_DIR"
echo "Rust dylib copied to $LIB_DIR/libmy_picasa.dylib"