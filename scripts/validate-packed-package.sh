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
consumer_gradle_home="${GRADLE_USER_HOME:-$work_dir/gradle-home}"
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

ios_deployment_target() {
  ruby -e '
    text = File.read(File.join(ARGV.fetch(0), "ReactNativeProseEditor.podspec"))
    match = text.match(/s\.platforms\s*=\s*\{\s*:ios\s*=>\s*["\x27]([0-9]+(?:\.[0-9]+){1,2})["\x27]\s*\}/)
    abort "podspec must declare one literal iOS deployment target" unless match
    puts match[1]
  ' "$1" || fail "could not resolve the podspec iOS deployment target"
}

resolve_single_artifact() {
  local directory="$1"
  local pattern="$2"
  local label="$3"
  local matches=()
  local path
  while IFS= read -r path; do
    matches+=("$path")
  done < <(find "$directory" -maxdepth 1 -type f -name "$pattern" -print)
  [[ "${#matches[@]}" -eq 1 ]] || fail "expected exactly one $label artifact under $directory"
  printf '%s\n' "${matches[0]}"
}

manifest_entries() {
  ruby -rjson -e '
    manifest = JSON.parse(File.read(ARGV.fetch(0)))
    editor_functions = manifest.fetch("functions")
    abort "package ABI manifest must contain exactly 35 editor_v2 functions" unless editor_functions.length == 35
    editor_names = editor_functions.map { |entry| entry.fetch("name") }
    abort "package ABI manifest contains duplicate editor function names" unless editor_names.uniq.length == editor_names.length
    abort "package ABI manifest contains a non-v2 editor function" unless editor_names.all? { |name| name.start_with?("editor_v2_") }

    viewer = manifest.fetch("viewer")
    viewer_functions = viewer.fetch("functions")
    viewer_objects = viewer.fetch("objects")
    abort "package ABI manifest must contain exactly one viewer function" unless viewer_functions.length == 1
    abort "package ABI manifest must contain viewer_compile" unless viewer_functions.fetch(0).fetch("name") == "viewer_compile"
    abort "package ABI manifest must contain exactly one ViewerCompiledDocument object" unless viewer_objects.length == 1
    viewer_object = viewer_objects.fetch(0)
    abort "package ABI manifest must contain viewercompileddocument" unless viewer_object.fetch("name") == "viewercompileddocument"
    lifecycle = viewer_object.fetch("lifecycle")
    abort "package ABI manifest ViewerCompiledDocument lifecycle must be clone/free" unless lifecycle.sort == ["clone", "free"]
    methods = viewer_object.fetch("methods")
    method_names = methods.map { |entry| entry.fetch("name") }
    abort "package ABI manifest ViewerCompiledDocument methods are duplicate" unless method_names.uniq.length == method_names.length
    abort "package ABI manifest ViewerCompiledDocument methods are incomplete" unless method_names.sort == %w[elements is_empty preferred_text_block_name retained_bytes_decimal semantic_key trailing_empty_text_block_count]

    version = manifest.fetch("version")
    puts ["function", version.fetch("name"), version.fetch("checksum")].join("\t")
    editor_functions.sort_by { |entry| entry.fetch("name") }.each do |entry|
      puts ["function", entry.fetch("name"), entry.fetch("checksum")].join("\t")
    end
    viewer_functions.each do |entry|
      puts ["function", entry.fetch("name"), entry.fetch("checksum")].join("\t")
    end
    lifecycle.sort.each do |operation|
      puts ["lifecycle", "#{operation}_#{viewer_object.fetch("name")}", ""].join("\t")
    end
    methods.sort_by { |entry| entry.fetch("name") }.each do |entry|
      puts ["method", "#{viewer_object.fetch("name")}_#{entry.fetch("name")}", entry.fetch("checksum")].join("\t")
    end
  ' "$manifest_path"
}

expected_symbol_names() {
  local kind="$1"
  manifest_entries | awk -F '\t' -v kind="$kind" '$1 == kind { print $2 }' | sort
}

compare_exact_symbol_set() {
  local label="$1"
  local actual_file="$2"
  local kind="$3"
  local expected_file="$work_dir/expected-${kind}-symbols.txt"
  local missing_file unexpected_file

  expected_symbol_names "$kind" > "$expected_file"
  sort -u "$actual_file" -o "$actual_file"

  missing_file="$(mktemp "$work_dir/missing-symbols.XXXXXX")"
  unexpected_file="$(mktemp "$work_dir/unexpected-symbols.XXXXXX")"
  comm -23 "$expected_file" "$actual_file" > "$missing_file"
  comm -13 "$expected_file" "$actual_file" > "$unexpected_file"
  if [[ -s "$missing_file" ]]; then
    fail "$label is missing expected function symbol: $(head -n 1 "$missing_file")"
  fi
  if [[ -s "$unexpected_file" ]]; then
    fail "$label exposes unexpected UniFFI ${kind} symbol: $(head -n 1 "$unexpected_file")"
  fi
}

