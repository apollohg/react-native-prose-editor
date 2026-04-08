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
npm_config_cache="$pack_cache_dir" \
  npm_config_logs_dir="$pack_cache_dir/_logs" \
  npm pack --dry-run --ignore-scripts

echo "==> Package publish prep complete."
