#!/usr/bin/env bash
#
# Cross-compile editor-core for Android targets using cargo-ndk.
#
# Targets:
#   - aarch64-linux-android     -> arm64-v8a   (most modern devices)
#   - armv7-linux-androideabi   -> armeabi-v7a  (older 32-bit devices)
#   - x86_64-linux-android      -> x86_64       (emulators)
#
# Output: rust/out/jniLibs/{arm64-v8a,armeabi-v7a,x86_64}/libeditor_core.so
#
# Prerequisites:
#   - cargo-ndk: cargo install cargo-ndk
#   - Rust toolchain with Android targets installed:
#       rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#   - Android NDK (set ANDROID_NDK_HOME or let cargo-ndk auto-detect from ANDROID_HOME)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/editor-core"
OUT_DIR="$SCRIPT_DIR/android"
LIB_NAME="libeditor_core.so"
MIN_SDK_VERSION=24
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"

if command -v rustup >/dev/null 2>&1; then
    CARGO_BIN="$(rustup which cargo)"
    RUSTC_BIN="$(rustup which rustc)"
    export RUSTC="$RUSTC_BIN"
    export PATH="$(dirname "$CARGO_BIN"):$CARGO_HOME_DIR/bin:$PATH"
fi

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
    "x86_64-linux-android x86_64"
)

echo "==> Building editor-core for Android targets..."

cd "$CRATE_DIR"

for pair in "${TARGET_ABI_PAIRS[@]}"; do
    target="${pair%% *}"
    abi="${pair#* }"
    echo "  -> $target ($abi)"

    cargo ndk \
        --target "$target" \
        --platform "$MIN_SDK_VERSION" \
        build --release

    # Copy .so to jniLibs layout
    mkdir -p "$OUT_DIR/$abi"
    cp "$CRATE_DIR/target/$target/release/$LIB_NAME" "$OUT_DIR/$abi/$LIB_NAME"
done

echo "==> Android build complete: $OUT_DIR/"
