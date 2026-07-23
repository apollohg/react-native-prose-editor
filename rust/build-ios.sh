#!/usr/bin/env bash
#
# Cross-compile editor-core for iOS targets and produce the production
# XCFramework. Since the Task 16C cutover the v2 ABI is the only surface:
# every slice is verified to export exactly 29 editor_v2_* symbols and zero
# legacy editor_*/collaboration_* symbols.
#
# Targets:
#   - aarch64-apple-ios       (physical devices)
#   - aarch64-apple-ios-sim   (Apple Silicon simulators)
#   - x86_64-apple-ios        (Intel simulators)
#
# Output: rust/ios/EditorCore.xcframework/ (synced into ios/)
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

source "$SCRIPT_DIR/toolchain.sh"
TARGET_DIR="${CARGO_TARGET_DIR:-$CRATE_DIR/target}"

IOS_TARGETS=(
    "aarch64-apple-ios"
    "aarch64-apple-ios-sim"
    "x86_64-apple-ios"
)

echo "==> Building editor-core for iOS targets..."

for target in "${IOS_TARGETS[@]}"; do
    echo "  -> $target"
    "$RUST_TOOLCHAIN_CARGO" build --manifest-path "$CRATE_DIR/Cargo.toml" --release \
        --target "$target" --target-dir "$TARGET_DIR"
done

echo "==> Verifying v2 symbols in every slice..."
for target in "${IOS_TARGETS[@]}"; do
    v2_count="$(nm -gU "$TARGET_DIR/$target/release/$LIB_NAME" 2>/dev/null | grep -c 'uniffi_editor_core_fn_func_editor_v2_' || true)"
    legacy_lines="$(nm -gU "$TARGET_DIR/$target/release/$LIB_NAME" 2>/dev/null | grep -E 'uniffi_editor_core_(fn|checksum)_func_(editor_|collaboration_)' | grep -v 'editor_v2\|editor_core_version' || true)"
    echo "  $target: $v2_count editor_v2 symbols"
    if [ "$v2_count" -ne 29 ]; then
        echo "ERROR: expected exactly 29 editor_v2 symbols in $target slice" >&2
        exit 1
    fi
    if [ -n "$legacy_lines" ]; then
        echo "ERROR: legacy symbols present in $target slice:" >&2
        echo "$legacy_lines" >&2
        exit 1
    fi
done

echo "==> Creating fat library for simulator targets..."

mkdir -p "$OUT_DIR/sim-fat" "$OUT_DIR"

lipo -create \
    "$TARGET_DIR/aarch64-apple-ios-sim/release/$LIB_NAME" \
    "$TARGET_DIR/x86_64-apple-ios/release/$LIB_NAME" \
    -output "$OUT_DIR/sim-fat/$LIB_NAME"

echo "==> Creating XCFramework..."

# Remove previous framework if it exists
rm -rf "$OUT_DIR/EditorCore.xcframework"

xcodebuild -create-xcframework \
    -library "$TARGET_DIR/aarch64-apple-ios/release/$LIB_NAME" \
    -library "$OUT_DIR/sim-fat/$LIB_NAME" \
    -output "$OUT_DIR/EditorCore.xcframework"

echo "==> Syncing XCFramework into package ios/ for CocoaPods..."
rm -rf "$PKG_IOS_DIR/EditorCore.xcframework"
cp -R "$OUT_DIR/EditorCore.xcframework" "$PKG_IOS_DIR/EditorCore.xcframework"

echo "==> iOS build complete: $OUT_DIR/EditorCore.xcframework"
