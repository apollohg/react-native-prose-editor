#!/usr/bin/env bash
set -euo pipefail
package_dir="$(cd "$(dirname "$0")/.." && pwd)"
crate_dir="$package_dir/rust"
target_dir="${CARGO_TARGET_DIR:-$crate_dir/target}"
mode="${1:-all}"
cargo_binary="$(rustup which --toolchain 1.95.0 cargo)"
export RUSTC="$(rustup which --toolchain 1.95.0 rustc)"
export RUSTDOC="$(rustup which --toolchain 1.95.0 rustdoc)"
cargo() { "$cargo_binary" "$@"; }
cd "$crate_dir"

if [[ "$mode" == all || "$mode" == bindings ]]; then
    cargo build --release --locked --target-dir "$target_dir"
    cargo run --release --locked --features cli --bin uniffi-bindgen --target-dir "$target_dir" -- \
        generate --library "$target_dir/release/libnative_editor_highlighting.dylib" \
        --language swift --out-dir "$target_dir/bindings/swift"
    cargo run --release --locked --features cli --bin uniffi-bindgen --target-dir "$target_dir" -- \
        generate --library "$target_dir/release/libnative_editor_highlighting.dylib" \
        --language kotlin --no-format --out-dir "$package_dir/android/src/main/java"
    mkdir -p "$package_dir/ios/native_editor_highlightingFFI"
    cp "$target_dir/bindings/swift/native_editor_highlighting.swift" "$package_dir/ios/Generated_highlighting.swift"
    cp "$target_dir/bindings/swift/native_editor_highlightingFFI.h" "$package_dir/ios/native_editor_highlightingFFI/"
    cp "$target_dir/bindings/swift/native_editor_highlightingFFI.modulemap" "$package_dir/ios/native_editor_highlightingFFI/module.modulemap"
fi

if [[ "$mode" == all || "$mode" == ios ]]; then
    for target in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
        IPHONEOS_DEPLOYMENT_TARGET=16.4 cargo build --release --locked --target "$target" --target-dir "$target_dir"
    done
    mkdir -p "$target_dir/ios-simulator"
    lipo -create "$target_dir/aarch64-apple-ios-sim/release/libnative_editor_highlighting.a" \
        "$target_dir/x86_64-apple-ios/release/libnative_editor_highlighting.a" \
        -output "$target_dir/ios-simulator/libnative_editor_highlighting.a"
    output="$package_dir/ios/NativeEditorHighlighting.xcframework"
    if [[ -d "$output" ]]; then rm -rf "$output"; fi
    xcodebuild -create-xcframework \
        -library "$target_dir/aarch64-apple-ios/release/libnative_editor_highlighting.a" \
        -library "$target_dir/ios-simulator/libnative_editor_highlighting.a" -output "$output"
fi

if [[ "$mode" == all || "$mode" == android ]]; then
    for pair in 'aarch64-linux-android arm64-v8a' 'armv7-linux-androideabi armeabi-v7a' 'i686-linux-android x86' 'x86_64-linux-android x86_64'; do
        target="${pair%% *}"
        abi="${pair#* }"
        cargo ndk --target "$target" --platform 24 build --release --locked --target-dir "$target_dir"
        mkdir -p "$package_dir/android/src/main/jniLibs/$abi"
        cp "$target_dir/$target/release/libnative_editor_highlighting.so" "$package_dir/android/src/main/jniLibs/$abi/"
    done
fi
if [[ "$mode" != all && "$mode" != bindings && "$mode" != ios && "$mode" != android ]]; then
    echo 'Expected all, bindings, ios, or android.' >&2
    exit 2
fi
