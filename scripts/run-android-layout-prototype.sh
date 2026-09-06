#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_dir"
sdk_dir="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
adb_bin="$sdk_dir/platform-tools/adb"
if [[ ! -x "$adb_bin" ]]; then adb_bin="$(command -v adb)"; fi
device_id="${ANDROID_SERIAL:-${ANDROID_DEVICE_ID:-}}"
if [[ -z "$device_id" ]]; then
    device_id="$("$adb_bin" devices | awk 'NR > 1 && $2 == "device" {print $1; exit}')"
fi
if [[ -z "$device_id" ]]; then
    echo "Start an Android emulator or connect a device, then rerun this command." >&2
    exit 1
fi
if [[ ! -f example/android/gradlew ]]; then npm run prebuild:example:android; fi
(
    cd example/android
    ./gradlew :react-native-rich-text-editor:assembleDebugAndroidTest
)
prototype_apk="$repo_dir/android/build/outputs/apk/androidTest/debug/react-native-rich-text-editor-debug-androidTest.apk"
"$adb_bin" -s "$device_id" install -r "$prototype_apk"
"$adb_bin" -s "$device_id" shell am start -n com.apollohg.editor.test/com.apollohg.editor.prototype.PrototypeEditorActivity
