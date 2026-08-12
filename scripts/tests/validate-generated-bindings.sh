#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-generated-bindings.XXXXXX")"
trap 'rm -rf "$fixture_dir"' EXIT

header="$fixture_dir/editor_coreFFI.h"
printf 'first   \nsecond\t\nthird\n' > "$header"
bash "$repo_root/rust/generate-bindings.sh" --normalize-header "$header"
expected="$fixture_dir/expected.h"
printf 'first\nsecond\nthird\n' > "$expected"
cmp -s "$header" "$expected" || {
  echo "ERROR: generated header normalization is not deterministic" >&2
  exit 1
}

cmp -s "$repo_root/ios/Generated_editor_core.swift" \
  "$repo_root/rust/bindings/swift/editor_core.swift" || {
  echo "ERROR: generated Swift source copies differ" >&2
  exit 1
}
cmp -s "$repo_root/ios/editor_coreFFI/editor_coreFFI.h" \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" || {
  echo "ERROR: generated FFI header copies differ" >&2
  exit 1
}

expected_function_symbols="$(rg -o 'uniffi_editor_core_fn_func_editor_v2_[[:alnum:]_]+' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_checksum_symbols="$(rg -o 'uniffi_editor_core_checksum_func_editor_v2_[[:alnum:]_]+' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_viewer_function_symbols="$(rg -o 'uniffi_editor_core_fn_func_viewer_compile' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_viewer_function_checksums="$(rg -o 'uniffi_editor_core_checksum_func_viewer_compile' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_viewer_method_symbols="$(rg -o 'uniffi_editor_core_fn_method_viewercompileddocument_[[:alnum:]_]+' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_viewer_method_checksums="$(rg -o 'uniffi_editor_core_checksum_method_viewercompileddocument_[[:alnum:]_]+' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
expected_viewer_lifecycle_symbols="$(rg -o 'uniffi_editor_core_fn_(clone|free)_viewercompileddocument' \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h" | sort -u)"
[[ "$(printf '%s\n' "$expected_function_symbols" | sed '/^$/d' | wc -l | tr -d ' ')" == "35" ]] || {
  echo "ERROR: generated FFI header must expose exactly 35 editor_v2 functions" >&2
  exit 1
}
[[ "$(printf '%s\n' "$expected_checksum_symbols" | sed '/^$/d' | wc -l | tr -d ' ')" == "35" ]] || {
  echo "ERROR: generated FFI header must expose exactly 35 editor_v2 checksums" >&2
  exit 1
}
[[ "$expected_viewer_function_symbols" == "uniffi_editor_core_fn_func_viewer_compile" ]] || {
  echo "ERROR: generated FFI header must expose exactly viewer_compile" >&2
  exit 1
}
[[ "$expected_viewer_function_checksums" == "uniffi_editor_core_checksum_func_viewer_compile" ]] || {
  echo "ERROR: generated FFI header must expose the viewer_compile checksum" >&2
  exit 1
}
[[ "$(printf '%s\n' "$expected_viewer_method_symbols" | sed '/^$/d' | wc -l | tr -d ' ')" == "6" ]] || {
  echo "ERROR: generated FFI header must expose exactly six ViewerCompiledDocument methods" >&2
  exit 1
}
[[ "$(printf '%s\n' "$expected_viewer_method_checksums" | sed '/^$/d' | wc -l | tr -d ' ')" == "6" ]] || {
  echo "ERROR: generated FFI header must expose exactly six ViewerCompiledDocument checksums" >&2
  exit 1
}
[[ "$(printf '%s\n' "$expected_viewer_lifecycle_symbols" | sed '/^$/d' | wc -l | tr -d ' ')" == "2" ]] || {
  echo "ERROR: generated FFI header must expose ViewerCompiledDocument clone/free lifecycle symbols" >&2
  exit 1
}

for artifact in \
  "$repo_root/rust/bindings/swift/editor_core.swift" \
  "$repo_root/rust/bindings/kotlin/uniffi/editor_core/editor_core.kt" \
  "$repo_root/ios/Generated_editor_core.swift" \
  "$repo_root/ios/editor_coreFFI/editor_coreFFI.h"; do
  actual_function_symbols="$(rg -o 'uniffi_editor_core_fn_func_editor_v2_[[:alnum:]_]+' "$artifact" | sort -u)"
  actual_checksum_symbols="$(rg -o 'uniffi_editor_core_checksum_func_editor_v2_[[:alnum:]_]+' "$artifact" | sort -u)"
  actual_viewer_function_symbols="$(rg -o 'uniffi_editor_core_fn_func_viewer_compile' "$artifact" | sort -u)"
  actual_viewer_function_checksums="$(rg -o 'uniffi_editor_core_checksum_func_viewer_compile' "$artifact" | sort -u)"
  actual_viewer_method_symbols="$(rg -o 'uniffi_editor_core_fn_method_viewercompileddocument_[[:alnum:]_]+' "$artifact" | sort -u)"
  actual_viewer_method_checksums="$(rg -o 'uniffi_editor_core_checksum_method_viewercompileddocument_[[:alnum:]_]+' "$artifact" | sort -u)"
  actual_viewer_lifecycle_symbols="$(rg -o 'uniffi_editor_core_fn_(clone|free)_viewercompileddocument' "$artifact" | sort -u)"
  [[ "$actual_function_symbols" == "$expected_function_symbols" ]] || {
    echo "ERROR: generated function symbols differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_checksum_symbols" == "$expected_checksum_symbols" ]] || {
    echo "ERROR: generated checksum symbols differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_viewer_function_symbols" == "$expected_viewer_function_symbols" ]] || {
    echo "ERROR: generated viewer function symbols differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_viewer_function_checksums" == "$expected_viewer_function_checksums" ]] || {
    echo "ERROR: generated viewer function checksums differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_viewer_method_symbols" == "$expected_viewer_method_symbols" ]] || {
    echo "ERROR: generated ViewerCompiledDocument methods differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_viewer_method_checksums" == "$expected_viewer_method_checksums" ]] || {
    echo "ERROR: generated ViewerCompiledDocument checksums differ in $artifact" >&2
    exit 1
  }
  [[ "$actual_viewer_lifecycle_symbols" == "$expected_viewer_lifecycle_symbols" ]] || {
    echo "ERROR: generated ViewerCompiledDocument lifecycle symbols differ in $artifact" >&2
    exit 1
  }
