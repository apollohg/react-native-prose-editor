import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const required = [
    'LICENSE', 'THIRD_PARTY_NOTICES.md', 'RUST-STANDARD-LIBRARY-NOTICES.html',
    'dist/index.js', 'dist/index.d.ts',
    'ios/Generated_highlighting.swift',
    'ios/native_editor_highlightingFFI/module.modulemap',
    'ios/NativeEditorHighlighting.xcframework/Info.plist',
    'ios/NativeEditorHighlighting.xcframework/ios-arm64/libnative_editor_highlighting.a',
    'ios/NativeEditorHighlighting.xcframework/ios-arm64_x86_64-simulator/libnative_editor_highlighting.a',
    'android/src/main/java/uniffi/native_editor_highlighting/native_editor_highlighting.kt',
    ...['arm64-v8a', 'armeabi-v7a', 'x86', 'x86_64'].map(
        (abi) => `android/src/main/jniLibs/${abi}/libnative_editor_highlighting.so`
    ),
];
const missing = required.filter((path) => !existsSync(`${root}/${path}`));
if (missing.length) throw new Error(`Build the package before packing. Missing: ${missing.join(', ')}`);
