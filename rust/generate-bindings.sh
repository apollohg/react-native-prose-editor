#!/usr/bin/env bash
#
# Generate Swift and Kotlin bindings from the UniFFI definitions.
#
# Output:
#   rust/bindings/swift/    -> Swift source + modulemap
#   rust/bindings/kotlin/   -> Kotlin source
#
# This script uses the uniffi-bindgen binary target defined in the crate,
# which is gated behind the "cli" feature.
#
# Prerequisites:
#   - The crate must be built for the host target first (cargo build --release)
#
# Since the Task 16C production cutover, the v2 ABI is the ONLY surface: the
# generated bindings and the dylib are verified to expose all 29 editor_v2_*
# symbols plus editor_core_version and zero legacy editor_*/collaboration_*
# symbols before the script succeeds.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/editor-core"
OUT_DIR="$SCRIPT_DIR/bindings"
TARGET_DIR="${CARGO_TARGET_DIR:-$CRATE_DIR/target}"
STATICLIB_PATH="$TARGET_DIR/release/libeditor_core.a"
CDYLIB_PATH="$TARGET_DIR/release/libeditor_core.dylib"

V2_SYMBOLS=(
    editor_v2_create
    editor_v2_destroy
    editor_v2_get_state
    editor_v2_get_document_json
    editor_v2_get_document_html
    editor_v2_get_content_snapshot
    editor_v2_replace_document
    editor_v2_apply_input
    editor_v2_apply_command
    editor_v2_apply_local_api
    editor_v2_set_selection
    editor_v2_undo
    editor_v2_redo
    editor_v2_collaboration_begin_connect
    editor_v2_collaboration_socket_open
    editor_v2_collaboration_receive
    editor_v2_collaboration_socket_close
    editor_v2_collaboration_take_outbound
    editor_v2_collaboration_set_awareness
    editor_v2_collaboration_peers
    editor_v2_collaboration_tick
    editor_v2_collaboration_detach
    editor_v2_collaboration_reattach
    editor_v2_snapshot_export
    editor_v2_snapshot_restore
    editor_v2_render_update
    editor_v2_resolve_scalar_selection
    editor_v2_doc_to_scalar
    editor_v2_scalar_to_doc
)

normalize_header() {
    local header_path="$1"
    local normalized_path="${header_path}.normalized"
    awk '
      {
        sub(/[[:blank:]]+$/, "")
        if ($0 == "") {
          blank_lines += 1
        } else {
          while (blank_lines > 0) {
            print ""
            blank_lines -= 1
          }
          print
        }
      }
    ' "$header_path" > "$normalized_path"
    mv "$normalized_path" "$header_path"
}

usage() {
    echo "usage: $0 [--normalize-header PATH]" >&2
    exit 2
}

if [[ "${1:-}" == "--normalize-header" ]]; then
    [[ "$#" == "2" ]] || usage
    normalize_header "$2"
    exit 0
elif [[ "$#" != "0" ]]; then
    echo "unknown argument: ${1:-}" >&2
    exit 2
fi

# The crate pins Rust 1.95; honor an explicit override but default to the
# pinned toolchain so a different default toolchain cannot slip through.
RUST_TOOLCHAIN_DIR="${RUST_TOOLCHAIN_DIR:-$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin}"
if [[ -x "$RUST_TOOLCHAIN_DIR/cargo" ]]; then
    export RUSTC="$RUST_TOOLCHAIN_DIR/rustc"
    export RUSTDOC="$RUST_TOOLCHAIN_DIR/rustdoc"
    CARGO_CMD=("$RUST_TOOLCHAIN_DIR/cargo")
elif command -v rustup >/dev/null 2>&1; then
    CARGO_BIN="$(rustup which cargo)"
    RUSTC_BIN="$(rustup which rustc)"
    RUSTDOC_BIN="$(rustup which rustdoc)"
    export RUSTC="$RUSTC_BIN"
    export RUSTDOC="$RUSTDOC_BIN"
    CARGO_CMD=("$CARGO_BIN")
