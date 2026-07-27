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
    'ReactNativeProseEditor.podspec',
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
    read('ReactNativeProseEditor.podspec'),
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
assert.match(
    read('react-native.config.js'),
    /ReactNativeProseEditor\.podspec[\s\S]*ReactNativeProseEditorSpec\/provider/,
    'React Native config must keep package-root podspec discovery documented with the codegen provider',
);
assert.doesNotMatch(read('expo-module.config.json'), /NativeProseViewer/);

assert.ok(exists('ReactNativeProseEditor.podspec'), 'package-root podspec is required for Expo/RN codegen discovery');
assert.ok(!exists('ios/ReactNativeProseEditor.podspec'), 'legacy nested podspec must not shadow package-root codegen discovery');
assert.ok(!exists('ios/build/generated'), 'consumer-generated iOS codegen output must not be checked into the package');
const packageJson = JSON.parse(read('package.json'));
assert.ok(packageJson.files.includes('ReactNativeProseEditor.podspec'), 'npm package must include the root podspec');
assert.ok(!packageJson.files.includes('ios/*.podspec'), 'npm package must not publish a nested podspec');
assert.match(
    read('expo-module.config.json'),
    /"podspecPath"\s*:\s*"\.\/ReactNativeProseEditor\.podspec"/,
    'Expo autolinking must point to the package-root podspec',
);
assert.deepEqual(packageJson.codegenConfig, {
    name: 'ReactNativeProseEditorSpec',
    type: 'components',
    jsSrcsDir: 'src/specs',
    android: { javaPackageName: 'com.apollohg.editor.viewer' },
    ios: { componentProvider: { PreparedProseViewer: 'PREPPreparedProseViewerComponentView' } },
});

const viewerPodspec = read('ReactNativeProseEditor.podspec');
assert.doesNotMatch(
    viewerPodspec,
    /s\.module_name\s*=/,
    'React Native codegen owns the PreparedProseViewer Swift compatibility module name',
);
assert.match(
    viewerPodspec,
    /s\.private_header_files\s*=\s*'ios\/Viewer\/Fabric\/PREPPreparedProseViewerComponentView\.h'/,
    'the Fabric C++ component header must remain private to the pod implementation',
);
assert.match(viewerPodspec, /s\.source_files\s*=\s*\['ios\/\*\.swift', 'ios\/Viewer\/\*\*\/\*\.\{swift,h,mm\}', 'common\/cpp\/\*\*\/\*\.\{h,cpp\}'\]/);
for (const path of [
    'ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm',
    'ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm',
]) {
    const implementation = read(path);
    assert.match(
        implementation,
        /#if __has_include\("react_renderer_components_PreparedProseViewer-Swift\.h"\)\s*\n#import "react_renderer_components_PreparedProseViewer-Swift\.h"\s*\n#else\s*\n#error "PreparedProseViewer codegen is stale or mismatched: expected react_renderer_components_PreparedProseViewer-Swift\.h"\s*\n#endif/,
        `${path} must import the exact React Native codegen Swift compatibility header and reject stale codegen`,
    );
    assert.doesNotMatch(
        implementation,
        /ReactNativeProseEditor-Swift\.h/,
        `${path} must not import the obsolete pod-name Swift compatibility header`,
    );
}
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
