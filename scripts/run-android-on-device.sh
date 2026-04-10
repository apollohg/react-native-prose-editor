#!/usr/bin/env bash
set -euo pipefail

project_dir="example/android"
task=":apollohg-react-native-prose-editor:connectedDebugAndroidTest"
device_id="${ANDROID_DEVICE_ID:-${ANDROID_SERIAL:-}}"
test_class=""
extra_args=()

if [[ -f "$project_dir/.device-test.env" ]]; then
  # shellcheck disable=SC1090
  source "$project_dir/.device-test.env"
  device_id="${ANDROID_DEVICE_ID:-${ANDROID_SERIAL:-$device_id}}"
fi

usage() {
  cat <<'EOF'
Usage: scripts/run-android-on-device.sh [--device-id <serial>] [--class <fqcn>] [--] [gradle args...]

Runs Android instrumentation tests for the native editor library on a connected
device or emulator. If no device id is provided, the first attached device in
"adb devices" state "device" is used.

Options:
  --device-id <serial>   Specific adb serial to target.
  --class <fqcn>         Fully-qualified instrumentation test class filter.
  --help                 Show this help text.

Environment:
  ANDROID_DEVICE_ID      Specific adb serial to target.
  ANDROID_SERIAL         Standard adb serial override.

Examples:
  npm run android:test:device
  npm run android:test:perf:device
  ANDROID_DEVICE_ID=<serial> npm run android:test:perf:device
EOF
}

while (($# > 0)); do
  case "$1" in
    --device-id)
      if (($# < 2)); then
        echo "Missing value for --device-id" >&2
        exit 1
      fi
      device_id="$2"
      shift 2
      ;;
    --class)
      if (($# < 2)); then
        echo "Missing value for --class" >&2
        exit 1
      fi
      test_class="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      extra_args+=("$@")
      break
      ;;
    *)
      extra_args+=("$1")
      shift
      ;;
  esac
done

resolve_device_id() {
  if [[ -n "$device_id" ]]; then
    printf '%s\n' "$device_id"
    return 0
  fi

  local detected_device
  detected_device="$(
    adb devices \
      | awk '$2 == "device" { print $1; exit }'
  )"

  if [[ -n "$detected_device" ]]; then
    printf '%s\n' "$detected_device"
    return 0
  fi

  echo "No connected Android devices or emulators in adb state 'device' were found." >&2
  echo "Connect a device or set ANDROID_DEVICE_ID/ANDROID_SERIAL." >&2
  exit 1
}

device_id="$(resolve_device_id)"
export ANDROID_SERIAL="$device_id"

gradle_args=()
if [[ -n "$test_class" ]]; then
  gradle_args+=("-Pandroid.testInstrumentationRunnerArguments.class=$test_class")
fi

echo "Running Android connected tests on device: $device_id"

cd "$project_dir"
./gradlew "$task" "${gradle_args[@]}" "${extra_args[@]}"
