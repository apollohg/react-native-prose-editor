#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/native-editor-toolchain.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  [[ "$haystack" == *"$needle"* ]] || fail "expected output to contain: $needle"
}

make_toolchain() {
  local name="$1"
  local toolchain_dir="$fixture_root/$name/bin"
  mkdir -p "$toolchain_dir"

  for tool in cargo rustc rustdoc; do
    cat > "$toolchain_dir/$tool" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
tool="$(basename "$0")"
if [[ "${1:-}" == "--version" ]]; then
  printf '%s 1.95.0 (fixture)\n' "$tool"
  exit 0
fi
if [[ "$tool" == "cargo" && "${1:-}" == "ndk" ]]; then
  exec cargo-ndk "${@:2}"
fi
printf '%s:%s\n' "$tool" "$*" >> "${TOOLCHAIN_FIXTURE_LOG:?}"
SCRIPT
    chmod +x "$toolchain_dir/$tool"
  done

  printf '%s\n' "$toolchain_dir"
}

make_rustup() {
  local bin_dir="$fixture_root/fake-bin"
  mkdir -p "$bin_dir"
  cat > "$bin_dir/rustup" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${RUSTUP_FIXTURE_LOG:?}"
if [[ "$1" != "which" || "$2" != "--toolchain" || "$3" != "1.95.0" ]]; then
  echo "unexpected rustup arguments: $*" >&2
  exit 64
fi
if [[ "${RUSTUP_FIXTURE_FAIL_TOOL:-}" == "$4" ]]; then
  echo "toolchain '1.95.0' is not installed" >&2
  exit 1
fi
if [[ "${RUSTUP_FIXTURE_SPLIT_TOOL:-}" == "$4" ]]; then
  printf '%s/other/bin/%s\n' "$RUSTUP_FIXTURE_ROOT" "$4"
else
  printf '%s/%s/bin/%s\n' "$RUSTUP_FIXTURE_ROOT" "$RUSTUP_FIXTURE_TOOLCHAIN" "$4"
fi
SCRIPT
  chmod +x "$bin_dir/rustup"
  printf '%s\n' "$bin_dir"
}

make_cargo_ndk() {
  local bin_dir="$1"
  cat > "$bin_dir/cargo-ndk" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
printf 'CARGO=%s\n' "${CARGO:-}" >> "${CARGO_NDK_FIXTURE_LOG:?}"

target=""
target_dir=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --target)
      target="$2"
      shift 2
      ;;
    --target-dir)
      target_dir="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

: "${target:?missing --target}"
: "${target_dir:?missing --target-dir}"
mkdir -p "$target_dir/$target/release"
printf 'fixture\n' > "$target_dir/$target/release/libeditor_core.so"
SCRIPT
  chmod +x "$bin_dir/cargo-ndk"

  cat > "$bin_dir/nm" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
for symbol in $(seq 1 29); do
  printf 'uniffi_editor_core_fn_func_editor_v2_%s\n' "$symbol"
done
SCRIPT
  chmod +x "$bin_dir/nm"
}

arm64_dir="$(make_toolchain arm64-host)"
x86_dir="$(make_toolchain x86_64-host)"
mkdir -p "$fixture_root/other/bin"
cp "$arm64_dir/cargo" "$fixture_root/other/bin/cargo"
cp "$arm64_dir/rustc" "$fixture_root/other/bin/rustc"
cp "$arm64_dir/rustdoc" "$fixture_root/other/bin/rustdoc"
fake_bin="$(make_rustup)"
fake_bin_physical="$(cd "$fake_bin" && pwd -P)"

run_source() {
  local selected_toolchain="$1"
  local log_path="$2"
  RUSTUP_FIXTURE_ROOT="$fixture_root" \
  RUSTUP_FIXTURE_TOOLCHAIN="$selected_toolchain" \
  RUSTUP_FIXTURE_LOG="$fixture_root/rustup.log" \
  TOOLCHAIN_FIXTURE_LOG="$log_path" \
  PATH="$fake_bin:/usr/bin:/bin" \
  bash -c 'source "$1"; printf "cargo=%s\nrustc=%s\nrustdoc=%s\nplugins=%s\n" "$RUST_TOOLCHAIN_CARGO" "$RUSTC" "$RUSTDOC" "${RUST_TOOLCHAIN_CARGO_PLUGIN_DIR:-}"; "$RUST_TOOLCHAIN_CARGO" bench --fixture' bash "$repo_root/rust/toolchain.sh"
}

