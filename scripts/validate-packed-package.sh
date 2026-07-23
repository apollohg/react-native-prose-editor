#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
manifest_path="$repo_root/scripts/package-abi-manifest.json"
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

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command '$1' is not installed"
}

require_file() {
  local root="$1"
  local relative_path="$2"
  [[ -s "$root/$relative_path" ]] || fail "missing or empty $relative_path under $root"
}

manifest_rows() {
  ruby -rjson -e '
    manifest = JSON.parse(File.read(ARGV.fetch(0)))
    entries = manifest.fetch("functions")
    abort "package ABI manifest must contain exactly 29 editor_v2 functions" unless entries.length == 29
    names = entries.map { |entry| entry.fetch("name") }
    abort "package ABI manifest contains duplicate function names" unless names.uniq.length == names.length
    abort "package ABI manifest contains a non-v2 function" unless names.all? { |name| name.start_with?("editor_v2_") }
    version = manifest.fetch("version")
    puts [version.fetch("name"), version.fetch("checksum")].join("\t")
    entries.sort_by { |entry| entry.fetch("name") }.each do |entry|
      puts [entry.fetch("name"), entry.fetch("checksum")].join("\t")
    end
  ' "$manifest_path"
}

expected_symbol_names() {
  manifest_rows | cut -f1 | sort
}

compare_exact_symbol_set() {
  local label="$1"
  local actual_file="$2"
  local expected_file="$work_dir/expected-symbols.txt"
  local missing_file unexpected_file legacy

  expected_symbol_names > "$expected_file"
  sort -u "$actual_file" -o "$actual_file"

  legacy="$(grep -E '^(editor_|collaboration_)' "$actual_file" | grep -Ev '^editor_v2_|^editor_core_version$' || true)"
  if [[ -n "$legacy" ]]; then
    fail "$label exposes legacy UniFFI function symbol: $(printf '%s\n' "$legacy" | head -n 1)"
  fi

  missing_file="$(mktemp "$work_dir/missing-symbols.XXXXXX")"
  unexpected_file="$(mktemp "$work_dir/unexpected-symbols.XXXXXX")"
  comm -23 "$expected_file" "$actual_file" > "$missing_file"
  comm -13 "$expected_file" "$actual_file" > "$unexpected_file"
  if [[ -s "$missing_file" ]]; then
    fail "$label is missing expected function symbol: $(head -n 1 "$missing_file")"
  fi
  if [[ -s "$unexpected_file" ]]; then
    fail "$label exposes unexpected UniFFI function symbol: $(head -n 1 "$unexpected_file")"
  fi
}

validate_symbol_text() {
  local label="$1"
  local text="$2"
  local functions checksums
  functions="$(mktemp "$work_dir/function-symbols.XXXXXX")"
  checksums="$(mktemp "$work_dir/checksum-symbols.XXXXXX")"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_fn_func_([a-z0-9_]+).*/\1/p' > "$functions"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_checksum_func_([a-z0-9_]+).*/\1/p' > "$checksums"
  compare_exact_symbol_set "$label" "$functions"
  compare_exact_symbol_set "$label checksum surface" "$checksums"
}

validate_checksum_guards() {
  local binding_path="$1"
  local language="$2"
  ruby -rjson -e '
    manifest = JSON.parse(File.read(ARGV.fetch(0)))
    language = ARGV.fetch(1)
    text = File.read(ARGV.fetch(2))
    expected = { manifest.fetch("version").fetch("name") => manifest.fetch("version").fetch("checksum") }
    manifest.fetch("functions").each { |entry| expected[entry.fetch("name")] = entry.fetch("checksum") }
    pattern = language == "Swift" ?
      /uniffi_editor_core_checksum_func_([a-z0-9_]+)\(\) != ([0-9]+)/ :
      /uniffi_editor_core_checksum_func_([a-z0-9_]+)\(\) != ([0-9]+)\.toShort\(\)/
    matches = text.scan(pattern).map { |name, checksum| [name, Integer(checksum)] }
    names = matches.map(&:first)
    abort "#{language} has duplicate checksum guards" unless names.uniq.length == names.length
    actual = matches.to_h
    expected.each do |name, checksum|
      abort "#{language} checksum mismatch for #{name}" unless actual[name] == checksum
    end
    unexpected = actual.keys - expected.keys
    abort "#{language} has unexpected checksum guard for #{unexpected.sort.first}" unless unexpected.empty?
  ' "$manifest_path" "$language" "$binding_path" || fail "checksum guard validation failed for $binding_path"
}

