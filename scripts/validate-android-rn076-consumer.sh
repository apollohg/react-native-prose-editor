#!/usr/bin/env bash

set -euo pipefail
unset npm_config_allow_scripts NPM_CONFIG_ALLOW_SCRIPTS

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture_root="$repo_root/scripts/tests/android-rn076-consumer"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-rn076-consumer.XXXXXX")"
export ANDROID_USER_HOME="$work_dir/android-user-home"
mkdir -p "$ANDROID_USER_HOME"

cleanup() {
  if [[ "${RN076_KEEP_WORK_DIR:-0}" == "1" ]]; then
    echo "RN 0.76 validation work directory: $work_dir" >&2
  else
    rm -rf "$work_dir"
  fi
}
trap cleanup EXIT

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

require_file() {
  local root="$1"
  local path="$2"
  [[ -s "$root/$path" ]] || fail "RN 0.76 package is missing $path"
}

validate_package_root() {
  local root="$1"
  root="$(cd "$root" && pwd -P)"

  require_file "$root" "android/src/main/jni/CMakeLists.txt"
  require_file "$root" "android/expo/build.gradle"
  require_file "$root" "common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp"
  require_file "$root" "common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerComponentDescriptor.h"
  require_file "$root" "src/specs/PreparedProseViewerNativeComponent.ts"
  for abi in arm64-v8a armeabi-v7a x86 x86_64; do
    require_file "$root" "rust/android/$abi/libeditor_core.so"
  done

  node --input-type=module - "$root" <<'NODE'
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.argv[2];
const fail = (message) => {
  console.error(`ERROR: ${message}`);
  process.exit(1);
};
const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const codegen = manifest.codegenConfig;
if (
  codegen?.type !== 'components' ||
  codegen?.jsSrcsDir !== 'src/specs' ||
  codegen?.android?.javaPackageName !== 'com.apollohg.editor.viewer'
) {
  fail('RN 0.76 package codegen must expose the PreparedProseViewer component');
}
const componentSource = readFileSync(
  join(root, 'src/specs/PreparedProseViewerNativeComponent.ts'),
  'utf8',
);
if (!componentSource.includes("codegenNativeComponent<NativeProps>('PreparedProseViewer'")) {
  fail('RN 0.76 package codegen must expose the PreparedProseViewer component');
}

const androidBuild = readFileSync(join(root, 'android/build.gradle'), 'utf8');
if (
  androidBuild.includes("id 'expo-module-gradle-plugin'") ||
  !androidBuild.includes("'ExpoModulesCorePlugin.gradle'") ||
  !androidBuild.includes('applyKotlinExpoModulesCorePlugin()') ||
  !androidBuild.includes('useCoreDependencies()')
) {
  fail('RN 0.76 package must use the Expo Modules Core compatibility plugin');
}

const expo = JSON.parse(readFileSync(join(root, 'expo-module.config.json'), 'utf8'));
if (expo.android?.gradlePath !== 'android/expo/build.gradle') {
  fail('RN 0.76 package must route Expo autolinking through the Android facade');
}
if (expo.android?.path !== 'android/expo') {
  fail('Expo module autolinking must use the Android facade project directory');
}
if (!expo.android?.modules?.includes('com.apollohg.editor.NativeEditorModule')) {
  fail('RN 0.76 package Expo module entry is missing com.apollohg.editor.NativeEditorModule');
}
const expoFacade = readFileSync(join(root, 'android/expo/build.gradle'), 'utf8');
if (!expoFacade.includes("api project(':apollohg_react-native-prose-editor')")) {
  fail('RN 0.76 package Expo facade must depend on the React Native project');
}

const reactNativeConfig = readFileSync(join(root, 'react-native.config.js'), 'utf8');
if (
  !reactNativeConfig.includes('PreparedProseViewerComponentDescriptor') ||
  !reactNativeConfig.includes('../android/src/main/jni/CMakeLists.txt')
) {
  fail('RN 0.76 package React Native autolinking metadata is incomplete');
}
NODE
}

extract_tarball() {
  local tarball="$1"
  local destination="$2"
  mkdir -p "$destination"
  tar -xzf "$tarball" -C "$destination"
  [[ -d "$destination/package" ]] || fail "RN 0.76 tarball must contain package/"
}