for host in arm64-host x86_64-host; do
  : > "$fixture_root/rustup.log"
  : > "$fixture_root/$host.log"
  output="$(run_source "$host" "$fixture_root/$host.log")"
  expected_dir="$fixture_root/$host/bin"
  assert_contains "$output" "cargo=$expected_dir/cargo"
  assert_contains "$output" "rustc=$expected_dir/rustc"
  assert_contains "$output" "rustdoc=$expected_dir/rustdoc"
  assert_contains "$output" "plugins=$fake_bin_physical"
  [[ "$(cat "$fixture_root/$host.log")" == 'cargo:bench --fixture' ]] || fail "$host cargo argv was not preserved"
  [[ "$(cat "$fixture_root/rustup.log")" == $'which --toolchain 1.95.0 cargo\nwhich --toolchain 1.95.0 rustc\nwhich --toolchain 1.95.0 rustdoc' ]] || fail "$host did not resolve all three tools through the pinned rustup toolchain"
done

: > "$fixture_root/override.log"
override_output="$(TOOLCHAIN_FIXTURE_LOG="$fixture_root/override.log" PATH="/usr/bin:/bin" RUST_TOOLCHAIN_DIR="$x86_dir" bash -c 'source "$1"; printf "cargo=%s\nrustc=%s\nrustdoc=%s\n" "$RUST_TOOLCHAIN_CARGO" "$RUSTC" "$RUSTDOC"; "$RUST_TOOLCHAIN_CARGO" check --override' bash "$repo_root/rust/toolchain.sh")"
assert_contains "$override_output" "cargo=$x86_dir/cargo"
assert_contains "$override_output" "rustc=$x86_dir/rustc"
assert_contains "$override_output" "rustdoc=$x86_dir/rustdoc"
[[ "$(cat "$fixture_root/override.log")" == 'cargo:check --override' ]] || fail "override cargo argv was not preserved"

missing_dir="$fixture_root/missing-tool/bin"
mkdir -p "$missing_dir"
cp "$arm64_dir/cargo" "$missing_dir/cargo"
cp "$arm64_dir/rustc" "$missing_dir/rustc"
if RUST_TOOLCHAIN_DIR="$missing_dir" PATH="/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/missing.out" 2>&1; then
  fail "accepted an override without rustdoc"
fi
assert_contains "$(cat "$fixture_root/missing.out")" 'RUST_TOOLCHAIN_DIR must contain executable cargo, rustc, and rustdoc'

if PATH="/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/no-rustup.out" 2>&1; then
  fail "accepted a missing rustup"
fi
assert_contains "$(cat "$fixture_root/no-rustup.out")" 'rustup is required to resolve pinned Rust toolchain 1.95.0'

if RUSTUP_FIXTURE_ROOT="$fixture_root" RUSTUP_FIXTURE_TOOLCHAIN=arm64-host RUSTUP_FIXTURE_LOG="$fixture_root/rustup.log" RUSTUP_FIXTURE_FAIL_TOOL=rustdoc PATH="$fake_bin:/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/missing-pinned.out" 2>&1; then
  fail "accepted a missing pinned rustdoc"
fi
assert_contains "$(cat "$fixture_root/missing-pinned.out")" 'failed to resolve rustdoc from rustup toolchain 1.95.0'

