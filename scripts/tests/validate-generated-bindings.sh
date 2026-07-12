#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-generated-bindings.XXXXXX")"
trap 'rm -rf "$fixture_dir"' EXIT

header="$fixture_dir/editor_coreFFI.h"
printf 'first   \nsecond\t\nthird\n' > "$header"
bash "$repo_root/rust/generate-bindings.sh" --normalize-header "$header"
expected="$fixture_dir/expected.h"
printf 'first\nsecond\nthird\n' > "$expected"
cmp -s "$header" "$expected" || {
  echo "ERROR: generated header normalization is not deterministic" >&2
  exit 1
}

cmp -s "$repo_root/ios/Generated_editor_core.swift" \
  "$repo_root/rust/bindings/swift/editor_core.swift" || {
  echo "ERROR: generated Swift source copies differ" >&2
  exit 1
}
cmp -s "$repo_root/ios/editor_coreFFI/editor_coreFFI.h" \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" || {
  echo "ERROR: generated FFI header copies differ" >&2
  exit 1
}

if rg -n '[[:blank:]]+$' \
  "$repo_root/ios/editor_coreFFI/editor_coreFFI.h" \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h"; then
  echo "ERROR: generated FFI headers contain trailing whitespace" >&2
  exit 1
fi

echo "Generated binding normalization and copy validation passed."
