#!/usr/bin/env bash

set -euo pipefail

repo_root="$(
  cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P
)"
validator="$repo_root/scripts/validate-packed-package.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-xcframework fixtures.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

assert_rejected() {
  local fixture="$1"
  local description="$2"
  if "$validator" --validate-xcframework "$fixture" > /dev/null 2>&1; then
    echo "ERROR: validator accepted $description" >&2
    exit 1
  fi
}

"$validator" --validate-xcframework "$repo_root/ios/EditorCore.xcframework"

metadata_fixture="$fixture_root/metadata mismatch.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$metadata_fixture"
/usr/libexec/PlistBuddy -c 'Set :AvailableLibraries:1:SupportedPlatformVariant device' "$metadata_fixture/Info.plist"
assert_rejected "$metadata_fixture" "mismatched XCFramework slice metadata"

architecture_fixture="$fixture_root/architecture mismatch.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$architecture_fixture"
cp "$architecture_fixture/ios-arm64/libeditor_core.a" \
  "$architecture_fixture/ios-arm64_x86_64-simulator/libeditor_core.a"
assert_rejected "$architecture_fixture" "a simulator archive without x86_64"

corrupt_fixture="$fixture_root/corrupt archive.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$corrupt_fixture"
: > "$corrupt_fixture/ios-arm64/libeditor_core.a"
assert_rejected "$corrupt_fixture" "an empty static archive"

echo "XCFramework validation fixture tests passed."