wrong_cargo_dir="$fixture_root/wrong-cargo/bin"
wrong_rustc_dir="$fixture_root/wrong-rustc/bin"
wrong_rustdoc_dir="$fixture_root/wrong-rustdoc/bin"
for wrong_dir in "$wrong_cargo_dir" "$wrong_rustc_dir" "$wrong_rustdoc_dir"; do
  mkdir -p "$wrong_dir"
  cp "$arm64_dir/cargo" "$wrong_dir/cargo"
  cp "$arm64_dir/rustc" "$wrong_dir/rustc"
  cp "$arm64_dir/rustdoc" "$wrong_dir/rustdoc"
done
sed 's/1\.95\.0/1.94.0/g' "$arm64_dir/cargo" > "$wrong_cargo_dir/cargo"
sed 's/1\.95\.0/1.94.0/g' "$arm64_dir/rustc" > "$wrong_rustc_dir/rustc"
sed 's/1\.95\.0/1.94.0/g' "$arm64_dir/rustdoc" > "$wrong_rustdoc_dir/rustdoc"
for wrong_dir in "$wrong_cargo_dir" "$wrong_rustc_dir" "$wrong_rustdoc_dir"; do
  chmod +x "$wrong_dir/cargo" "$wrong_dir/rustc" "$wrong_dir/rustdoc"
done
if RUST_TOOLCHAIN_DIR="$wrong_cargo_dir" PATH="/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/wrong-cargo.out" 2>&1; then
  fail "accepted a wrong-version cargo override"
fi
assert_contains "$(cat "$fixture_root/wrong-cargo.out")" 'pinned Rust toolchain requires cargo 1.95.0'

if RUST_TOOLCHAIN_DIR="$wrong_rustc_dir" PATH="/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/wrong-rustc.out" 2>&1; then
  fail "accepted a wrong-version rustc override"
fi
assert_contains "$(cat "$fixture_root/wrong-rustc.out")" 'pinned Rust toolchain requires rustc 1.95.0'

if RUST_TOOLCHAIN_DIR="$wrong_rustdoc_dir" PATH="/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/wrong-rustdoc.out" 2>&1; then
  fail "accepted a wrong-version rustdoc override"
fi
assert_contains "$(cat "$fixture_root/wrong-rustdoc.out")" 'pinned Rust toolchain requires rustdoc 1.95.0'

if RUSTUP_FIXTURE_ROOT="$fixture_root" RUSTUP_FIXTURE_TOOLCHAIN=arm64-host RUSTUP_FIXTURE_LOG="$fixture_root/rustup.log" RUSTUP_FIXTURE_SPLIT_TOOL=rustdoc PATH="$fake_bin:/usr/bin:/bin" bash -c 'source "$1"' bash "$repo_root/rust/toolchain.sh" >"$fixture_root/split.out" 2>&1; then
  fail "accepted rustup tools from different directories"
fi
assert_contains "$(cat "$fixture_root/split.out")" 'resolved cargo, rustc, and rustdoc must share one directory'

android_fixture_dir="$fixture_root/android-build"
mkdir -p "$android_fixture_dir/editor-core"
cp "$repo_root/rust/build-android.sh" "$android_fixture_dir/build-android.sh"
cp "$repo_root/rust/toolchain.sh" "$android_fixture_dir/toolchain.sh"
cargo_home="$fixture_root/cargo-home"
mkdir -p "$cargo_home/bin"
make_cargo_ndk "$cargo_home/bin"
: > "$fixture_root/cargo-ndk.log"
env -i \
  PATH="$fake_bin:/usr/bin:/bin" \
  RUST_TOOLCHAIN_DIR="$arm64_dir" \
  CARGO_HOME="$cargo_home" \
  CARGO_NDK_FIXTURE_LOG="$fixture_root/cargo-ndk.log" \
  bash "$android_fixture_dir/build-android.sh"
expected_cargo_ndk_log="$(printf 'CARGO=%s\n' "$arm64_dir/cargo" "$arm64_dir/cargo" "$arm64_dir/cargo" "$arm64_dir/cargo")"
[[ "$(cat "$fixture_root/cargo-ndk.log")" == "$expected_cargo_ndk_log" ]] || \
  fail "cargo-ndk discovered through CARGO_HOME did not receive the pinned Cargo through CARGO"

echo "Toolchain discovery fixture tests passed."