validate_abi_root() {
  local root="$1"
  local header="$root/ios/editor_coreFFI/editor_coreFFI.h"
  local swift="$root/ios/Generated_editor_core.swift"
  local kotlin="$root/rust/bindings/kotlin/uniffi/editor_core/editor_core.kt"
  require_file "$root" "ios/editor_coreFFI/editor_coreFFI.h"
  require_file "$root" "ios/Generated_editor_core.swift"
  require_file "$root" "rust/bindings/kotlin/uniffi/editor_core/editor_core.kt"
  validate_symbol_text "ABI header" "$(<"$header")"
  validate_checksum_guards "$swift" Swift
  validate_checksum_guards "$kotlin" Kotlin
}

compare_copy() {
  local source="$1"
  local copied="$2"
  local display_path="$3"
  [[ -f "$source" ]] || fail "copy source is missing: $display_path"
  [[ -f "$copied" ]] || fail "copy is missing: $display_path"
  cmp -s "$source" "$copied" || fail "copy mismatch: $display_path"
}

validate_copies() {
  local source_root="$1"
  local packed_root="$2"

  compare_copy "$source_root/rust/bindings/swift/editor_core.swift" "$source_root/ios/Generated_editor_core.swift" "ios/Generated_editor_core.swift"
  compare_copy "$source_root/ios/Generated_editor_core.swift" "$packed_root/ios/Generated_editor_core.swift" "ios/Generated_editor_core.swift"
  compare_copy "$source_root/rust/bindings/swift/editor_coreFFI.h" "$source_root/ios/editor_coreFFI/editor_coreFFI.h" "ios/editor_coreFFI/editor_coreFFI.h"
  compare_copy "$source_root/ios/editor_coreFFI/editor_coreFFI.h" "$packed_root/ios/editor_coreFFI/editor_coreFFI.h" "ios/editor_coreFFI/editor_coreFFI.h"
  compare_copy "$source_root/rust/bindings/swift/editor_coreFFI.modulemap" "$source_root/ios/editor_coreFFI/module.modulemap" "ios/editor_coreFFI/module.modulemap"
  compare_copy "$source_root/ios/editor_coreFFI/module.modulemap" "$packed_root/ios/editor_coreFFI/module.modulemap" "ios/editor_coreFFI/module.modulemap"
  compare_copy "$source_root/rust/bindings/kotlin/uniffi/editor_core/editor_core.kt" "$packed_root/rust/bindings/kotlin/uniffi/editor_core/editor_core.kt" "rust/bindings/kotlin/uniffi/editor_core/editor_core.kt"

  for archive in \
    "ios-arm64/libeditor_core.a" \
    "ios-arm64_x86_64-simulator/libeditor_core.a"; do
    compare_copy "$source_root/rust/ios/EditorCore.xcframework/$archive" "$source_root/ios/EditorCore.xcframework/$archive" "ios/EditorCore.xcframework/$archive"
    compare_copy "$source_root/ios/EditorCore.xcframework/$archive" "$packed_root/ios/EditorCore.xcframework/$archive" "ios/EditorCore.xcframework/$archive"
  done

  for abi in arm64-v8a armeabi-v7a x86 x86_64; do
    compare_copy "$source_root/rust/android/$abi/libeditor_core.so" "$packed_root/rust/android/$abi/libeditor_core.so" "rust/android/$abi/libeditor_core.so"
  done
}

