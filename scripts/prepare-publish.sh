#!/usr/bin/env bash

set -euo pipefail

repo_root="$(
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
)"

cd "$repo_root"

echo "==> Syncing versioned files..."
npm run sync:version

echo "==> Rebuilding Rust artifacts and generated bindings..."
npm run build:rust

echo "==> Building JavaScript package..."
npm run build

echo "==> Validating npm package contents..."
pack_cache_dir="${NPM_CONFIG_CACHE:-/tmp/native-editor-npm-cache}"
mkdir -p "$pack_cache_dir"
pack_manifest="$(mktemp "${TMPDIR:-/tmp}/native-editor-pack.XXXXXX")"
trap 'rm -f "$pack_manifest"' EXIT
npm_config_cache="$pack_cache_dir" \
  npm_config_logs_dir="$pack_cache_dir/_logs" \
  npm pack --dry-run --ignore-scripts --json > "$pack_manifest"
cat "$pack_manifest"

for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  artifact="rust/android/$abi/libeditor_core.so"
  if ! grep -Fq "\"path\": \"$artifact\"" "$pack_manifest"; then
    echo "ERROR: npm package is missing $artifact" >&2
    exit 1
  fi
done

echo "==> Package publish prep complete."
