#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const manifest = JSON.parse(read('scripts/package-abi-manifest.json')).preparedProseViewer;

assert.deepEqual(manifest, {
    architecture: 'fabric-only',
    component: 'PreparedProseViewer',
    nativeFacade: 'ProseViewerSource + ProseViewerConfiguration',
    forbidden: [
        'NativeProseViewerExpoView',
        'heightCache',
        'measureContentHeight',
        'renderDocumentJson',
        'renderDocumentHtml',
        'containerWidth',
        'onContentHeightChange',
        'renderJson',
        'Paper registration',
    ],
});

// These paths are the shipped viewer boundary. Historical changelog entries,
// negative-validation fixtures, and editable-editor height events are
// intentionally outside this scan.
const viewerBoundary = [
    'src/NativeProseViewer.tsx',
    'src/specs/NativePreparedProseViewer.ts',
    'ios/NativeEditorModule.swift',
    'ios/ProseViewerView.swift',
    'android/src/main/java/com/apollohg/editor/NativeEditorModule.kt',
    'android/src/main/java/com/apollohg/editor/ProseViewerView.kt',
    'expo-module.config.json',
    'react-native.config.js',
].map((path) => [path, read(path)]);

for (const [path, source] of viewerBoundary) {
    for (const name of [
        'NativeProseViewerExpoView',
        'heightCache',
        'measureContentHeight',
        'renderDocumentJson',
        'renderDocumentHtml',
    ]) {
        assert.ok(!source.includes(name), `${path} still exposes removed viewer boundary ${name}`);
    }
}

const jsViewer = read('src/NativeProseViewer.tsx');
const fabricSpec = read('src/specs/NativePreparedProseViewer.ts');
for (const name of ['containerWidth', 'onContentHeightChange']) {
    assert.ok(!jsViewer.includes(name), `NativeProseViewer still exposes ${name}`);
    assert.ok(!fabricSpec.includes(name), `PreparedProseViewer spec still exposes ${name}`);
}

for (const path of [
    'ios/ProseViewerView.swift',
    'android/src/main/java/com/apollohg/editor/ProseViewerView.kt',
]) {
    assert.ok(!read(path).includes('renderJson'), `${path} still accepts a renderJson facade input`);
}

const moduleSources = [
    read('ios/NativeEditorModule.swift'),
    read('android/src/main/java/com/apollohg/editor/NativeEditorModule.kt'),
    read('expo-module.config.json'),
    read('react-native.config.js'),
].join('\n');
assert.doesNotMatch(moduleSources, /\bPaper\b|RCTViewManager|requireNativeComponent/);
assert.match(read('package.json'), /"type": "components"/);
assert.match(read('react-native.config.js'), /PreparedProseViewerComponentDescriptor/);
assert.doesNotMatch(read('expo-module.config.json'), /NativeProseViewer/);

console.log('Prepared prose viewer hard-cutover source contract passed.');
