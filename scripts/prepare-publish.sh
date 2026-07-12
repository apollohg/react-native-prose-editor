#!/usr/bin/env bash

set -euo pipefail

repo_root="$(
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd
)"

cd "$repo_root"

echo "==> Syncing versioned files..."
npm run sync:version

echo "==> Validating source security contracts..."
npm run security:validate

echo "==> Rebuilding Rust artifacts and generated bindings..."
npm run build:rust

echo "==> Building JavaScript package..."
npm run build

echo "==> Validating the packed npm artifact and CocoaPods consumer..."
npm run package:validate

echo "==> Package publish prep complete."
