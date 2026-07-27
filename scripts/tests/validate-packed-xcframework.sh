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

assert_package_entries_rejected() {
  local fixture="$1"
  local description="$2"
  if "$validator" --validate-package-entries "$fixture" > /dev/null 2>&1; then
    echo "ERROR: validator accepted $description" >&2
    exit 1
  fi
}

package_fixture="$fixture_root/package entries"
mkdir -p "$package_fixture/dist" "$package_fixture/ios/Viewer/Fabric"
cp "$repo_root/package.json" "$package_fixture/package.json"
cp "$repo_root/expo-module.config.json" "$package_fixture/expo-module.config.json"
cp "$repo_root/ReactNativeProseEditor.podspec" "$package_fixture/ReactNativeProseEditor.podspec"
for viewer_source in \
  "ios/Viewer/CoreTextProseLayoutEngine.swift" \
  "ios/Viewer/PreparedProseDrawingView.swift" \
  "ios/Viewer/PreparedProseInstrumentation.swift" \
  "ios/Viewer/PreparedProseLayout.swift" \
  "ios/Viewer/PreparedProseLayoutCache.swift" \
  "ios/Viewer/PreparedProseLayoutRegistry.swift" \
  "ios/Viewer/ViewerDocument.swift" \
  "ios/Viewer/ViewerFontEnvironment.swift" \
  "ios/Viewer/ViewerImagePipeline.swift" \
  "ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.h" \
  "ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm" \
  "ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm"
do
  printf '// package entry fixture\n' > "$package_fixture/$viewer_source"
done
printf 'module.exports = {};' > "$package_fixture/dist/index.js"
printf 'export declare class NativeEditorBoundaryError {}\nexport interface NativeCollaborationTransportConfig {}\nexport interface NativeCollaborationTransportEvent {}\nexport interface EditorProps { resourceLimits?: unknown; requestTimeoutMs?: number }\n' > "$package_fixture/dist/index.d.ts"
"$validator" --validate-package-entries "$package_fixture"
: > "$package_fixture/dist/index.js"
assert_package_entries_rejected "$package_fixture" "an empty dist/index.js"
printf 'module.exports = {};' > "$package_fixture/dist/index.js"
rm "$package_fixture/dist/index.d.ts"
assert_package_entries_rejected "$package_fixture" "a missing dist/index.d.ts"
printf 'export declare class NativeEditorBoundaryError {}\nexport interface NativeCollaborationTransportConfig {}\nexport interface NativeCollaborationTransportEvent {}\nexport interface EditorProps { resourceLimits?: unknown }\n' > "$package_fixture/dist/index.d.ts"
printf 'module.exports = { requestTimeoutMs: 60000 };' > "$package_fixture/dist/index.js"
assert_package_entries_rejected "$package_fixture" "a declaration symbol present only in JavaScript"

"$validator" --validate-xcframework "$repo_root/ios/EditorCore.xcframework"

reordered_fixture="$fixture_root/reordered valid metadata.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$reordered_fixture"
plutil -replace AvailableLibraries -json '[
  {
    "BinaryPath": "libeditor_core.a",
    "LibraryIdentifier": "ios-arm64_x86_64-simulator",
    "LibraryPath": "libeditor_core.a",
    "SupportedArchitectures": ["arm64", "x86_64"],
    "SupportedPlatform": "ios",
    "SupportedPlatformVariant": "simulator"
  },
  {
    "BinaryPath": "libeditor_core.a",
    "LibraryIdentifier": "ios-arm64",
    "LibraryPath": "libeditor_core.a",
    "SupportedArchitectures": ["arm64"],
    "SupportedPlatform": "ios"
  }
]' "$reordered_fixture/Info.plist"
"$validator" --validate-xcframework "$reordered_fixture"

metadata_fixture="$fixture_root/metadata mismatch.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$metadata_fixture"
/usr/libexec/PlistBuddy -c 'Set :AvailableLibraries:0:SupportedPlatform tvos' "$metadata_fixture/Info.plist"
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

corrupt_member_fixture="$fixture_root/corrupt member.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$corrupt_member_fixture"
printf 'not-a-mach-o-object' > "$fixture_root/invalid-object.o"
ar -qS "$corrupt_member_fixture/ios-arm64/libeditor_core.a" "$fixture_root/invalid-object.o"
assert_rejected "$corrupt_member_fixture" "a nonempty archive with a corrupt object member"

missing_symbol_fixture="$fixture_root/missing symbols.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$missing_symbol_fixture"
printf 'int unrelated_symbol(void) { return 1; }\n' > "$fixture_root/unrelated.c"
xcrun clang -target arm64-apple-ios15.1 -c "$fixture_root/unrelated.c" -o "$fixture_root/unrelated.o"
ar -rcs "$fixture_root/unrelated.a" "$fixture_root/unrelated.o"
cp "$fixture_root/unrelated.a" "$missing_symbol_fixture/ios-arm64/libeditor_core.a"
assert_rejected "$missing_symbol_fixture" "an architecture-correct archive without structured-create symbols"

mixed_symbol_fixture="$fixture_root/mixed per-architecture symbols.xcframework"
cp -R "$repo_root/ios/EditorCore.xcframework" "$mixed_symbol_fixture"
xcrun clang -target x86_64-apple-ios15.1-simulator -c "$fixture_root/unrelated.c" \
  -o "$fixture_root/unrelated-x86_64.o"
ar -rcs "$fixture_root/unrelated-x86_64.a" "$fixture_root/unrelated-x86_64.o"
lipo -create \
  "$repo_root/ios/EditorCore.xcframework/ios-arm64/libeditor_core.a" \
  "$fixture_root/unrelated-x86_64.a" \
  -output "$mixed_symbol_fixture/ios-arm64_x86_64-simulator/libeditor_core.a"
assert_rejected \
  "$mixed_symbol_fixture" \
  "a fat archive whose structured-create symbols exist only in its arm64 slice"

"$validator" --validate-android-library \
  "$repo_root/rust/android/x86_64/libeditor_core.so" x86_64
if "$validator" --validate-android-library \
  "$repo_root/rust/android/x86_64/libeditor_core.so" arm64-v8a > /dev/null 2>&1; then
  echo "ERROR: validator accepted an x86_64 library mislabeled as arm64-v8a" >&2
  exit 1
fi
printf 'int unrelated_symbol(void) { return 1; }\n' > "$fixture_root/unrelated-elf.c"
cc -shared "$fixture_root/unrelated-elf.c" -o "$fixture_root/unrelated.so"
if "$validator" --validate-android-library "$fixture_root/unrelated.so" x86_64 > /dev/null 2>&1; then
  echo "ERROR: validator accepted an ELF library without structured-create symbols" >&2
  exit 1
fi

echo "XCFramework validation fixture tests passed."
