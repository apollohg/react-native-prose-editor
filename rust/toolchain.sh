#!/usr/bin/env bash
#
# Source this file to resolve the repository's pinned Rust 1.95.0 toolchain.
# It exports RUST_TOOLCHAIN_CARGO, RUSTC, and RUSTDOC from one installation.

RUST_TOOLCHAIN_VERSION="1.95.0"

toolchain_error() {
    echo "error: $*" >&2
    return 1
}

toolchain_validate_binary() {
    local binary_path="$1"
    local tool_name="$2"
    local version_output

    [[ -x "$binary_path" ]] || toolchain_error "$tool_name is not executable: $binary_path" || return 1
    version_output="$("$binary_path" --version 2>&1)" || \
        toolchain_error "failed to read $tool_name version from $binary_path" || return 1
    [[ "$version_output" == "$tool_name $RUST_TOOLCHAIN_VERSION "* || "$version_output" == "$tool_name $RUST_TOOLCHAIN_VERSION" ]] || \
        toolchain_error "pinned Rust toolchain requires $tool_name $RUST_TOOLCHAIN_VERSION (found: $version_output)" || return 1
}

toolchain_bin_directory() {
    cd "$(dirname "$1")" && pwd -P
}

toolchain_resolve() {
    local cargo_path rustc_path rustdoc_path rustup_path toolchain_dir

    rustup_path="$(command -v rustup || true)"
    if [[ -n "$rustup_path" && -z "${RUST_TOOLCHAIN_CARGO_PLUGIN_DIR:-}" ]]; then
        RUST_TOOLCHAIN_CARGO_PLUGIN_DIR="$(toolchain_bin_directory "$rustup_path")"
    fi

    if [[ -n "${RUST_TOOLCHAIN_DIR:-}" ]]; then
        toolchain_dir="$RUST_TOOLCHAIN_DIR"
        [[ -x "$toolchain_dir/cargo" && -x "$toolchain_dir/rustc" && -x "$toolchain_dir/rustdoc" ]] || \
            toolchain_error "RUST_TOOLCHAIN_DIR must contain executable cargo, rustc, and rustdoc: $toolchain_dir" || return 1
        cargo_path="$toolchain_dir/cargo"
        rustc_path="$toolchain_dir/rustc"
        rustdoc_path="$toolchain_dir/rustdoc"
    else
        [[ -n "$rustup_path" ]] || \
            toolchain_error "rustup is required to resolve pinned Rust toolchain $RUST_TOOLCHAIN_VERSION" || return 1

        cargo_path="$("$rustup_path" which --toolchain "$RUST_TOOLCHAIN_VERSION" cargo 2>/dev/null)" || \
            toolchain_error "failed to resolve cargo from rustup toolchain $RUST_TOOLCHAIN_VERSION" || return 1
        rustc_path="$("$rustup_path" which --toolchain "$RUST_TOOLCHAIN_VERSION" rustc 2>/dev/null)" || \
            toolchain_error "failed to resolve rustc from rustup toolchain $RUST_TOOLCHAIN_VERSION" || return 1
        rustdoc_path="$("$rustup_path" which --toolchain "$RUST_TOOLCHAIN_VERSION" rustdoc 2>/dev/null)" || \
            toolchain_error "failed to resolve rustdoc from rustup toolchain $RUST_TOOLCHAIN_VERSION" || return 1

        toolchain_dir="$(toolchain_bin_directory "$cargo_path")" || return 1
        [[ "$toolchain_dir" == "$(toolchain_bin_directory "$rustc_path")" && "$toolchain_dir" == "$(toolchain_bin_directory "$rustdoc_path")" ]] || \
            toolchain_error "resolved cargo, rustc, and rustdoc must share one directory" || return 1
        RUST_TOOLCHAIN_DIR="$toolchain_dir"
    fi

    toolchain_validate_binary "$cargo_path" cargo || return 1
    toolchain_validate_binary "$rustc_path" rustc || return 1
    toolchain_validate_binary "$rustdoc_path" rustdoc || return 1

    export RUST_TOOLCHAIN_DIR
    export RUST_TOOLCHAIN_CARGO="$cargo_path"
    export RUST_TOOLCHAIN_CARGO_PLUGIN_DIR
    export RUSTC="$rustc_path"
    export RUSTDOC="$rustdoc_path"
}

toolchain_resolve