validate_symbol_text() {
  local label="$1"
  local text="$2"
  local functions methods lifecycle checksum_functions checksum_methods
  functions="$(mktemp "$work_dir/function-symbols.XXXXXX")"
  methods="$(mktemp "$work_dir/method-symbols.XXXXXX")"
  lifecycle="$(mktemp "$work_dir/lifecycle-symbols.XXXXXX")"
  checksum_functions="$(mktemp "$work_dir/checksum-function-symbols.XXXXXX")"
  checksum_methods="$(mktemp "$work_dir/checksum-method-symbols.XXXXXX")"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_fn_func_([a-z0-9_]+).*/\1/p' > "$functions"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_fn_method_([a-z0-9_]+).*/\1/p' > "$methods"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_fn_(clone|free)_([a-z0-9_]+).*/\1_\2/p' > "$lifecycle"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_checksum_func_([a-z0-9_]+).*/\1/p' > "$checksum_functions"
  printf '%s\n' "$text" | sed -nE 's/.*uniffi_editor_core_checksum_method_([a-z0-9_]+).*/\1/p' > "$checksum_methods"
  compare_exact_symbol_set "$label" "$functions" function
  compare_exact_symbol_set "$label object methods" "$methods" method
  compare_exact_symbol_set "$label object lifecycle" "$lifecycle" lifecycle
  compare_exact_symbol_set "$label checksum surface" "$checksum_functions" function
  compare_exact_symbol_set "$label object-method checksum surface" "$checksum_methods" method
}

validate_checksum_guards() {
  local binding_path="$1"
  local language="$2"
  ruby -rjson -e '
    manifest = JSON.parse(File.read(ARGV.fetch(0)))
    language = ARGV.fetch(1)
    text = File.read(ARGV.fetch(2))
    viewer = manifest.fetch("viewer")
    viewer_object = viewer.fetch("objects").fetch(0)
    expected = { manifest.fetch("version").fetch("name") => manifest.fetch("version").fetch("checksum") }
    manifest.fetch("functions").each { |entry| expected[entry.fetch("name")] = entry.fetch("checksum") }
    viewer.fetch("functions").each { |entry| expected[entry.fetch("name")] = entry.fetch("checksum") }
    viewer_object.fetch("methods").each do |entry|
      expected["#{viewer_object.fetch("name")}_#{entry.fetch("name")}"] = entry.fetch("checksum")
    end
    pattern = language == "Swift" ?
      /uniffi_editor_core_checksum_(?:func|method)_([a-z0-9_]+)\(\) != ([0-9]+)/ :
      /uniffi_editor_core_checksum_(?:func|method)_([a-z0-9_]+)\(\) != ([0-9]+)\.toShort\(\)/
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
  validate_symbol_text "Swift binding" "$(<"$swift")"
  validate_symbol_text "Kotlin binding" "$(<"$kotlin")"
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
  ruby "$repo_root/scripts/validate-uniffi-checksum-values.rb" \
    --manifest "$manifest_path" \
    --label "Android $abi library" \
    --elf "$abi" "$library_path" || fail "Android $abi library native checksum value validation failed"
}

