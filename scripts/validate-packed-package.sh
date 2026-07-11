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
cocoapods_cache_dir="${CP_CACHE_DIR:-$work_dir/cocoapods-cache}"

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

require_command npm
require_command tar
require_command pod
require_command ruby

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
require_file "ios/editor_coreFFI/module.modulemap"
require_file "ios/EditorCore.xcframework/Info.plist"
require_file "ios/EditorCore.xcframework/ios-arm64/libeditor_core.a"
require_file "ios/EditorCore.xcframework/ios-arm64_x86_64-simulator/libeditor_core.a"

for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  require_file "rust/android/$abi/libeditor_core.so"
done

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
    pod install --no-repo-update
)

[[ -f "$consumer_dir/Podfile.lock" ]] || fail "CocoaPods did not produce Podfile.lock"
grep -Fq "ReactNativeProseEditor (from \`$package_dir/ios\`)" "$consumer_dir/Podfile.lock" || \
  fail "CocoaPods did not resolve ReactNativeProseEditor from the extracted npm package"

echo "==> Packed npm artifact and CocoaPods consumer validation passed."