validate_android_library() {
  local library_path="$1"
  local abi="$2"
  local file_output nm_output
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
  validate_symbol_text "Android $abi library" "$nm_output"
}

validate_archive_architectures() {
  local archive_path="$1"
  local expected_architectures="$2"
  local label="$3"
  local architecture_info actual_architectures normalized_architectures architecture thin_archive nm_output

  [[ -s "$archive_path" ]] || fail "$label archive is missing or empty"
  architecture_info="$(lipo -info "$archive_path" 2>&1)" || fail "$label is not a valid Mach-O archive: $architecture_info"
  actual_architectures="$(lipo -archs "$archive_path" 2>&1)" || fail "$label architectures cannot be read: $architecture_info"
  normalized_architectures="$(printf '%s\n' "$actual_architectures" | tr ' ' '\n' | sed '/^$/d' | sort | tr '\n' ' ' | sed 's/ $//')"
  [[ "$normalized_architectures" == "$expected_architectures" ]] || fail "$label must contain exactly [$expected_architectures], found [$normalized_architectures]"

  for architecture in $actual_architectures; do
    thin_archive="$work_dir/${label//[^[:alnum:]]/_}-$architecture.a"
    if [[ "$actual_architectures" == *" "* ]]; then
      lipo "$archive_path" -thin "$architecture" -output "$thin_archive" >/dev/null 2>&1 || fail "$label cannot extract its $architecture archive"
    else
      cp "$archive_path" "$thin_archive"
    fi
    file "$thin_archive" | grep -Fq 'current ar archive' || fail "$label $architecture slice is not a static archive"
    nm_output="$(nm -gU "$thin_archive" 2>&1)" || true
    validate_symbol_text "$label $architecture archive" "$nm_output"
  done
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
      { "BinaryPath" => "libeditor_core.a", "LibraryIdentifier" => "ios-arm64", "LibraryPath" => "libeditor_core.a", "SupportedArchitectures" => ["arm64"], "SupportedPlatform" => "ios" },
      { "BinaryPath" => "libeditor_core.a", "LibraryIdentifier" => "ios-arm64_x86_64-simulator", "LibraryPath" => "libeditor_core.a", "SupportedArchitectures" => ["arm64", "x86_64"], "SupportedPlatform" => "ios", "SupportedPlatformVariant" => "simulator" },
    ]
    abort "XCFramework AvailableLibraries must exactly describe the device and simulator slices" unless actual.sort_by { |entry| entry.fetch("LibraryIdentifier") } == expected.sort_by { |entry| entry.fetch("LibraryIdentifier") }
  ' "$plist_json" || fail "XCFramework slice metadata does not match the packaged libraries"
  validate_archive_architectures "$xcframework_dir/ios-arm64/libeditor_core.a" "arm64" "iOS device"
  validate_archive_architectures "$xcframework_dir/ios-arm64_x86_64-simulator/libeditor_core.a" "arm64 x86_64" "iOS simulator"
}

require_declaration_symbol() {
  local root="$1"
  local symbol="$2"
  find "$root/dist" -type f -name '*.d.ts' -exec grep -Fq "$symbol" {} + || fail "packed TypeScript declarations are missing $symbol"
}

