#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

# The test host links EditorCore.xcframework, which is a build output.
# shellcheck source=./require-native-artifacts.sh
source "$repo_root/scripts/require-native-artifacts.sh"
require_native_artifacts ios

workspace="$repo_root/ios-tests/NativeEditorTests.xcworkspace"
scheme="${NATIVE_EDITOR_IOS_TEST_SCHEME:-NativeEditorTests}"
destination="${IOS_DESTINATION:-}"
simulator_name="${IOS_SIMULATOR_NAME:-}"
device_id="${IOS_DEVICE_ID:-}"
team_id="${IOS_DEVELOPMENT_TEAM:-}"

usage() {
  cat <<'EOF'
Usage: scripts/run-ios-tests.sh [--destination <destination>] [--simulator <name>] [--device-id <udid>] [--team-id <team>] [--] [xcodebuild args...]

Runs the native iOS test suite from the CocoaPods workspace so pod targets like
ExpoModulesCore are included in the build graph.

Options:
  --destination <destination>  Exact xcodebuild destination string.
  --simulator <name>           Preferred simulator name when auto-selecting.
  --device-id <udid>           Physical iOS device identifier.
  --team-id <team>             Development team used for physical device signing.
  --help                       Show this help text.

Environment:
  IOS_DESTINATION              Exact xcodebuild destination string override.
  IOS_SIMULATOR_NAME           Preferred simulator name when auto-selecting.
  IOS_DEVICE_ID                Physical iOS device identifier override.
  IOS_DEVELOPMENT_TEAM         Development team used for physical device signing.
  NATIVE_EDITOR_IOS_TEST_SCHEME
                              Xcode scheme override; defaults to NativeEditorTests.

Examples:
  npm run test:ios
  npm run test:ios -- -only-testing:NativeEditorTests/RenderBridgeTests
  IOS_SIMULATOR_NAME="iPhone 17" npm run test:ios
  IOS_DEVICE_ID="<udid>" IOS_DEVELOPMENT_TEAM="<team>" npm run test:ios
  NATIVE_EDITOR_IOS_TEST_SCHEME=NativeEditorPreparedProsePerformance npm run test:ios
EOF
}

while (($# > 0)); do
  case "$1" in
    --destination)
      if (($# < 2)); then
        echo "Missing value for --destination" >&2
        exit 1
      fi
      destination="$2"
      shift 2
      ;;
    --simulator)
      if (($# < 2)); then
        echo "Missing value for --simulator" >&2
        exit 1
      fi
      simulator_name="$2"
      shift 2
      ;;
    --device-id)
      if (($# < 2)); then
        echo "Missing value for --device-id" >&2
        exit 1
      fi
      device_id="$2"
      shift 2
      ;;
    --team-id)
      if (($# < 2)); then
        echo "Missing value for --team-id" >&2
        exit 1
      fi
      team_id="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      break
      ;;
  esac
done

if [[ ! -d "$workspace" ]]; then
  echo "Workspace not found at $workspace. Run 'cd ios-tests && pod install' first." >&2
  exit 1
fi

if grep -q 'path = "Pods/Target Support Files/Pods-NativeEditorTests/ExpoModulesProvider.swift"' \
  "$repo_root/ios-tests/NativeEditorTests.xcodeproj/project.pbxproj"; then
  echo "The iOS test bundle must not compile a second ExpoModulesProvider." >&2
  exit 1
fi

resolve_destination() {
  local destinations_output line matched_id fallback_id

  if [[ -n "$destination" ]]; then
    printf '%s\n' "$destination"
    return 0
  fi

  if [[ -n "$device_id" ]]; then
    printf 'platform=iOS,id=%s\n' "$device_id"
    return 0
  fi

  local booted_id=""
  booted_id="$(
    xcrun simctl list devices available \
      | sed -nE 's/^[[:space:]]+([^()]+) \(([0-9A-F-]+)\) \(Booted\)$/\1|\2/p' \
      | while IFS='|' read -r name id; do
          if [[ -n "$simulator_name" && "$name" != "$simulator_name" ]]; then
            continue
          fi
          printf '%s\n' "$id"
          break
        done
  )"
  if [[ -n "$booted_id" ]]; then
    printf 'id=%s\n' "$booted_id"
    return 0
  fi

  destinations_output="$(xcodebuild -showdestinations -workspace "$workspace" -scheme "$scheme" 2>/dev/null)"
  matched_id="$(
    printf '%s\n' "$destinations_output" \
      | sed -nE 's/^[[:space:]]+\{ platform:iOS Simulator,.* id:([^,]+),.* name:([^}]+) \}$/\2|\1/p' \
      | while IFS='|' read -r name id; do
          if [[ "$name" == "Any iOS Simulator Device" ]]; then
            continue
          fi
          if [[ -n "$simulator_name" && "$name" != "$simulator_name" ]]; then
            continue
          fi
          if [[ "$name" == iPhone* ]]; then
            printf '%s\n' "$id"
            break
          fi
        done
  )"
  if [[ -n "$matched_id" ]]; then
    printf 'id=%s\n' "$matched_id"
    return 0
  fi

  fallback_id="$(
    printf '%s\n' "$destinations_output" \
      | sed -nE 's/^[[:space:]]+\{ platform:iOS Simulator,.* id:([^,]+),.* name:([^}]+) \}$/\2|\1/p' \
      | while IFS='|' read -r name id; do
          if [[ "$name" == "Any iOS Simulator Device" ]]; then
            continue
          fi
          if [[ -n "$simulator_name" && "$name" != "$simulator_name" ]]; then
            continue
          fi
          printf '%s\n' "$id"
          break
        done
  )"
  if [[ -n "$fallback_id" ]]; then
    printf 'id=%s\n' "$fallback_id"
    return 0
  fi

  echo "No available iOS simulator destination was found for scheme '$scheme'." >&2
  exit 1
}

destination="$(resolve_destination)"

is_physical_device_destination() {
  [[ "$1" == platform=iOS,* ]]
}

if [[ "$destination" =~ ^id= ]]; then
  simulator_id="${destination#id=}"
  xcrun simctl boot "$simulator_id" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$simulator_id" -b
fi

echo "Running iOS tests with destination: $destination"

xcodebuild_args=(
  test
  -workspace "$workspace"
  -scheme "$scheme"
  -destination "$destination"
)

device_build_settings=()
if is_physical_device_destination "$destination"; then
  if [[ -z "$team_id" ]]; then
    echo "Physical device testing requires --team-id or IOS_DEVELOPMENT_TEAM." >&2
    exit 1
  fi
  xcodebuild_args+=(
    -allowProvisioningUpdates
    -allowProvisioningDeviceRegistration
  )
  device_build_settings+=(
    "IOS_DEVELOPMENT_TEAM=$team_id"
    "DEVELOPMENT_TEAM=$team_id"
    "CODE_SIGN_STYLE=Automatic"
  )
fi

xcodebuild_cmd=(
  xcodebuild
  "${xcodebuild_args[@]}"
  "$@"
)

if [[ ${#device_build_settings[@]} -gt 0 ]]; then
  xcodebuild_cmd+=("${device_build_settings[@]}")
fi

"${xcodebuild_cmd[@]}"
