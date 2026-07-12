#!/usr/bin/env bash

set -euo pipefail

repo_root="$(
  cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P
)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-packed-package.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

package_dir="$work_dir/package"
consumer_dir="$work_dir/consumer"
pack_cache_dir="$work_dir/npm-cache"
cocoapods_cache_dir="$work_dir/cocoapods-cache"
cocoapods_home_dir="$work_dir/cocoapods-home"
export NODE_COMPILE_CACHE="$work_dir/node-compile-cache"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

require_file() {
  local relative_path="$1"
  [[ -s "$package_dir/$relative_path" ]] || fail "packed npm package is missing or has an empty $relative_path"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command '$1' is not installed"
}

require_declaration_symbol() {
  local symbol="$1"
  find "$package_dir/dist" -type f -name '*.d.ts' -exec grep -Fq "$symbol" {} + || \
    fail "packed TypeScript declarations are missing $symbol"
}

require_structured_create_symbols() {
  local binary_path="$1"
  local nm_output="$2"
  local label="$3"
  grep -Eq '[[:space:]]_?uniffi_editor_core_fn_func_editor_create_result$' <<< "$nm_output" || \
    fail "$label is missing the structured editor-create symbol ($binary_path)"
  grep -Eq '[[:space:]]_?uniffi_editor_core_checksum_func_editor_create_result$' <<< "$nm_output" || \
    fail "$label is missing the structured editor-create checksum ($binary_path)"
}

validate_android_library() {
  local library_path="$1"
  local abi="$2"
  local file_output
  local nm_output
  [[ -s "$library_path" ]] || fail "Android $abi library is missing or empty"
  file_output="$(file "$library_path")"
  case "$abi" in
    arm64-v8a) [[ "$file_output" == *"ELF 64-bit"* && "$file_output" == *"ARM aarch64"* ]] ;;
    armeabi-v7a) [[ "$file_output" == *"ELF 32-bit"* && "$file_output" == *"ARM, EABI5"* ]] ;;
    x86) [[ "$file_output" == *"ELF 32-bit"* && "$file_output" == *"Intel 80386"* ]] ;;
    x86_64) [[ "$file_output" == *"ELF 64-bit"* && "$file_output" == *"x86-64"* ]] ;;
    *) fail "unknown Android ABI: $abi" ;;
  esac || fail "Android $abi library has the wrong machine type: $file_output"
  nm_output="$(nm -D -g "$library_path" 2>&1)" || fail "Android $abi library has no readable dynamic symbol table: $nm_output"
  require_structured_create_symbols "$library_path" "$nm_output" "Android $abi library"
}

