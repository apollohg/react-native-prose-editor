#!/usr/bin/env bash
#
# Generate Swift and Kotlin bindings from the UniFFI definitions.
#
# Output:
#   rust/out/bindings/swift/    -> Swift source + modulemap
#   rust/out/bindings/kotlin/   -> Kotlin source
#
# This script uses the uniffi-bindgen binary target defined in the crate,
# which is gated behind the "cli" feature.
#
# Prerequisites:
#   - The crate must be built for the host target first (cargo build --release)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/editor-core"
OUT_DIR="$SCRIPT_DIR/bindings"
STATICLIB_PATH="$CRATE_DIR/target/release/libeditor_core.a"
CDYLIB_PATH="$CRATE_DIR/target/release/libeditor_core.dylib"

if command -v rustup >/dev/null 2>&1; then
    CARGO_BIN="$(rustup which cargo)"
    RUSTC_BIN="$(rustup which rustc)"
    export RUSTC="$RUSTC_BIN"
    CARGO_CMD=("$CARGO_BIN")
else
    CARGO_CMD=(cargo)
fi

# Always rebuild before generating bindings so UniFFI sees the current exported API.
echo "==> Building editor-core for host target..."
"${CARGO_CMD[@]}" build --manifest-path "$CRATE_DIR/Cargo.toml" --release

# uniffi-bindgen --library mode needs to find Cargo.toml via cargo metadata,
# so we run from within the crate directory.
cd "$CRATE_DIR"

echo "==> Generating Swift bindings..."
mkdir -p "$OUT_DIR/swift"
"${CARGO_CMD[@]}" run --release \
    --features cli \
    --bin uniffi-bindgen -- \
    generate --library "$STATICLIB_PATH" \
    --language swift \
    --out-dir "$OUT_DIR/swift"

echo "==> Generating Kotlin bindings..."
mkdir -p "$OUT_DIR/kotlin"
"${CARGO_CMD[@]}" run --release \
    --features cli \
    --bin uniffi-bindgen -- \
    generate --library "$CDYLIB_PATH" \
    --language kotlin \
    --out-dir "$OUT_DIR/kotlin"

echo "==> Copying Swift binding into ios/ for Xcode compilation..."
PKG_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cp "$OUT_DIR/swift/editor_core.swift" "$PKG_DIR/ios/Generated_editor_core.swift"
mkdir -p "$PKG_DIR/ios/editor_coreFFI"
cp "$OUT_DIR/swift/editor_coreFFI.h" "$PKG_DIR/ios/editor_coreFFI/editor_coreFFI.h"
cp "$OUT_DIR/swift/editor_coreFFI.modulemap" "$PKG_DIR/ios/editor_coreFFI/module.modulemap"

echo "==> Bindings generated and copied:"
echo "  Swift:     $OUT_DIR/swift/"
echo "  Kotlin:    $OUT_DIR/kotlin/"
echo "  iOS copy:  $PKG_DIR/ios/Generated_editor_core.swift"
echo "  iOS FFI:   $PKG_DIR/ios/editor_coreFFI/"
echo "  Android:   Gradle sources include $OUT_DIR/kotlin/ via build.gradle"