else
    CARGO_CMD=(cargo)
fi

# Always rebuild before generating bindings so UniFFI sees the current exported API.
echo "==> Building editor-core for host target..."
"${CARGO_CMD[@]}" build --manifest-path "$CRATE_DIR/Cargo.toml" --release \
    --target-dir "$TARGET_DIR"

echo "==> Verifying the dylib exposes exactly the ${#V2_SYMBOLS[@]} editor_v2_* symbols..."
for symbol in "${V2_SYMBOLS[@]}"; do
    nm -gU "$CDYLIB_PATH" | grep -q "uniffi_editor_core_fn_func_${symbol}" || {
        echo "error: dylib is missing uniffi_editor_core_fn_func_${symbol}" >&2
        exit 1
    }
done
nm -gU "$CDYLIB_PATH" | grep -q "uniffi_editor_core_fn_func_editor_core_version" || {
    echo "error: dylib is missing the editor_core_version query" >&2
    exit 1
}
LEGACY_LINES="$(nm -gU "$CDYLIB_PATH" | grep -E 'uniffi_editor_core_(fn|checksum)_func_(editor_|collaboration_)' | grep -v 'editor_v2\|editor_core_version' || true)"
if [[ -n "$LEGACY_LINES" ]]; then
    echo "error: dylib exposes legacy editor_*/collaboration_* symbols (expected 0):" >&2
    echo "$LEGACY_LINES" >&2
    exit 1
fi

# uniffi-bindgen --library mode needs to find Cargo.toml via cargo metadata,
# so we run from within the crate directory.
cd "$CRATE_DIR"

echo "==> Generating Swift bindings..."
mkdir -p "$OUT_DIR/swift"
"${CARGO_CMD[@]}" run --release \
    --features cli \
    --target-dir "$TARGET_DIR" \
    --bin uniffi-bindgen -- \
    generate --library "$STATICLIB_PATH" \
    --language swift \
    --out-dir "$OUT_DIR/swift"
normalize_header "$OUT_DIR/swift/editor_coreFFI.h"
normalize_header "$OUT_DIR/swift/editor_core.swift"

echo "==> Generating Kotlin bindings..."
mkdir -p "$OUT_DIR/kotlin"
"${CARGO_CMD[@]}" run --release \
    --features cli \
    --target-dir "$TARGET_DIR" \
    --bin uniffi-bindgen -- \
    generate --library "$CDYLIB_PATH" \
    --language kotlin \
    --out-dir "$OUT_DIR/kotlin"

echo "==> Verifying the generated bindings expose the v2 symbols..."
for symbol in "${V2_SYMBOLS[@]}"; do
    for artifact in \
        "$OUT_DIR/swift/editor_coreFFI.h" \
        "$OUT_DIR/swift/editor_core.swift" \
        "$OUT_DIR/kotlin/uniffi/editor_core/editor_core.kt"; do
        grep -q "uniffi_editor_core_fn_func_${symbol}" "$artifact" || {
            echo "error: generated binding $artifact is missing uniffi_editor_core_fn_func_${symbol}" >&2
            exit 1
        }
    done
done
for artifact in \
    "$OUT_DIR/swift/editor_coreFFI.h" \
    "$OUT_DIR/swift/editor_core.swift" \
    "$OUT_DIR/kotlin/uniffi/editor_core/editor_core.kt"; do
    LEGACY_ARTIFACT_LINES="$(grep -E 'uniffi_editor_core_(fn|checksum)_func_(editor_|collaboration_)' "$artifact" | grep -v 'editor_v2\|editor_core_version' || true)"
    if [[ -n "$LEGACY_ARTIFACT_LINES" ]]; then
        echo "error: generated binding $artifact exposes legacy symbols (expected 0):" >&2
        echo "$LEGACY_ARTIFACT_LINES" >&2
        exit 1
    fi
done

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