validate_archive_architectures() {
  local archive_path="$1"
  local expected_architectures="$2"
  local label="$3"
  local architecture_info
  local actual_architectures
  local normalized_architectures
  local architecture
  local thin_archive
  local archive_members
  local extracted_objects_dir
  local unexpected_members

  [[ -s "$archive_path" ]] || fail "$label archive is missing or empty"
  architecture_info="$(lipo -info "$archive_path" 2>&1)" || fail "$label is not a valid Mach-O archive: $architecture_info"
  actual_architectures="$(lipo -archs "$archive_path" 2>&1)" || fail "$label architectures cannot be read: $actual_architectures"
  normalized_architectures="$(printf '%s\n' "$actual_architectures" | tr ' ' '\n' | sed '/^$/d' | sort | tr '\n' ' ' | sed 's/ $//')"
  [[ "$normalized_architectures" == "$expected_architectures" ]] || \
    fail "$label must contain exactly [$expected_architectures], found [$normalized_architectures] ($architecture_info)"

  for architecture in $actual_architectures; do
    thin_archive="$work_dir/${label//[^[:alnum:]]/_}-$architecture.a"
    if [[ "$actual_architectures" == *" "* ]]; then
      lipo "$archive_path" -thin "$architecture" -output "$thin_archive" >/dev/null 2>&1 || \
        fail "$label cannot extract its $architecture archive"
    else
      cp "$archive_path" "$thin_archive"
    fi
    file "$thin_archive" | grep -Fq 'current ar archive' || \
      fail "$label $architecture slice is not a static archive"
    archive_members="$(ar -t "$thin_archive" 2>&1)" || \
      fail "$label $architecture static archive is corrupt: $archive_members"
    [[ -n "$archive_members" ]] || fail "$label $architecture static archive contains no members"
    unexpected_members="$(printf '%s\n' "$archive_members" | sed -e '/^__.SYMDEF/d' -e '/\.o$/d')"
    [[ -z "$unexpected_members" ]] || \
      fail "$label $architecture static archive contains unexpected members: $unexpected_members"
    extracted_objects_dir="$work_dir/${label//[^[:alnum:]]/_}-$architecture-objects"
    mkdir -p "$extracted_objects_dir"
    (
      cd "$extracted_objects_dir"
      ar -x "$thin_archive"
      nm -gU ./*.o >/dev/null 2>&1
    ) || fail "$label $architecture archive contains an unreadable Mach-O object member"
  done
  local nm_output
  nm_output="$(nm -gU "$archive_path" 2>&1)" || fail "$label archive symbols cannot be read: $nm_output"
  require_structured_create_symbols "$archive_path" "$nm_output" "$label archive"
}

validate_xcframework() {
  local xcframework_dir="$1"
  local plist_path="$xcframework_dir/Info.plist"
  local plist_json="$work_dir/xcframework-info.json"

  [[ -s "$plist_path" ]] || fail "XCFramework Info.plist is missing or empty"
  plutil -convert json -o "$plist_json" "$plist_path" || fail "XCFramework Info.plist is invalid"
  ruby -rjson -e '
    actual = JSON.parse(File.read(ARGV.fetch(0))).fetch("AvailableLibraries")
    expected = [
      {
        "BinaryPath" => "libeditor_core.a",
        "LibraryIdentifier" => "ios-arm64",
        "LibraryPath" => "libeditor_core.a",
        "SupportedArchitectures" => ["arm64"],
        "SupportedPlatform" => "ios",
      },
      {
        "BinaryPath" => "libeditor_core.a",
        "LibraryIdentifier" => "ios-arm64_x86_64-simulator",
        "LibraryPath" => "libeditor_core.a",
        "SupportedArchitectures" => ["arm64", "x86_64"],
        "SupportedPlatform" => "ios",
        "SupportedPlatformVariant" => "simulator",
      },
    ]
    by_identifier = ->(library) { library.fetch("LibraryIdentifier") }
    actual = actual.sort_by(&by_identifier)
    expected = expected.sort_by(&by_identifier)
    abort "XCFramework AvailableLibraries must exactly describe the device and simulator slices" unless actual == expected
  ' "$plist_json" || fail "XCFramework slice metadata does not match the packaged libraries"

  validate_archive_architectures \
    "$xcframework_dir/ios-arm64/libeditor_core.a" \
    "arm64" \
    "iOS device"
  validate_archive_architectures \
    "$xcframework_dir/ios-arm64_x86_64-simulator/libeditor_core.a" \
    "arm64 x86_64" \
    "iOS simulator"
}

require_command npm
require_command tar
require_command pod
require_command ruby
require_command plutil
require_command lipo
require_command file
require_command ar
require_command nm

if [[ "${1:-}" == "--validate-package-entries" ]]; then
  [[ "$#" == "2" ]] || fail "usage: $0 --validate-package-entries PATH"
  package_dir="$2"
  require_file "dist/index.js"
  require_file "dist/index.d.ts"
  require_declaration_symbol "NativeEditorBoundaryError"
  require_declaration_symbol "resourceLimits"
  require_declaration_symbol "requestTimeoutMs"
  echo "Packed JavaScript entry-point validation passed."
  exit 0
elif [[ "${1:-}" == "--validate-xcframework" ]]; then
  [[ "$#" == "2" ]] || fail "usage: $0 --validate-xcframework PATH"
  validate_xcframework "$2"
  echo "XCFramework metadata and static archive validation passed."
  exit 0
elif [[ "${1:-}" == "--validate-android-library" ]]; then
  [[ "$#" == "3" ]] || fail "usage: $0 --validate-android-library PATH ABI"
  validate_android_library "$2" "$3"
  echo "Android library machine type and structured symbols passed."
  exit 0
elif [[ "$#" != "0" ]]; then
  fail "unknown argument: $1"
fi

mkdir -p "$pack_cache_dir"

echo "==> Packing the publishable npm artifact..."
pack_json="$work_dir/npm-pack.json"
(
  cd "$repo_root"
  npm_config_cache="$pack_cache_dir" \
    npm_config_logs_dir="$pack_cache_dir/logs" \
    npm pack --ignore-scripts --json --pack-destination "$work_dir" > "$pack_json"
)

tarball_name="$(ruby -rjson -e 'entries = JSON.parse(File.read(ARGV.fetch(0))); abort "npm pack returned no artifact" unless entries.length == 1; puts entries.fetch(0).fetch("filename")' "$pack_json")"
tarball_path="$work_dir/$tarball_name"
[[ -f "$tarball_path" ]] || fail "npm pack did not create $tarball_name"

echo "==> Extracting and validating native artifact layout..."
tar -xzf "$tarball_path" -C "$work_dir"
[[ -d "$package_dir" ]] || fail "npm tarball does not contain the canonical package/ root"

require_file "ios/editor_coreFFI/editor_coreFFI.h"
require_file "dist/index.js"
require_file "dist/index.d.ts"
require_file "ios/editor_coreFFI/module.modulemap"
require_file "LICENSE"
require_file "ios/EditorCore.xcframework/Info.plist"
require_file "ios/EditorCore.xcframework/ios-arm64/libeditor_core.a"
require_file "ios/EditorCore.xcframework/ios-arm64_x86_64-simulator/libeditor_core.a"
validate_xcframework "$package_dir/ios/EditorCore.xcframework"

require_declaration_symbol "NativeEditorBoundaryError"
require_declaration_symbol "resourceLimits"
require_declaration_symbol "requestTimeoutMs"
grep -Fq "uniffi_editor_core_fn_func_editor_create_result" "$package_dir/ios/editor_coreFFI/editor_coreFFI.h" || \
  fail "packed generated FFI header is missing editor_create_result"
grep -Fq 'Function("editorCreateResult")' "$package_dir/android/src/main/java/com/apollohg/editor/NativeEditorModule.kt" || \
  fail "packed Android Expo module is not using structured editor creation"
grep -Fq 'Function("editorCreateResult")' "$package_dir/ios/NativeEditorModule.swift" || \
  fail "packed iOS Expo module is not using structured editor creation"

for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  require_file "rust/android/$abi/libeditor_core.so"
  validate_android_library "$package_dir/rust/android/$abi/libeditor_core.so" "$abi"
done
android_hash_count="$(shasum -a 256 "$package_dir"/rust/android/*/libeditor_core.so | awk '{print $1}' | sort -u | wc -l | tr -d '[:space:]')"
[[ "$android_hash_count" == "4" ]] || fail "packed Android ABI libraries must be four distinct binaries"

ffi_header_count="$(find "$package_dir" -type f -name 'editor_coreFFI.h' | wc -l | tr -d '[:space:]')"
modulemap_count="$(find "$package_dir" -type f \( -name 'module.modulemap' -o -name 'editor_coreFFI.modulemap' \) | wc -l | tr -d '[:space:]')"
[[ "$ffi_header_count" == "1" ]] || fail "packed npm package must contain exactly one editor_coreFFI.h (found $ffi_header_count)"
[[ "$modulemap_count" == "1" ]] || fail "packed npm package must contain exactly one UniFFI modulemap (found $modulemap_count)"

echo "==> Parsing the podspec from the extracted package..."
podspec_json="$work_dir/podspec.json"
pod ipc spec "$package_dir/ios/ReactNativeProseEditor.podspec" > "$podspec_json"
ruby -rjson -e '
  spec = JSON.parse(File.read(ARGV.fetch(0)))
  license = spec.fetch("license")
  abort "podspec license type must be Apache-2.0" unless license.fetch("type") == "Apache-2.0"
  abort "podspec license file must resolve to ../LICENSE" unless license.fetch("file") == "../LICENSE"
  frameworks = Array(spec.fetch("vendored_frameworks"))
  abort "podspec must vend EditorCore.xcframework" unless frameworks.include?("EditorCore.xcframework")
' "$podspec_json"

react_native_dir="$repo_root/example/node_modules/react-native"
expo_modules_core_dir="$repo_root/example/node_modules/expo-modules-core"
example_project_dir="$repo_root/example/ios/NativeEditorExample.xcodeproj"
[[ -f "$react_native_dir/scripts/react_native_pods.rb" ]] || fail "local example React Native dependencies are missing; run npm install in example/"
[[ -f "$expo_modules_core_dir/ExpoModulesCore.podspec" ]] || fail "local example ExpoModulesCore dependency is missing; run npm install in example/"
[[ -d "$example_project_dir" ]] || fail "local example Xcode project is missing"

echo "==> Resolving a CocoaPods consumer against the extracted package..."
mkdir -p "$consumer_dir"
cp -R "$example_project_dir" "$consumer_dir/NativeEditorExample.xcodeproj"
cat > "$consumer_dir/package.json" <<'JSON'
{
  "name": "packed-tarball-consumer",
  "private": true
}
JSON
cat > "$consumer_dir/Podfile" <<'RUBY'
require 'pathname'
require File.join(ENV.fetch('PACKED_REACT_NATIVE_DIR'), 'scripts', 'react_native_pods')

react_native_path = Pathname.new(ENV.fetch('PACKED_REACT_NATIVE_DIR'))
  .relative_path_from(Pathname.new(__dir__))
  .to_s

install! 'cocoapods', :integrate_targets => false
platform :ios, '15.1'
prepare_react_native_project!
project File.join(__dir__, 'NativeEditorExample.xcodeproj')

target 'NativeEditorExample' do
  pod 'ExpoModulesCore', :path => ENV.fetch('PACKED_EXPO_MODULES_CORE_DIR')
  pod 'ReactNativeProseEditor', :path => ENV.fetch('PACKED_EDITOR_IOS_DIR')
  use_react_native!(
    :path => react_native_path,
    :app_path => __dir__,
    :hermes_enabled => true,
    :fabric_enabled => false,
  )
end
RUBY

(
  cd "$consumer_dir"
  PACKED_EDITOR_IOS_DIR="$package_dir/ios" \
  PACKED_EXPO_MODULES_CORE_DIR="$expo_modules_core_dir" \
  PACKED_REACT_NATIVE_DIR="$react_native_dir" \
  CP_CACHE_DIR="$cocoapods_cache_dir" \
  CP_HOME_DIR="$cocoapods_home_dir" \
    pod install --no-repo-update
)

[[ -f "$consumer_dir/Podfile.lock" ]] || fail "CocoaPods did not produce Podfile.lock"
grep -Fq "ReactNativeProseEditor (from \`$package_dir/ios\`)" "$consumer_dir/Podfile.lock" || \
  fail "CocoaPods did not resolve ReactNativeProseEditor from the extracted npm package"

echo "==> Packed npm artifact and CocoaPods consumer validation passed."
