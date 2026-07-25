#!/usr/bin/env bash
#
# Compiled editor-core artifacts are build outputs, not source. They are not
# tracked in git: CI builds them from the pinned Rust toolchain and publishes
# them inside the npm tarball. Any task that links them must therefore state
# that requirement up front rather than failing later inside a linker.
#
# Usage: require_native_artifacts ios | android | all

set -euo pipefail

require_native_artifacts() {
  local platform="${1:?Usage: require_native_artifacts ios|android|all}"
  local repo_root
  repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

  local -a missing=()
  local -a required=()

  case "$platform" in
    ios)
      required=(
        "ios/EditorCore.xcframework/ios-arm64/libeditor_core.a"
        "ios/EditorCore.xcframework/ios-arm64_x86_64-simulator/libeditor_core.a"
      )
      ;;
    android)
      required=(
        "rust/android/arm64-v8a/libeditor_core.so"
        "rust/android/armeabi-v7a/libeditor_core.so"
        "rust/android/x86/libeditor_core.so"
        "rust/android/x86_64/libeditor_core.so"
      )
      ;;
    all)
      require_native_artifacts ios
      require_native_artifacts android
      return 0
      ;;
    *)
      echo "error: unknown artifact platform '$platform'" >&2
      return 1
      ;;
  esac

  local artifact
  for artifact in "${required[@]}"; do
    [[ -f "$repo_root/$artifact" ]] || missing+=("$artifact")
  done

  if [[ ${#missing[@]} -eq 0 ]]; then
    return 0
  fi

  local build_script="build:rust"
  [[ "$platform" == "ios" ]] && build_script="build:rust:ios"
  [[ "$platform" == "android" ]] && build_script="build:rust:android"

  {
    echo "error: compiled editor-core artifacts are missing:"
    printf '  %s\n' "${missing[@]}"
    echo
    echo "These are build outputs and are not tracked in git. Build them with:"
    echo "  npm run $build_script"
    echo
    echo "This needs the pinned Rust toolchain (see rust/toolchain.sh)."
    [[ "$platform" == "android" ]] && echo "Android also needs ANDROID_NDK_HOME and cargo-ndk."
  } >&2
  return 1
}

# Allow direct invocation as well as sourcing.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  require_native_artifacts "${1:-all}"
fi