validate_ios_consumer() {
  local root="$1"
  local ios_consumer="$work_dir/ios-consumer"
  local react_native_dir="$repo_root/example/node_modules/react-native"
  local expo_modules_core_dir="$repo_root/example/node_modules/expo-modules-core"
  local example_project_dir="$repo_root/example/ios/NativeEditorExample.xcodeproj"
  local workspace_path
  root="$(cd "$root" && pwd -P)"
  require_command pod
  require_command xcodebuild
  [[ -f "$react_native_dir/scripts/react_native_pods.rb" ]] || fail "local example React Native dependencies are missing; run npm install in example/"
  [[ -f "$expo_modules_core_dir/ExpoModulesCore.podspec" ]] || fail "local example ExpoModulesCore dependency is missing; run npm install in example/"
  [[ -d "$example_project_dir" ]] || fail "local example Xcode project is missing"

  mkdir -p "$ios_consumer"
  cp -R "$example_project_dir" "$ios_consumer/NativeEditorExample.xcodeproj"
  cat > "$ios_consumer/package.json" <<'JSON'
{
  "name": "packed-tarball-ios-consumer",
  "private": true
}
JSON
  mkdir -p "$ios_consumer/NativeEditorExample"
  cat > "$ios_consumer/NativeEditorExample/AppDelegate.swift" <<'SWIFT'
import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
  var window: UIWindow?
}
SWIFT
  cat > "$ios_consumer/Probe.swift" <<'SWIFT'
import ReactNativeProseEditor

func packedEditorCoreLinkProbe() {
  _ = editorV2Create(configJson: "{}", snapshotState: nil)
  _ = editorV2RenderUpdate(editorId: "1", mirrorScalarAnchor: nil, mirrorScalarHead: nil)
  _ = editorV2CollaborationTick(editorId: "1", nowMillis: "0")
  _ = editorV2CollaborationDetach(editorId: "1")
  _ = editorV2CollaborationReattach(editorId: "1")
}
SWIFT
  ruby -e '
    path = ARGV.fetch(0)
    project = File.read(path)
    project.sub!("/* End PBXBuildFile section */", "\t\tA19C00000000000000000002 /* Probe.swift in Sources */ = {isa = PBXBuildFile; fileRef = A19C000000000000000000001 /* Probe.swift */; };\n/* End PBXBuildFile section */")
    project.sub!("/* End PBXFileReference section */", "\t\tA19C000000000000000000001 /* Probe.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = Probe.swift; sourceTree = \"<group>\"; };\n/* End PBXFileReference section */")
    project.sub!("F11748412D0307B40044C1D9 /* AppDelegate.swift */,", "F11748412D0307B40044C1D9 /* AppDelegate.swift */,\n\t\t\t\tA19C000000000000000000001 /* Probe.swift */,")
    project.sub!("F11748422D0307B40044C1D9 /* AppDelegate.swift in Sources */,", "F11748422D0307B40044C1D9 /* AppDelegate.swift in Sources */,\n\t\t\t\tA19C00000000000000000002 /* Probe.swift in Sources */,")
    project.gsub!("\t\t\t\tE8503B04C124877981CF4A94 /* [Expo] Configure project */,\n", "")
    project.gsub!("\t\t\t\t00DD1BFF1BD5951E006B06BC /* Bundle React Native code and images */,\n", "")
    abort "could not add the Swift link probe to the consumer project" unless project.include?("Probe.swift in Sources")
    File.write(path, project)
  ' "$ios_consumer/NativeEditorExample.xcodeproj/project.pbxproj"
  cat > "$ios_consumer/Podfile" <<'RUBY'
require 'pathname'
require File.join(ENV.fetch('PACKED_REACT_NATIVE_DIR'), 'scripts', 'react_native_pods')

react_native_path = Pathname.new(ENV.fetch('PACKED_REACT_NATIVE_DIR')).relative_path_from(Pathname.new(__dir__)).to_s
platform :ios, '15.1'
prepare_react_native_project!
project File.join(__dir__, 'NativeEditorExample.xcodeproj')

target 'NativeEditorExample' do
  pod 'ExpoModulesCore', :path => ENV.fetch('PACKED_EXPO_MODULES_CORE_DIR')
  pod 'ReactNativeProseEditor', :path => ENV.fetch('PACKED_EDITOR_IOS_DIR')
  use_react_native!(
    :path => react_native_path,
    :app_path => __dir__,
    :hermes_enabled => false,
    :fabric_enabled => false,
  )
