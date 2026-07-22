#!/usr/bin/env bash
#
# Cross-compile editor-core for Android targets using cargo-ndk. Since the
# Task 16C cutover the v2 ABI is the only surface: every ABI is verified to
# export exactly 26 editor_v2_* symbols and zero legacy
# editor_*/collaboration_* symbols.
#
# Targets:
#   - aarch64-linux-android     -> arm64-v8a   (most modern devices)
#   - armv7-linux-androideabi   -> armeabi-v7a  (older 32-bit devices)
#   - i686-linux-android        -> x86           (32-bit emulators)
#   - x86_64-linux-android      -> x86_64       (emulators)
#
# Output: rust/android/{arm64-v8a,armeabi-v7a,x86,x86_64}/libeditor_core.so
#
# Prerequisites:
#   - cargo-ndk: cargo install cargo-ndk
#   - Rust toolchain with Android targets installed:
#       rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
#   - Android NDK (set ANDROID_NDK_HOME or let cargo-ndk auto-detect from ANDROID_HOME)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/editor-core"
OUT_DIR="$SCRIPT_DIR/android"
LIB_NAME="libeditor_core.so"
MIN_SDK_VERSION=24

TC="${RUST_TOOLCHAIN_DIR:-$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin}"
TARGET_DIR="${CARGO_TARGET_DIR:-$CRATE_DIR/target}"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
export RUSTC="$TC/rustc"
export PATH="$TC:$CARGO_HOME_DIR/bin:$PATH"

# Verify cargo-ndk is installed
if ! command -v cargo-ndk &>/dev/null; then
    echo "ERROR: cargo-ndk is not installed." >&2
    echo "Install it with: cargo install cargo-ndk" >&2
    exit 1
fi

# Bash 3.2 on macOS does not support associative arrays, so keep this as a
# simple list of "target abi" pairs.
TARGET_ABI_PAIRS=(
    "aarch64-linux-android arm64-v8a"
    "armv7-linux-androideabi armeabi-v7a"
    "i686-linux-android x86"
    "x86_64-linux-android x86_64"
)

echo "==> Building editor-core for Android targets..."

cd "$CRATE_DIR"

for pair in "${TARGET_ABI_PAIRS[@]}"; do
    target="${pair%% *}"
    abi="${pair#* }"
    echo "  -> $target ($abi)"

    "$TC/cargo" ndk \
        --target "$target" \
        --platform "$MIN_SDK_VERSION" \
        build --release --target-dir "$TARGET_DIR"

    v2_count="$(nm -gU "$TARGET_DIR/$target/release/$LIB_NAME" 2>/dev/null | grep -c 'uniffi_editor_core_fn_func_editor_v2_' || true)"
    legacy_lines="$(nm -gU "$TARGET_DIR/$target/release/$LIB_NAME" 2>/dev/null | grep -E 'uniffi_editor_core_(fn|checksum)_func_(editor_|collaboration_)' | grep -v 'editor_v2\|editor_core_version' || true)"
    echo "  $target: $v2_count editor_v2 symbols"
    if [ "$v2_count" -ne 26 ]; then
        echo "ERROR: expected exactly 26 editor_v2 symbols in $target .so" >&2
        exit 1
    fi
    if [ -n "$legacy_lines" ]; then
        echo "ERROR: legacy symbols present in $target .so:" >&2
        echo "$legacy_lines" >&2
        exit 1
    fi

    # Copy .so to jniLibs layout
    mkdir -p "$OUT_DIR/$abi"
    cp "$TARGET_DIR/$target/release/$LIB_NAME" "$OUT_DIR/$abi/$LIB_NAME"
    test -s "$OUT_DIR/$abi/$LIB_NAME"
done

echo "==> Android build complete: $OUT_DIR/"