done

for obsolete in \
  editor_v2_collaboration_begin_connect \
  editor_v2_collaboration_take_outbound \
  editor_v2_collaboration_tick; do
  if rg -n "uniffi_editor_core_(fn|checksum)_func_${obsolete}" \
    "$repo_root/rust/bindings/swift/editor_core.swift" \
    "$repo_root/rust/bindings/swift/editor_coreFFI.h" \
    "$repo_root/rust/bindings/kotlin/uniffi/editor_core/editor_core.kt" \
    "$repo_root/ios/Generated_editor_core.swift" \
    "$repo_root/ios/editor_coreFFI/editor_coreFFI.h"; then
    echo "ERROR: generated bindings still expose obsolete ${obsolete}" >&2
    exit 1
  fi
done

if [[ -e "$repo_root/android/src/main/java/uniffi/editor_core/editor_core.kt" ]]; then
  echo "ERROR: Android must compile rust/bindings/kotlin; do not add a duplicate UniFFI binding" >&2
  exit 1
fi

if rg -n '[[:blank:]]+$' \
  "$repo_root/ios/editor_coreFFI/editor_coreFFI.h" \
  "$repo_root/rust/bindings/swift/editor_coreFFI.h"; then
  echo "ERROR: generated FFI headers contain trailing whitespace" >&2
  exit 1
fi

echo "Generated binding normalization, 35 editor-v2 symbol, viewer ABI, checksum, and copy validation passed."