end
RUBY
  (
    cd "$ios_consumer"
      PACKED_EDITOR_IOS_DIR="$root/ios" \
      PACKED_EXPO_MODULES_CORE_DIR="$expo_modules_core_dir" \
      PACKED_REACT_NATIVE_DIR="$react_native_dir" \
      CP_CACHE_DIR="$cocoapods_cache_dir" \
      CP_HOME_DIR="$cocoapods_home_dir" \
      pod install --no-repo-update
  ) || fail "iOS consumer pod install failed"
  [[ -f "$ios_consumer/Podfile.lock" ]] || fail "CocoaPods did not produce Podfile.lock"
  workspace_path="$ios_consumer/NativeEditorExample.xcworkspace"
  rm -rf "$workspace_path"
  cp -R "$repo_root/example/ios/NativeEditorExample.xcworkspace" "$workspace_path"
  (
    cd "$ios_consumer"
    xcodebuild -workspace NativeEditorExample.xcworkspace -scheme NativeEditorExample -configuration Debug -sdk iphonesimulator -derivedDataPath "$work_dir/ios-derived-data" CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO build
  ) || fail "iOS consumer xcodebuild failed"
}

validate_android_consumer() {
  local root="$1"
  local android_consumer="$work_dir/android-consumer"
  local example_android_dir="$repo_root/example/android"
  local example_node_modules="$repo_root/example/node_modules"
  local apk
  root="$(cd "$root" && pwd -P)"
  require_command unzip
  [[ -x "$example_android_dir/gradlew" ]] || fail "local example Gradle wrapper is missing"
  [[ -d "$example_node_modules" ]] || fail "local example Android dependencies are missing; run npm install in example/"

  mkdir -p "$android_consumer"
  cp -R "$example_android_dir" "$android_consumer/android"
  cp "$repo_root/example/package.json" "$android_consumer/package.json"
  ln -s "$example_node_modules" "$android_consumer/node_modules"
  cat >> "$android_consumer/android/settings.gradle" <<'GRADLE'

include ':packedEditorCore'
project(':packedEditorCore').projectDir = new File(System.getenv('PACKED_EDITOR_ANDROID_DIR'))
GRADLE
  cat >> "$android_consumer/android/app/build.gradle" <<'GRADLE'

dependencies {
    implementation project(':packedEditorCore')
}
GRADLE
  mkdir -p "$android_consumer/android/app/src/main/java/com/apollohg/nativeeditorexample"
  cat > "$android_consumer/android/app/src/main/java/com/apollohg/nativeeditorexample/PackedEditorCoreProbe.kt" <<'KOTLIN'
package com.apollohg.nativeeditorexample

import uniffi.editor_core.*

object PackedEditorCoreProbe {
  fun referenceFinalApi() {
    editorV2Create("{}", null)
    editorV2RenderUpdate("1", null, null)
    editorV2CollaborationTick("1", "0")
    editorV2CollaborationDetach("1")
    editorV2CollaborationReattach("1")
  }
}
KOTLIN
  (
    cd "$android_consumer/android"
    PACKED_EDITOR_ANDROID_DIR="$root/android" \
      NODE_PATH="$example_node_modules" \
      GRADLE_USER_HOME="$work_dir/gradle-home" \
      ./gradlew app:assembleDebug -x lint -x test --no-daemon
  ) || fail "Android consumer assembleDebug failed"
  apk="$android_consumer/android/app/build/outputs/apk/debug/app-debug.apk"
  [[ -s "$apk" ]] || fail "Android consumer did not produce app-debug.apk"
  for abi in arm64-v8a armeabi-v7a x86 x86_64; do
    unzip -Z1 "$apk" | grep -Fxq "lib/$abi/libeditor_core.so" || fail "Android consumer package is missing libeditor_core.so for $abi"
  done
}