resolve_tarball() {
  local supplied="${1:-}"
  if [[ -n "$supplied" ]]; then
    [[ -f "$supplied" ]] || fail "release tarball does not exist: $supplied"
    (cd "$(dirname "$supplied")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$supplied")")
    return
  fi

  local pack_dir="$work_dir/pack"
  mkdir -p "$pack_dir"
  (
    cd "$repo_root"
    npm pack --silent --ignore-scripts --pack-destination "$pack_dir" >/dev/null
  )
  local tarballs=("$pack_dir"/*.tgz)
  [[ "${#tarballs[@]}" -eq 1 && -f "${tarballs[0]}" ]] || fail "npm pack must produce exactly one tarball"
  printf '%s\n' "${tarballs[0]}"
}

case "${1:-}" in
  --validate-package-root)
    [[ "$#" -eq 2 ]] || fail "usage: $0 --validate-package-root PATH"
    validate_package_root "$2"
    echo "RN 0.76 packed-package contents passed."
    exit 0
    ;;
  --validate-tarball-contents)
    [[ "$#" -eq 2 ]] || fail "usage: $0 --validate-tarball-contents TARBALL"
    extract_tarball "$2" "$work_dir/extracted"
    validate_package_root "$work_dir/extracted/package"
    echo "RN 0.76 packed-package contents passed."
    exit 0
    ;;
esac

tarball_path="$(resolve_tarball "${1:-${RELEASE_TARBALL:-}}")"
extract_tarball "$tarball_path" "$work_dir/extracted"
validate_package_root "$work_dir/extracted/package"

consumer_root="$work_dir/consumer"
mkdir -p "$consumer_root/android"
cp "$fixture_root/package.json" "$fixture_root/index.js" "$fixture_root/babel.config.js" "$fixture_root/metro.config.js" "$consumer_root/"
cp "$fixture_root/settings.gradle" "$fixture_root/build.gradle" "$fixture_root/gradle.properties" "$consumer_root/android/"
cp -R "$fixture_root/app" "$consumer_root/android/"
node --input-type=module - "$consumer_root/package.json" "$tarball_path" <<'NODE'
import { readFileSync, writeFileSync } from 'node:fs';

const manifestPath = process.argv[2];
const tarballPath = process.argv[3];
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
manifest.dependencies['@apollohg/react-native-prose-editor'] = `file:${tarballPath}`;
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

(
  cd "$consumer_root"
  export CI="${CI:-true}"
  npm install --package-lock-only --ignore-scripts --no-audit --no-fund
  npm ci --ignore-scripts --no-audit --no-fund
  export NODE_ENV=production
  node --input-type=module - "node_modules/@react-native/gradle-plugin/gradle/wrapper/gradle-wrapper.properties" <<'NODE'
import { readFileSync, writeFileSync } from 'node:fs';

const path = process.argv[2];
const contents = readFileSync(path, 'utf8')
  .replace(/^networkTimeout=.*$/m, 'networkTimeout=120000')
  .replace(/-all\.zip$/m, '-bin.zip');
writeFileSync(path, contents);
NODE
  node node_modules/react-native/scripts/generate-codegen-artifacts.js \
    -p "$consumer_root" \
    -t android \
    -o "$consumer_root/codegen"
  node_modules/@react-native/gradle-plugin/gradlew \
    -p "$consumer_root/android" \
    :app:assembleRelease \
    -PnewArchEnabled=true \
    -PreactNativeArchitectures=x86_64 \
    -Pkotlin.compiler.execution.strategy=in-process \
    --no-daemon
)

apks=("$consumer_root/android/app/build/outputs/apk/release/"*.apk)
[[ "${#apks[@]}" -eq 1 && -s "${apks[0]}" ]] || fail "RN 0.76 consumer did not produce exactly one release APK"
apk="${apks[0]}"
apk_entries="$work_dir/apk-entries.txt"
unzip -Z1 "$apk" > "$apk_entries"
grep -Fxq "lib/x86_64/libeditor_core.so" "$apk_entries" || fail "RN 0.76 APK is missing libeditor_core.so"
grep -Fxq "assets/index.android.bundle" "$apk_entries" || fail "RN 0.76 APK is missing the bundled JavaScript entry"

echo "RN 0.76 New Architecture release consumer passed."