validate_archive_architectures() {
  local archive_path="$1"
  local expected_architectures="$2"
  local label="$3"
  local deployment_target="$4"
  local architecture_info actual_architectures normalized_architectures architecture thin_archive
  local extracted_objects_dir
  local load_commands
  local architecture_nm_output architecture_nm_status unexpected_nm_lines

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
    ruby "$repo_root/scripts/validate-uniffi-checksum-values.rb" \
      --manifest "$manifest_path" \
      --label "$label $architecture archive" \
      --macho-archive "$architecture" "$thin_archive" || fail "$label $architecture archive native checksum value validation failed"
    extracted_objects_dir="$work_dir/${label//[^[:alnum:]]/_}-$architecture-objects"
    mkdir -p "$extracted_objects_dir"
    (
      cd "$extracted_objects_dir"
      ar -x "$thin_archive"
      # Apple nm can reject Rust objects whose embedded bitcode is newer than
      # its LLVM reader. otool still proves each member has a Mach-O header.
      for object in ./*.o; do
        otool -hv "$object" 2>&1 | grep -q "Mach header" || exit 1
      done
    ) || fail "$label $architecture archive contains an unreadable Mach-O object member"
    load_commands="$work_dir/${label//[^[:alnum:]]/_}-$architecture-load-commands.txt"
    otool -l "$thin_archive" > "$load_commands" \
      || fail "$label $architecture archive load commands cannot be read"
    ruby -e '
      target = ARGV.fetch(0).split(".").map(&:to_i)
      target.fill(0, target.length...3)
      label = ARGV.fetch(1)
      objects = []
      versions = {}
      current = nil
      pending = nil
      File.foreach(ARGV.fetch(2)) do |line|
        if (match = line.match(/\.a\((.+)\):\s*$/))
          current = match[1]
          objects << current
          pending = nil
          next
        end
        pending = :minos if line.match?(/^\s*cmd LC_BUILD_VERSION\s*$/)
        pending = :version if line.match?(/^\s*cmd LC_VERSION_MIN_IPHONEOS\s*$/)
        next unless pending && current
        field = pending == :minos ? "minos" : "version"
        next unless (match = line.match(/^\s*#{field}\s+([0-9]+(?:\.[0-9]+){0,2})/))
        value = match[1]
        parts = value.split(".").map(&:to_i)
        parts.fill(0, parts.length...3)
        if (parts <=> target) == 1
          abort "#{label} object #{current} requires iOS #{value}, above #{ARGV.fetch(0)}"
        end
        versions[current] = value
        pending = nil
      end
      abort "#{label} archive contains no Mach-O object members" if objects.empty?
      missing = objects.uniq.reject { |object| versions.key?(object) }
      abort "#{label} object #{missing.first} has no iOS deployment load command" unless missing.empty?
    ' "$deployment_target" "$label $architecture" "$load_commands" \
      || fail "$label $architecture archive deployment target validation failed"
    architecture_nm_status=0
    architecture_nm_output="$(nm -gU "$thin_archive" 2>&1)" || architecture_nm_status=$?
    if [[ "$architecture_nm_status" -ne 0 ]]; then
      # Tolerate only the documented producer/reader skew and no-symbol notes;
      # all other Apple nm errors invalidate the archive.
      unexpected_nm_lines="$(printf '%s\n' "$architecture_nm_output" | grep -E '(nm: error: |: no symbols$)' | grep -v 'Unknown attribute kind' | grep -v ': no symbols$' || true)"
      [[ -z "$unexpected_nm_lines" ]] || fail "$label $architecture archive symbols cannot be read: $architecture_nm_output"
    fi
    validate_symbol_text "$label $architecture archive" "$architecture_nm_output"
  done
}

validate_xcframework() {
  local xcframework_dir="$1"
  local deployment_target="$2"
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
  validate_archive_architectures "$xcframework_dir/ios-arm64/libeditor_core.a" "arm64" "iOS device" "$deployment_target"
  validate_archive_architectures "$xcframework_dir/ios-arm64_x86_64-simulator/libeditor_core.a" "arm64 x86_64" "iOS simulator" "$deployment_target"
}

require_declaration_symbol() {
  local root="$1"
  local symbol="$2"
  find "$root/dist" -type f -name '*.d.ts' -exec grep -Fq "$symbol" {} + || fail "packed TypeScript declarations are missing $symbol"
}

reject_dist_symbol() {
  local root="$1"
  local symbol="$2"
  if find "$root/dist" -type f \( -name '*.js' -o -name '*.d.ts' \) -exec grep -Fq "$symbol" {} +; then
    fail "packed JavaScript surface still contains obsolete collaboration API $symbol"
  fi
}

validate_ios_consumer() {
  local root="$1"
  local tarball_path="$2"
  local ios_consumer="$work_dir/ios-consumer"
  local ios_project="$ios_consumer/ios"
  local packed_editor_dir="$ios_consumer/node_modules/react-native-rich-text-editor"
  local react_native_dir="$repo_root/example/node_modules/react-native"
  local expo_dir="$repo_root/example/node_modules/expo"
  local react_native_dependencies_archive
  local react_native_core_archive
  local hermes_archive
  local workspace_path packed_editor_package_json resolved_package_json resolved_package_dir
  react_native_dependencies_archive="$(resolve_single_artifact "$repo_root/example/ios/Pods/ReactNativeDependencies-artifacts" 'reactnative-dependencies-*-debug.tar.gz' 'ReactNativeDependencies debug')"
  react_native_core_archive="$(resolve_single_artifact "$repo_root/example/ios/Pods/ReactNativeCore-artifacts" 'reactnative-core-*-debug.tar.gz' 'ReactNativeCore debug')"
  hermes_archive="$(resolve_single_artifact "$repo_root/example/ios/Pods/hermes-engine-artifacts" 'hermes-ios-*-debug.tar.gz' 'Hermes iOS debug')"
  root="$(cd "$root" && pwd -P)"
  tarball_path="$(cd "$(dirname "$tarball_path")" && pwd -P)/$(basename "$tarball_path")"
  [[ -f "$tarball_path" ]] || fail "iOS packed consumer tarball is missing: $tarball_path"
  require_command pod
  require_command xcodebuild
  [[ -f "$react_native_dir/scripts/react_native_pods.rb" ]] || fail "local example React Native dependencies are missing; run npm install in example/"
  [[ -f "$expo_dir/scripts/autolinking.rb" ]] || fail "local example Expo autolinking dependency is missing; run npm install in example/"
  [[ -s "$react_native_dependencies_archive" ]] || fail "local ReactNativeDependencies artifact is missing from example/ios/Pods"
  [[ -s "$react_native_core_archive" ]] || fail "local ReactNativeCore artifact is missing from example/ios/Pods"
  [[ -s "$hermes_archive" ]] || fail "local Hermes artifact is missing from example/ios/Pods"

  mkdir -p "$ios_consumer"
  # Model a real package consumer: npm installs the generated tarball and
  # package.json supplies its RN peers from the known-good local installation.
  # Keeping each peer as a file dependency makes the install network-free while
  # preserving the dependency graph Expo inspects for autolinking and codegen.
  # This is intentionally a fresh UIKit target. Reusing the Expo example
  # project pulls app-only bundle and Hermes build phases into this package
  # consumer instead of proving the pod's own integration boundary.
  mkdir -p "$ios_project/PackedConsumer.xcodeproj"
  mkdir -p "$ios_project/PackedConsumer.xcodeproj/project.xcworkspace"
  cat > "$ios_project/PackedConsumer.xcodeproj/project.xcworkspace/contents.xcworkspacedata" <<'WORKSPACE'
<?xml version="1.0" encoding="UTF-8"?>
<Workspace version = "1.0">
  <FileRef location = "self:"></FileRef>
</Workspace>
WORKSPACE
  # Keep this project deliberately self-contained: the system Ruby available
  # on macOS does not necessarily include CocoaPods' xcodeproj gem.
  cat > "$ios_project/PackedConsumer.xcodeproj/project.pbxproj" <<'PBXPROJ'
// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {};
	objectVersion = 56;
	objects = {
		A00000000000000000000001 /* AppDelegate.swift in Sources */ = {isa = PBXBuildFile; fileRef = A00000000000000000000011 /* AppDelegate.swift */; };
		A00000000000000000000002 /* Probe.swift in Sources */ = {isa = PBXBuildFile; fileRef = A00000000000000000000012 /* Probe.swift */; };
		A00000000000000000000011 /* AppDelegate.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = AppDelegate.swift; sourceTree = "<group>"; };
		A00000000000000000000012 /* Probe.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = Probe.swift; sourceTree = "<group>"; };
		A00000000000000000000013 /* PackedConsumer.app */ = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = PackedConsumer.app; sourceTree = BUILT_PRODUCTS_DIR; };
		A00000000000000000000020 /* PackedConsumer */ = {isa = PBXGroup; children = (A00000000000000000000011 /* AppDelegate.swift */, A00000000000000000000012 /* Probe.swift */,); path = PackedConsumer; sourceTree = "<group>"; };
		A00000000000000000000021 = {isa = PBXGroup; children = (A00000000000000000000020 /* PackedConsumer */, A00000000000000000000022 /* Products */,); sourceTree = "<group>"; };
		A00000000000000000000022 /* Products */ = {isa = PBXGroup; children = (A00000000000000000000013 /* PackedConsumer.app */,); name = Products; sourceTree = "<group>"; };
		A00000000000000000000030 /* Frameworks */ = {isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0; };
		A00000000000000000000031 /* Resources */ = {isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0; };
		A00000000000000000000032 /* Sources */ = {isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = (A00000000000000000000001 /* AppDelegate.swift in Sources */, A00000000000000000000002 /* Probe.swift in Sources */,); runOnlyForDeploymentPostprocessing = 0; };
		A00000000000000000000040 /* PackedConsumer */ = {isa = PBXNativeTarget; buildConfigurationList = A00000000000000000000050 /* Build configuration list for PBXNativeTarget \"PackedConsumer\" */; buildPhases = (A00000000000000000000032 /* Sources */, A00000000000000000000030 /* Frameworks */, A00000000000000000000031 /* Resources */,); buildRules = (); dependencies = (); name = PackedConsumer; productName = PackedConsumer; productReference = A00000000000000000000013 /* PackedConsumer.app */; productType = "com.apple.product-type.application"; };
		A00000000000000000000041 /* Project object */ = {isa = PBXProject; attributes = { LastUpgradeCheck = 1700; }; buildConfigurationList = A00000000000000000000051 /* Build configuration list for PBXProject \"PackedConsumer\" */; compatibilityVersion = "Xcode 14.0"; developmentRegion = en; hasScannedForEncodings = 0; knownRegions = (en, Base,); mainGroup = A00000000000000000000021; productRefGroup = A00000000000000000000022 /* Products */; projectDirPath = ""; projectRoot = ""; targets = (A00000000000000000000040 /* PackedConsumer */,); };
		A00000000000000000000060 /* Debug */ = {isa = XCBuildConfiguration; buildSettings = { CLANG_ENABLE_EXPLICIT_MODULES = NO; CLANG_ENABLE_MODULES = YES; IPHONEOS_DEPLOYMENT_TARGET = 16.4; SDKROOT = iphoneos; SWIFT_ENABLE_EXPLICIT_MODULES = NO; }; name = Debug; };
		A00000000000000000000061 /* Release */ = {isa = XCBuildConfiguration; buildSettings = { CLANG_ENABLE_EXPLICIT_MODULES = NO; CLANG_ENABLE_MODULES = YES; IPHONEOS_DEPLOYMENT_TARGET = 16.4; SDKROOT = iphoneos; SWIFT_ENABLE_EXPLICIT_MODULES = NO; }; name = Release; };
		A00000000000000000000062 /* Debug */ = {isa = XCBuildConfiguration; buildSettings = { CLANG_ENABLE_EXPLICIT_MODULES = NO; CODE_SIGN_STYLE = Automatic; GENERATE_INFOPLIST_FILE = YES; IPHONEOS_DEPLOYMENT_TARGET = 16.4; LD_RUNPATH_SEARCH_PATHS = ("$(inherited)", "@executable_path/Frameworks",); PRODUCT_BUNDLE_IDENTIFIER = dev.nativeeditor.packedconsumer; PRODUCT_NAME = PackedConsumer; SWIFT_ENABLE_EXPLICIT_MODULES = NO; SWIFT_VERSION = 5.0; TARGETED_DEVICE_FAMILY = "1,2"; }; name = Debug; };
		A00000000000000000000063 /* Release */ = {isa = XCBuildConfiguration; buildSettings = { CLANG_ENABLE_EXPLICIT_MODULES = NO; CODE_SIGN_STYLE = Automatic; GENERATE_INFOPLIST_FILE = YES; IPHONEOS_DEPLOYMENT_TARGET = 16.4; LD_RUNPATH_SEARCH_PATHS = ("$(inherited)", "@executable_path/Frameworks",); PRODUCT_BUNDLE_IDENTIFIER = dev.nativeeditor.packedconsumer; PRODUCT_NAME = PackedConsumer; SWIFT_ENABLE_EXPLICIT_MODULES = NO; SWIFT_VERSION = 5.0; TARGETED_DEVICE_FAMILY = "1,2"; }; name = Release; };
		A00000000000000000000050 /* Build configuration list for PBXNativeTarget \"PackedConsumer\" */ = {isa = XCConfigurationList; buildConfigurations = (A00000000000000000000062 /* Debug */, A00000000000000000000063 /* Release */,); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release; };
		A00000000000000000000051 /* Build configuration list for PBXProject \"PackedConsumer\" */ = {isa = XCConfigurationList; buildConfigurations = (A00000000000000000000060 /* Debug */, A00000000000000000000061 /* Release */,); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release; };
	};
	rootObject = A00000000000000000000041 /* Project object */;
}
PBXPROJ
  cat > "$ios_consumer/package.json" <<JSON
{
  "name": "packed-tarball-ios-consumer",
  "private": true,
  "dependencies": {
    "react-native-rich-text-editor": "file:$tarball_path",
    "expo": "file:$expo_dir",
    "react": "file:$repo_root/example/node_modules/react",
    "react-native": "file:$react_native_dir"
  }
}
JSON
  (
    cd "$ios_consumer"
    npm_config_cache="$pack_cache_dir" npm_config_logs_dir="$pack_cache_dir/logs" \
      npm install --ignore-scripts --no-audit --no-fund --offline --package-lock=false --legacy-peer-deps
  ) || fail "iOS packed consumer npm install failed"
  [[ -f "$packed_editor_dir/ReactNativeProseEditor.podspec" ]] || \
    fail "iOS packed consumer dependency is missing the root podspec"
  packed_editor_package_json="$(cd "$packed_editor_dir" && pwd -P)/package.json"
  resolved_package_json="$(
    cd "$ios_consumer"
    node --no-warnings --print "require.resolve('react-native-rich-text-editor/package.json')"
  )" || fail "iOS packed consumer cannot resolve react-native-rich-text-editor"
  [[ "$resolved_package_json" == "$packed_editor_package_json" ]] || \
    fail "iOS packed consumer resolved editor package outside consumer node_modules: $resolved_package_json"
  resolved_package_dir="$(cd "$(dirname "$resolved_package_json")" && pwd -P)"
  case "$resolved_package_dir" in
    "$repo_root"|"$repo_root"/*|"$root"|"$root"/*)
      fail "iOS packed consumer resolved editor package from repository or extraction staging: $resolved_package_dir"
      ;;
  esac
  mkdir -p "$ios_project/PackedConsumer"
  cat > "$ios_project/PackedConsumer/AppDelegate.swift" <<'SWIFT'
import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
  var window: UIWindow?
}
SWIFT
  cat > "$ios_project/PackedConsumer/Probe.swift" <<'SWIFT'
internal import ReactNativeProseEditor

func packedEditorCoreLinkProbe() {
  _ = editorV2Create(configJson: "{}", snapshotState: nil)
  _ = editorV2RenderUpdate(editorId: "1", mirrorScalarAnchor: nil, mirrorScalarHead: nil)
  _ = editorV2CollaborationDrive(editorId: "1", nowMillis: "0")
  _ = editorV2CollaborationLeaseOutbound(editorId: "1", generation: "1")
  _ = editorV2CollaborationAckOutbound(editorId: "1", generation: "1", leaseId: "1")
  _ = editorV2CollaborationNackOutbound(editorId: "1", generation: "1", leaseId: "1")
  _ = editorV2CollaborationDetach(editorId: "1")
  _ = editorV2CollaborationReattach(editorId: "1")
}
SWIFT
  cp "$react_native_dependencies_archive" "$ios_project/react-native-dependencies.tar.gz"
  cp "$react_native_core_archive" "$ios_project/react-native-core.tar.gz"
  cp "$hermes_archive" "$ios_project/hermes-ios-debug.tar.gz"
  cat > "$ios_project/Podfile" <<'RUBY'
require 'json'
require 'pathname'
require File.join(File.dirname(`node --print "require.resolve('expo/package.json')"`), 'scripts', 'autolinking')
require File.join(File.dirname(`node --print "require.resolve('react-native/package.json')"`), 'scripts', 'react_native_pods')

react_native_root = ENV.fetch('PACKED_REACT_NATIVE_DIR')
ENV['RCT_USE_RN_DEP'] ||= '1'
ENV['RCT_USE_PREBUILT_RNCORE'] ||= '1'
platform :ios, '16.4'
project File.join(__dir__, 'PackedConsumer.xcodeproj')
prepare_react_native_project!

target 'PackedConsumer' do
  use_expo_modules!

  config_command = [
    'node',
    '--no-warnings',
    '--eval',
    'require(\'expo/bin/autolinking\')',
    'expo-modules-autolinking',
    'react-native-config',
    '--json',
    '--platform',
    'ios'
  ]
  config = use_native_modules!(config_command)

  # Use Expo's supported autolinking configuration for the complete RN and
  # Expo pod graph. The target remains a fresh UIKit app and does not inherit
  # the example's app phases.
  use_react_native!(
    :path => config[:reactNativePath],
    :app_path => "#{Pod::Config.instance.installation_root}/..",
    :hermes_enabled => true,
  )

  post_install do |installer|
    # Keep RN's standard post-install adjustments for the supported pod graph.
    react_native_post_install(installer, config[:reactNativePath], :mac_catalyst_enabled => false)
    # Xcode evaluates this setting from the Pods project while precompiling
    # imported Swift modules, so set it at that project scope as well as each
    # generated target. This matches the known-good example configuration.
    installer.pods_project.build_configurations.each do |configuration|
      configuration.build_settings['CLANG_ENABLE_EXPLICIT_MODULES'] = 'NO'
      configuration.build_settings['SWIFT_ENABLE_EXPLICIT_MODULES'] = 'NO'
    end
    installer.pods_project.targets.each do |target|
      target.build_configurations.each do |configuration|
        # Pod build phases execute from Pods/, whereas use_react_native! receives
        # a path relative to the consumer root. Give those phases the absolute
        # source checkout so RN's generated-spec integrity checks resolve it.
        configuration.build_settings['REACT_NATIVE_PATH'] = react_native_root
        # Match the example's Xcode 26 setting without relaxing Clang diagnostics.
        configuration.build_settings['CLANG_ENABLE_EXPLICIT_MODULES'] = 'NO'
        configuration.build_settings['SWIFT_ENABLE_EXPLICIT_MODULES'] = 'NO'
        # RCT_REMOTE_PROFILE is present in Debug's React Core headers. Every pod
        # target that imports one of those headers must see the corresponding
        # inspector definition; a definition on ReactCodegen alone does not
        # propagate to ExpoModulesCore's separate Clang module importer.
        if configuration.name == 'Debug'
          definitions = Array(configuration.build_settings['GCC_PREPROCESSOR_DEFINITIONS'])
          definitions << '$(inherited)' unless definitions.include?('$(inherited)')
          definitions << 'RCT_ENABLE_INSPECTOR=1' unless definitions.include?('RCT_ENABLE_INSPECTOR=1')
          configuration.build_settings['GCC_PREPROCESSOR_DEFINITIONS'] = definitions
        end
      end
    end
  end
end
RUBY
  (
    cd "$ios_project"
      PACKED_REACT_NATIVE_DIR="$react_native_dir" \
      RCT_USE_LOCAL_RN_DEP="$ios_project/react-native-dependencies.tar.gz" \
      RCT_TESTONLY_RNCORE_TARBALL_PATH="$ios_project/react-native-core.tar.gz" \
      HERMES_ENGINE_TARBALL_PATH="$ios_project/hermes-ios-debug.tar.gz" \
      CP_CACHE_DIR="$cocoapods_cache_dir" \
      CP_HOME_DIR="$cocoapods_home_dir" \
      pod install --no-repo-update
  ) || fail "iOS consumer pod install failed"
  [[ -f "$ios_project/Podfile.lock" ]] || fail "CocoaPods did not produce Podfile.lock"
  editor_pod_xcconfig="$ios_project/Pods/Target Support Files/ReactNativeProseEditor/ReactNativeProseEditor.debug.xcconfig"
  [[ -f "$editor_pod_xcconfig" ]] || \
    fail "iOS packed consumer did not generate ReactNativeProseEditor build settings"
  grep -Fq '$(PODS_ROOT)/Headers/Private/Yoga' "$editor_pod_xcconfig" || \
    fail "React Native dependency helper did not retain ReactNativeProseEditor's private Yoga compile path"
  require_file "$ios_project/build/generated/ios/ReactCodegen" "react/renderer/components/ReactNativeProseEditorSpec/Props.h"
  require_file "$ios_project/build/generated/ios/ReactCodegen" "react/renderer/components/ReactNativeProseEditorSpec/EventEmitters.h"
  require_file "$ios_project/build/generated/ios/ReactCodegen" "RCTThirdPartyComponentsProvider.mm"
  provider_source="$ios_project/build/generated/ios/ReactCodegen/RCTThirdPartyComponentsProvider.mm"
  grep -Fq '@"PreparedProseViewer": NSClassFromString(@"PREPPreparedProseViewerComponentView")' \
    "$provider_source" || \
    fail "iOS packed consumer codegen is missing the PreparedProseViewer third-party provider entry"
  grep -Fq 'react-native-rich-text-editor' \
    "$provider_source" || \
    fail "iOS packed consumer provider entry is not attributed to the packed dependency"
  workspace_path="$ios_project/PackedConsumer.xcworkspace"
  [[ -f "$workspace_path/contents.xcworkspacedata" ]] || \
    fail "CocoaPods did not generate a valid iOS consumer workspace"
  (
    cd "$ios_project"
    xcodebuild -workspace PackedConsumer.xcworkspace -scheme PackedConsumer -configuration Debug -sdk iphonesimulator -derivedDataPath "$work_dir/ios-derived-data" -jobs 1 CLANG_ENABLE_EXPLICIT_MODULES=NO SWIFT_ENABLE_EXPLICIT_MODULES=NO ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO build
  ) || fail "iOS consumer xcodebuild failed"
}

validate_android_consumer() {
  local root="$1"
  local android_consumer="$work_dir/android-consumer"
  local example_android_dir="$repo_root/example/android"
  local example_node_modules="$repo_root/example/node_modules"
  local apk apk_entries
  root="$(cd "$root" && pwd -P)"
  require_command unzip
  [[ -x "$example_android_dir/gradlew" ]] || fail "local example Gradle wrapper is missing"
  [[ -d "$example_node_modules" ]] || fail "local example Android dependencies are missing; run npm install in example/"

  mkdir -p "$android_consumer"
  cp -R "$example_android_dir" "$android_consumer/android"
  # The example checkout may contain Gradle transforms and CMake state for the
  # source package. A packed-consumer validation must start without those
  # artifacts so its only editor dependency is the extracted project below.
  rm -rf "$android_consumer/android/.gradle" \
    "$android_consumer/android/.idea" \
    "$android_consumer/android/build" \
    "$android_consumer/android/app/.cxx" \
    "$android_consumer/android/app/build"
  # Do not inherit the example app manifest: it declares this checkout via
  # file:.., which Expo autolinking would load alongside the extracted module
  # below and produce duplicate Kotlin classes. The symlink still supplies the
  # compatible Expo/RN tooling; only the packed Android project is included.
  ruby -rjson -e '
    example = JSON.parse(File.read(ARGV.fetch(0)))
    dependencies = example.fetch("dependencies")
    puts JSON.pretty_generate(
      "name" => "packed-tarball-android-consumer",
      "private" => true,
      "dependencies" => {
        "expo" => dependencies.fetch("expo"),
        "react" => dependencies.fetch("react"),
        "react-native" => dependencies.fetch("react-native"),
      },
    )
  ' "$repo_root/example/package.json" > "$android_consumer/package.json"
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
    editorV2CollaborationDrive("1", "0")
    editorV2CollaborationLeaseOutbound("1", "1")
    editorV2CollaborationAckOutbound("1", "1", "1")
    editorV2CollaborationNackOutbound("1", "1", "1")
    editorV2CollaborationDetach("1")
    editorV2CollaborationReattach("1")
  }
}
KOTLIN
  (
    cd "$android_consumer/android"
    PACKED_EDITOR_ANDROID_DIR="$root/android" \
      NODE_PATH="$example_node_modules" \
      GRADLE_USER_HOME="$consumer_gradle_home" \
      ./gradlew app:assembleDebug -x lint -x test --no-daemon
  ) || fail "Android consumer assembleDebug failed"
  apk="$android_consumer/android/app/build/outputs/apk/debug/app-debug.apk"
  [[ -s "$apk" ]] || fail "Android consumer did not produce app-debug.apk"
  apk_entries="$work_dir/android-consumer-apk-entries.txt"
  unzip -Z1 "$apk" > "$apk_entries" || fail "Android consumer APK entries cannot be read"
  for abi in arm64-v8a armeabi-v7a x86 x86_64; do
    grep -Fxq "lib/$abi/libeditor_core.so" "$apk_entries" || fail "Android consumer package is missing libeditor_core.so for $abi"
  done
}

pack_ios_consumer_tarball() {
  local root="$1"
  local pack_json tarball_name
  root="$(cd "$root" && pwd -P)"
  mkdir -p "$pack_cache_dir"
  pack_json="$(mktemp "$work_dir/ios-consumer-pack.XXXXXX.json")"
  (
    cd "$root"
    npm_config_cache="$pack_cache_dir" npm_config_logs_dir="$pack_cache_dir/logs" \
      npm pack --ignore-scripts --json --pack-destination "$work_dir" > "$pack_json"
  )
  tarball_name="$(ruby -rjson -e 'parsed = JSON.parse(File.read(ARGV.fetch(0))); entries = parsed.is_a?(Array) ? parsed : parsed.values; abort "npm pack returned no artifact" unless entries.length == 1 && entries.fetch(0).key?("filename"); puts entries.fetch(0).fetch("filename")' "$pack_json")"
  printf '%s\n' "$work_dir/$tarball_name"
}

validate_package_entries() {
  local root="$1"
  require_file "$root" "ReactNativeProseEditor.podspec"
  [[ ! -e "$root/ios/ReactNativeProseEditor.podspec" ]] || \
    fail "packed npm package must not contain the legacy nested podspec"
  [[ ! -e "$root/ios/build/generated" ]] || \
    fail "packed npm package must not contain consumer-generated iOS codegen output"
  ruby -rjson -e '
    root = ARGV.fetch(0)
    package = JSON.parse(File.read(File.join(root, "package.json")))
    files = package.fetch("files")
    abort "packed package must publish the root podspec" unless files.include?("ReactNativeProseEditor.podspec")
    abort "packed package must not publish a nested podspec glob" if files.include?("ios/*.podspec")
    codegen = package.fetch("codegenConfig")
    abort "codegen name drift" unless codegen.fetch("name") == "ReactNativeProseEditorSpec"
    abort "codegen must expose components" unless codegen.fetch("type") == "components"
    provider = codegen.fetch("ios").fetch("componentProvider")
    abort "PreparedProseViewer provider entry is missing" unless provider == { "PreparedProseViewer" => "PREPPreparedProseViewerComponentView" }
    expo = JSON.parse(File.read(File.join(root, "expo-module.config.json")))
    abort "Expo must discover the package-root podspec" unless expo.fetch("ios").fetch("podspecPath") == "./ReactNativeProseEditor.podspec"
    podspec = File.read(File.join(root, "ReactNativeProseEditor.podspec"))
    pod_name = podspec[/s\.name\s*=\s*\x27([^\x27]+)\x27/, 1]
    pod_module_name = podspec[/s\.module_name\s*=\s*\x27([^\x27]+)\x27/, 1]
    abort "Expo-imported pod name must match the public Swift module name" unless pod_name == "ReactNativeProseEditor" && pod_module_name == pod_name
  ' "$root" || fail "packed package codegen discovery contract failed"
  require_file "$root" "dist/index.js"
  require_file "$root" "dist/index.d.ts"
  require_file "$root" "android/src/debug/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt"
  require_file "$root" "android/src/release/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt"
  require_declaration_symbol "$root" "NativeEditorBoundaryError"
  require_declaration_symbol "$root" "NativeCollaborationTransportConfig"
  require_declaration_symbol "$root" "NativeCollaborationTransportEvent"
  require_declaration_symbol "$root" "resourceLimits"
  require_declaration_symbol "$root" "requestTimeoutMs"
  for obsolete in \
    createWebSocket \
    collaborationTakeOutbound \
    editorV2CollaborationBeginConnect \
    editorV2CollaborationTick \
    drainOutbound \
    collaborationGeneration \
    outboundFrameSink \
    onLocalDocumentCommit
  do
    reject_dist_symbol "$root" "$obsolete"
  done

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
    require_file "$root" "$viewer_source"
  done

  [[ ! -e "$root/ios/NativeProseViewerExpoView.swift" ]] || \
    fail "packed npm package contains removed NativeProseViewerExpoView.swift"
}

extract_package_tarball() {
  local tarball_path="$1"
  tarball_path="$(cd "$(dirname "$tarball_path")" && pwd -P)/$(basename "$tarball_path")"
  [[ -f "$tarball_path" ]] || fail "packed npm tarball is missing: $tarball_path"
  tar -xzf "$tarball_path" -C "$work_dir"
  [[ -d "$package_dir" ]] || fail "npm tarball does not contain the canonical package/ root"
}

validate_packed_package_root() {
  local root="$1"
  local ffi_header_count modulemap_count

  validate_package_entries "$root"
  "$repo_root/scripts/validate-android-rn076-consumer.sh" --validate-package-root "$root"
  validate_abi_root "$root"
  validate_xcframework "$root/ios/EditorCore.xcframework" "$(ios_deployment_target "$root")"
  for abi in arm64-v8a armeabi-v7a x86 x86_64; do
    validate_android_library "$root/rust/android/$abi/libeditor_core.so" "$abi"
  done

  ffi_header_count="$(find "$root" -type f -name 'editor_coreFFI.h' | wc -l | tr -d '[:space:]')"
  modulemap_count="$(find "$root" -type f \( -name 'module.modulemap' -o -name 'editor_coreFFI.modulemap' \) | wc -l | tr -d '[:space:]')"
  [[ "$ffi_header_count" == "1" ]] || fail "packed npm package must contain exactly one editor_coreFFI.h (found $ffi_header_count)"
  [[ "$modulemap_count" == "1" ]] || fail "packed npm package must contain exactly one UniFFI modulemap (found $modulemap_count)"
}

validate_packed_podspec() {
  local root="$1"
  local podspec_json="$work_dir/podspec.json"

  echo "==> Parsing the podspec from the extracted package..."
  RUBYOPT="${RUBYOPT:+$RUBYOPT }-r$repo_root/example/node_modules/react-native/scripts/react_native_pods.rb" \
    pod ipc spec "$root/ReactNativeProseEditor.podspec" > "$podspec_json"
  ruby -rjson -e '
    spec = JSON.parse(File.read(ARGV.fetch(0)))
    license = spec.fetch("license")
    abort "podspec license type must be Apache-2.0" unless license.fetch("type") == "Apache-2.0"
    abort "podspec license file must resolve to LICENSE" unless license.fetch("file") == "LICENSE"
    abort "podspec must vend exactly ios/EditorCore.xcframework" unless Array(spec.fetch("vendored_frameworks")) == ["ios/EditorCore.xcframework"]
    private_headers = Array(spec.fetch("private_header_files"))
    required_private_headers = [
      "ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.h",
      "common/cpp/react/renderer/components/PreparedProseViewer/**/*.h",
    ]
    abort "podspec must keep every Fabric implementation header private" unless required_private_headers.all? { |path| private_headers.include?(path) }
    source_files = Array(spec.fetch("source_files"))
    abort "podspec must compile Fabric implementation C++ sources" unless source_files.include?("common/cpp/**/*.{h,cpp}")
    abort "podspec must preserve the Fabric compiler header directory" unless spec.fetch("header_dir") == "react/renderer/components/PreparedProseViewer"
    abort "podspec public Swift module must agree with the Expo pod-name import" unless spec.fetch("name") == "ReactNativeProseEditor" && spec.fetch("module_name") == spec.fetch("name")
  ' "$podspec_json" || fail "packed podspec does not unconditionally vend EditorCore.xcframework"
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
    validate_xcframework "$2" "$(ios_deployment_target "$repo_root")"
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
    [[ "$#" == "2" || "$#" == "3" ]] || fail "usage: $0 --validate-ios-consumer PACKED_ROOT [TARBALL_PATH]"
    ios_consumer_tarball_path="${3:-}"
    if [[ -z "$ios_consumer_tarball_path" ]]; then
      ios_consumer_tarball_path="$(pack_ios_consumer_tarball "$2")"
    fi
    validate_ios_consumer "$2" "$ios_consumer_tarball_path"
    echo "iOS packed consumer compiles and links the final API."
    exit 0
    ;;
  --validate-android-consumer)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-android-consumer PACKED_ROOT"
    validate_android_consumer "$2"
    echo "Android packed consumer compiles and packages all ABI libraries."
    exit 0
    ;;
  --validate-android-tarball)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-android-tarball TARBALL"
    require_command tar; require_command ruby; require_command unzip
    extract_package_tarball "$2"
    validate_android_consumer "$package_dir"
    echo "Android packed tarball consumer compiles and packages all ABI libraries."
    exit 0
    ;;
  --validate-packed-tarball)
    [[ "$#" == "2" ]] || fail "usage: $0 --validate-packed-tarball TARBALL"
    require_command tar; require_command ruby; require_command pod; require_command plutil
    require_command lipo; require_command file; require_command nm
    extract_package_tarball "$2"
    validate_packed_package_root "$package_dir"
    validate_packed_podspec "$package_dir"
    echo "Packed npm artifact contents and exact native ABI validation passed."
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
tarball_name="$(ruby -rjson -e 'parsed = JSON.parse(File.read(ARGV.fetch(0))); entries = parsed.is_a?(Array) ? parsed : parsed.values; abort "npm pack returned no artifact" unless entries.length == 1 && entries.fetch(0).key?("filename"); puts entries.fetch(0).fetch("filename")' "$pack_json")"
tarball_path="$work_dir/$tarball_name"
[[ -f "$tarball_path" ]] || fail "npm pack did not create $tarball_name"
extract_package_tarball "$tarball_path"

validate_packed_package_root "$package_dir"
validate_abi_root "$repo_root"
validate_copies "$repo_root" "$package_dir"
validate_packed_podspec "$package_dir"

validate_ios_consumer "$package_dir" "$tarball_path"
validate_android_consumer "$package_dir"
echo "==> Packed npm artifact, exact ABI, and real consumer validation passed."
