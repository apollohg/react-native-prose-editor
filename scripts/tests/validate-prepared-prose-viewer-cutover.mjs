#!/usr/bin/env node

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const exists = (path) => existsSync(resolve(root, path));
const manifest = JSON.parse(read('scripts/package-abi-manifest.json')).preparedProseViewer;

assert.equal(manifest.architecture, 'fabric-only');
assert.equal(manifest.component, 'PreparedProseViewer');
assert.equal(manifest.nativeFacade, 'ProseViewerSource + ProseViewerConfiguration');
assert.deepEqual(manifest.forbidden, [
    'NativeProseViewerExpoView',
    'heightCache',
    'measureContentHeight',
    'renderDocumentJson',
    'renderDocumentHtml',
    'containerWidth',
    'onContentHeightChange',
    'renderJson',
    'Paper registration',
]);
assert.deepEqual(manifest.removedPaths, [
    'android/src/main/java/com/apollohg/editor/NativeProseViewerExpoView.kt',
    'android/src/test/java/com/apollohg/editor/NativeProseViewerExpoViewTest.kt',
    'android/src/test/java/com/apollohg/editor/ProseViewerViewTest.kt',
    'ios/NativeProseViewerExpoView.swift',
    'ios/Tests/ProseViewerViewTests.swift',
    'src/heightCache.ts',
    'src/__tests__/heightCache.test.ts',
]);

for (const path of manifest.removedPaths) {
    assert.ok(!exists(path), `removed legacy viewer path was restored: ${path}`);
}

// These are the production viewer boundary and its registrations. The
// validator/manifest/changelog are intentionally excluded because they retain
// negative assertions about removed APIs; editor-only height events are checked
// separately rather than treated as viewer props.
const viewerBoundary = [
    'src/index.ts',
    'src/NativeProseViewer.tsx',
    'src/specs/NativePreparedProseViewer.ts',
    'ios/NativeEditorModule.swift',
    'ios/ProseViewerView.swift',
    'android/src/main/java/com/apollohg/editor/NativeEditorModule.kt',
    'android/src/main/java/com/apollohg/editor/ProseViewerView.kt',
    'expo-module.config.json',
    'react-native.config.js',
    'ios/ReactNativeProseEditor.podspec',
    'android/build.gradle',
    'ios-tests/project.yml',
    'ios-tests/NativeEditorTests.xcodeproj/project.pbxproj',
].map((path) => [path, read(path)]);

const removedViewerBoundary = manifest.forbidden.slice(0, 5);
for (const [path, source] of viewerBoundary) {
    for (const name of removedViewerBoundary) {
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

const registrationSources = [
    read('ios/NativeEditorModule.swift'),
    read('android/src/main/java/com/apollohg/editor/NativeEditorModule.kt'),
    read('expo-module.config.json'),
    read('react-native.config.js'),
    read('ios/ReactNativeProseEditor.podspec'),
    read('android/build.gradle'),
    read('ios-tests/project.yml'),
    read('ios-tests/NativeEditorTests.xcodeproj/project.pbxproj'),
].join('\n');
assert.doesNotMatch(
    registrationSources,
    /\bPaper\b|RCTViewManager|requireNativeComponent/,
    'production registration metadata still contains a Paper viewer boundary',
);
for (const path of manifest.removedPaths) {
    const basename = path.split('/').at(-1);
    assert.ok(
        !registrationSources.includes(basename),
        `production project registration still references removed viewer file ${path}`,
    );
}
assert.match(read('package.json'), /"type": "components"/);
assert.match(read('react-native.config.js'), /PreparedProseViewerComponentDescriptor/);
assert.doesNotMatch(read('expo-module.config.json'), /NativeProseViewer/);

const viewerPodspec = read('ios/ReactNativeProseEditor.podspec');
assert.match(
    viewerPodspec,
    /s\.module_name\s*=\s*'ReactNativeProseEditor'/,
    'the pod module must match Objective-C++ imports of ReactNativeProseEditor-Swift.h',
);
assert.match(
    viewerPodspec,
    /s\.private_header_files\s*=\s*'Viewer\/Fabric\/PREPPreparedProseViewerComponentView\.h'/,
    'the Fabric C++ component header must remain private to the pod implementation',
);
const viewerPerformanceTests = read('ios/Tests/NativePerformanceTests.swift');
assert.match(
    viewerPerformanceTests,
    /String\(Double\(width\)\.bitPattern, radix: 16\)/,
    'viewer measurement cache identity must preserve the exact width bit pattern',
);
assert.doesNotMatch(
    viewerPerformanceTests,
    /String\(width\)/,
    'viewer measurement cache must not use locale-dependent CGFloat formatting',
);

console.log('Prepared prose viewer hard-cutover source contract passed.');