validate_package_entries() {
  local root="$1"
  require_file "$root" "dist/index.js"
  require_file "$root" "dist/index.d.ts"
  require_declaration_symbol "$root" "NativeEditorBoundaryError"
  require_declaration_symbol "$root" "resourceLimits"
  require_declaration_symbol "$root" "requestTimeoutMs"
}

case "${1:-}" in
  --validate-package-entries)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-package-entries PATH"
    validate_package_entries "$2"
    echo "Packed JavaScript entry-point validation passed."
    exit 0
    ;;
  --validate-abi-root)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-abi-root ROOT"
    require_command ruby
    validate_abi_root "$2"
    echo "Exact ABI and binding checksum validation passed."
    exit 0
    ;;
  --validate-copies)
    [[ "$#" == "3" ]] || fail "usage: $0 --validate-copies SOURCE_ROOT PACKED_ROOT"
    validate_copies "$2" "$3"
    echo "Generated and native artifact copy validation passed."
    exit 0
    ;;
  --validate-xcframework)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-xcframework PATH"
    require_command ruby; require_command plutil; require_command lipo; require_command file; require_command nm
    validate_xcframework "$2"
    echo "XCFramework metadata and exact static archive ABI validation passed."
    exit 0
    ;;
  --validate-android-library)
    [[ "$#" == "3" ]] || fail "usage: $0 --validate-android-library PATH ABI"
    require_command ruby; require_command file; require_command nm
    validate_android_library "$2" "$3"
    echo "Android library machine type and exact ABI validation passed."
    exit 0
    ;;
  --validate-ios-consumer)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-ios-consumer PACKED_ROOT"
    validate_ios_consumer "$2"
    echo "iOS packed consumer compiles and links the final API."
    exit 0
    ;;
  --validate-android-consumer)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-android-consumer PACKED_ROOT"
    validate_android_consumer "$2"
    echo "Android packed consumer compiles and packages all ABI libraries."
    exit 0
    ;;
  "") ;;
  *) fail "unknown argument: $1" ;;
esac

require_command npm
require_command tar
require_command ruby
require_command pod
require_command xcodebuild
require_command unzip
require_command plutil
require_command lipo
require_command file
require_command nm

mkdir -p "$pack_cache_dir"
echo "==> Packing the publishable npm artifact..."
pack_json="$work_dir/npm-pack.json"
(
  cd "$repo_root"
  npm_config_cache="$pack_cache_dir" npm_config_logs_dir="$pack_cache_dir/logs" npm pack --ignore-scripts --json --pack-destination "$work_dir" > "$pack_json"
)
tarball_name="$(ruby -rjson -e 'entries = JSON.parse(File.read(ARGV.fetch(0))); abort "npm pack returned no artifact" unless entries.length == 1; puts entries.fetch(0).fetch("filename")' "$pack_json")"
tarball_path="$work_dir/$tarball_name"
[[ -f "$tarball_path" ]] || fail "npm pack did not create $tarball_name"
tar -xzf "$tarball_path" -C "$work_dir"
[[ -d "$package_dir" ]] || fail "npm tarball does not contain the canonical package/ root"

validate_package_entries "$package_dir"
validate_abi_root "$repo_root"
validate_abi_root "$package_dir"
validate_copies "$repo_root" "$package_dir"
validate_xcframework "$package_dir/ios/EditorCore.xcframework"
for abi in arm64-v8a armeabi-v7a x86 x86_64; do
  validate_android_library "$package_dir/rust/android/$abi/libeditor_core.so" "$abi"
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
  abort "podspec must vend exactly EditorCore.xcframework" unless Array(spec.fetch("vendored_frameworks")) == ["EditorCore.xcframework"]
' "$podspec_json" || fail "packed podspec does not unconditionally vend EditorCore.xcframework"

validate_ios_consumer "$package_dir"
validate_android_consumer "$package_dir"
echo "==> Packed npm artifact, exact ABI, and real consumer validation passed."
