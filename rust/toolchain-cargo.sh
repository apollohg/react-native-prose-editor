#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd -P)"
source "$script_dir/toolchain.sh"
exec "$RUST_TOOLCHAIN_CARGO" "$@"
