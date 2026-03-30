#!/usr/bin/env bash
#
# Cross-compile editor-core for iOS targets and produce an XCFramework.
#
# Targets:
#   - aarch64-apple-ios       (physical devices)
#   - aarch64-apple-ios-sim   (Apple Silicon simulators)
#   - x86_64-apple-ios        (Intel simulators)
#
# Output: rust/out/EditorCore.xcframework/
#
# Prerequisites:
#   - Rust toolchain with iOS targets installed:
#       rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
#   - Xcode command-line tools

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/editor-core"
OUT_DIR="$SCRIPT_DIR/ios"
PKG_IOS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/ios"
LIB_NAME="libeditor_core.a"

if command -v rustup >/dev/null 2>&1; then
    CARGO_BIN="$(rustup which cargo)"
    RUSTC_BIN="$(rustup which rustc)"
    export RUSTC="$RUSTC_BIN"
    CARGO_CMD=("$CARGO_BIN")
else
    CARGO_CMD=(cargo)
fi

IOS_TARGETS=(
    "aarch64-apple-ios"
    "aarch64-apple-ios-sim"
    "x86_64-apple-ios"
)

echo "==> Building editor-core for iOS targets..."

for target in "${IOS_TARGETS[@]}"; do
    echo "  -> $target"
    "${CARGO_CMD[@]}" build --manifest-path "$CRATE_DIR/Cargo.toml" --release --target "$target"
done

echo "==> Creating fat library for simulator targets..."

mkdir -p "$OUT_DIR/sim-fat" "$OUT_DIR"

lipo -create \
    "$CRATE_DIR/target/aarch64-apple-ios-sim/release/$LIB_NAME" \
    "$CRATE_DIR/target/x86_64-apple-ios/release/$LIB_NAME" \
    -output "$OUT_DIR/sim-fat/$LIB_NAME"

echo "==> Creating XCFramework..."

# Remove previous framework if it exists
rm -rf "$OUT_DIR/EditorCore.xcframework"

xcodebuild -create-xcframework \
    -library "$CRATE_DIR/target/aarch64-apple-ios/release/$LIB_NAME" \
    -library "$OUT_DIR/sim-fat/$LIB_NAME" \
    -output "$OUT_DIR/EditorCore.xcframework"

echo "==> Syncing XCFramework into package ios/ for CocoaPods..."
rm -rf "$PKG_IOS_DIR/EditorCore.xcframework"
cp -R "$OUT_DIR/EditorCore.xcframework" "$PKG_IOS_DIR/EditorCore.xcframework"

echo "==> iOS build complete: $OUT_DIR/EditorCore.xcframework"
