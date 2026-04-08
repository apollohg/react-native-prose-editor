#!/usr/bin/env bash

set -euo pipefail

workspace="ios-tests/NativeEditorTests.xcworkspace"
scheme="NativeEditorTests"
destination="${IOS_DESTINATION:-}"
simulator_name="${IOS_SIMULATOR_NAME:-}"

usage() {
  cat <<'EOF'
Usage: scripts/run-ios-tests.sh [--destination <destination>] [--simulator <name>] [--] [xcodebuild args...]

Runs the native iOS test suite from the CocoaPods workspace so pod targets like
ExpoModulesCore are included in the build graph.

Options:
  --destination <destination>  Exact xcodebuild destination string.
  --simulator <name>           Preferred simulator name when auto-selecting.
  --help                       Show this help text.

Environment:
  IOS_DESTINATION              Exact xcodebuild destination string override.
  IOS_SIMULATOR_NAME           Preferred simulator name when auto-selecting.

Examples:
  npm run ios:test
  npm run ios:test -- -only-testing:NativeEditorTests/RenderBridgeTests
  IOS_SIMULATOR_NAME="iPhone 17" npm run ios:test
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

resolve_destination() {
  local destinations_output line matched_id fallback_id

  if [[ -n "$destination" ]]; then
    printf '%s\n' "$destination"
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

if [[ "$destination" =~ ^id= ]]; then
  simulator_id="${destination#id=}"
  xcrun simctl boot "$simulator_id" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$simulator_id" -b
fi

echo "Running iOS tests with destination: $destination"

xcodebuild test \
  -workspace "$workspace" \
  -scheme "$scheme" \
  -destination "$destination" \
  "$@"
