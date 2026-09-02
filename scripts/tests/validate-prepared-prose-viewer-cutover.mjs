#!/usr/bin/env node

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const requireFromRoot = createRequire(resolve(root, 'package.json'));
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
    'src/specs/PreparedProseViewerNativeComponent.ts',
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
const fabricSpec = read('src/specs/PreparedProseViewerNativeComponent.ts');
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
const reactNativeConfig = requireFromRoot('./react-native.config.js');
assert.ok(
    reactNativeConfig?.dependency?.platforms?.ios && typeof reactNativeConfig.dependency.platforms.ios === 'object',
    'React Native config must enable iOS autolinking with an ios platform object',
);
assert.deepEqual(
    reactNativeConfig.dependency.platforms.ios,
    {},
    'React Native config must not disable or redirect package-root iOS autolinking',
);
assert.doesNotMatch(read('expo-module.config.json'), /NativeProseViewer/);
const expoModuleConfig = JSON.parse(read('expo-module.config.json'));
assert.deepEqual(
    {
        path: expoModuleConfig.android.path,
        gradlePath: expoModuleConfig.android.gradlePath,
    },
    {
        path: 'android/expo',
        gradlePath: 'android/expo/build.gradle',
    },
    'Expo autolinking must use the isolated Android facade project',
);
assert.match(
    read('android/expo/build.gradle'),
    /api project\(':apollohg_react-native-prose-editor'\)/,
    'the Expo facade must depend on the React Native Android project',
);
assert.ok(
    !exists('android/src/main/jni/ReactNativeProseEditorSpec.cpp'),
    'React Native codegen owns the Android TurboModule provider implementation',
);

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
const podName = viewerPodspec.match(/s\.name\s*=\s*'([^']+)'/)?.[1];
const podModuleName = viewerPodspec.match(/s\.module_name\s*=\s*'([^']+)'/)?.[1];
assert.equal(podName, 'ReactNativeProseEditor', 'Expo autolinking must resolve the expected pod name');
assert.equal(
    podModuleName,
    podName,
    'Expo imports the pod name as its Swift module, so the public pod module name must agree exactly',
);
assert.doesNotMatch(
    viewerPodspec,
    /s\.module_name\s*=\s*'react_renderer_components_PreparedProseViewer'/,
    'Fabric header_dir must not leak into the pod public Swift module identity',
);
assert.match(
    viewerPodspec,
    /s\.private_header_files\s*=\s*\[\s*'ios\/Viewer\/Fabric\/PREPPreparedProseViewerComponentView\.h',\s*'common\/cpp\/react\/renderer\/components\/PreparedProseViewer\/\*\*\/\*\.h',\s*\]/,
    'all Fabric implementation headers must remain private to the pod implementation',
);
assert.match(viewerPodspec, /s\.source_files\s*=\s*\['ios\/\*\.swift', 'ios\/Viewer\/\*\*\/\*\.\{swift,h,mm\}', 'common\/cpp\/\*\*\/\*\.\{h,cpp\}'\]/);
assert.match(
    viewerPodspec,
    /s\.header_dir\s*=\s*'react\/renderer\/components\/PreparedProseViewer'/,
    'Fabric implementation headers must remain available to the pod compiler',
);
const moduleDependencyCalls = viewerPodspec.match(/install_modules_dependencies\(s\)/g) ?? [];
assert.equal(
    moduleDependencyCalls.length,
    1,
    'React Native module dependencies must be installed exactly once',
);
const podTargetXcconfigOffset = viewerPodspec.indexOf('s.pod_target_xcconfig =');
const moduleDependencyOffset = viewerPodspec.indexOf('install_modules_dependencies(s)');
assert.ok(
    podTargetXcconfigOffset >= 0 && moduleDependencyOffset > podTargetXcconfigOffset,
    'React Native dependency installation must follow the package pod_target_xcconfig so its generated Fabric/Yoga settings are retained',
);
assert.doesNotMatch(
    viewerPodspec,
    /s\.dependency\s+['"](?:Yoga|React-Core-prebuilt)['"]|Headers\/Private\/Yoga|ReactNativeDependencies-artifacts|ReactNativeCore-artifacts/,
    'the podspec must rely on install_modules_dependencies(s), not hard-coded Yoga or prebuilt React Native paths',
);
for (const path of [
    'common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseMeasurementsManager.h',
    'common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerComponentDescriptor.h',
    'common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.h',
    'common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h',
]) {
    assert.ok(exists(path), `Fabric implementation header must remain packaged and compiled: ${path}`);
}
const measurementsManagerHeader = read('common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseMeasurementsManager.h');
assert.match(
    measurementsManagerHeader,
    /#include <react\/renderer\/core\/ReactPrimitives\.h>/,
    'PreparedProseMeasurementsManager must source SurfaceId from ReactPrimitives',
);
assert.doesNotMatch(
    measurementsManagerHeader,
    /#include <react\/renderer\/core\/SurfaceId\.h>/,
    'PreparedProseMeasurementsManager must not include the removed SurfaceId header',
);
for (const path of [
    'ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm',
    'ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm',
]) {
    const implementation = read(path);
    assert.match(
        implementation,
        /#if __has_include\("ReactNativeProseEditor-Swift\.h"\)\s*\n#import "ReactNativeProseEditor-Swift\.h"\s*\n#else\s*\n#error "ReactNativeProseEditor Swift compatibility header is unavailable; verify the pod module name and consumer codegen"\s*\n#endif/,
        `${path} must import the public pod Swift compatibility header and reject stale module identity`,
    );
    assert.doesNotMatch(
        implementation,
        /react_renderer_components_PreparedProseViewer-Swift\.h/,
        `${path} must not import a Swift compatibility header derived from the Fabric header directory`,
    );
}
const preparedProseLayout = read('ios/Viewer/PreparedProseLayout.swift');
assert.match(
    preparedProseLayout,
    /let physicalWidth = widthPoints \* scale[\s\S]*let roundedWidth = physicalWidth\.rounded\(\)[\s\S]*return Int\(roundedWidth\)/,
    'viewer layout identity must canonicalize width to physical pixels',
);
assert.match(
    preparedProseLayout,
    /let widthPixels: Int[\s\S]*self\.displayScaleBits = Double\(displayScale\)\.bitPattern/,
    'viewer layout keys must pair physical width with the exact display-scale bit pattern',
);

const androidViewerLayout = read('android/src/main/java/com/apollohg/editor/viewer/AndroidProseLayoutEngine.kt');
assert.match(
    androidViewerLayout,
    /enum class FallbackLogicalCaretAffinity \{ LEADING_NEXT, TRAILING_PREVIOUS \}/,
    'Android fallback selection must model Layout logical caret affinity explicitly',
);
assert.match(
    androidViewerLayout,
    /val logicalRuns: List<FallbackLogicalBidiRun>,\s*val outerLineBoundary: \(FallbackVisualEdge\) -> Float,\s*val primaryHorizontal: \(Int\) -> Float,\s*val secondaryHorizontal: \(Int\) -> Float,/s,
    'Android fallback geometry must retain full logical runs and both public caret providers',
);
assert.match(
    androidViewerLayout,
    /desiredTrailing == primaryIsTrailingPrevious\(offset, geometry\)/,
    'Android fallback selection must choose primary or secondary from Layout-equivalent logical affinity',
);
assert.match(
    androidViewerLayout,
    /logicalCaretHorizontal\(neighborOffset, neighbor\.affinityAt\(neighborEdge\)\)/,
    'an internal soft-wrap terminal must borrow its visual neighbour using that neighbour\'s logical affinity',
);
assert.doesNotMatch(
    androidViewerLayout,
    /run\.isRtl\s*==\s*paragraphIsRtl/,
    'Android fallback selection must not infer primary caret affinity from run/paragraph direction parity',
);

console.log('Prepared prose viewer hard-cutover source contract passed.');
