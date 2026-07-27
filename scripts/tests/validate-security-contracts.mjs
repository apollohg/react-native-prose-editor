#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import {
    assertPinnedCargoBehaviorSpawnFixture,
    runSecurityBehaviorCommands,
    securityBehaviorCommands,
} from './security-behavior-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const releaseMode = process.argv.includes('--release');
const behaviorMode = process.argv.includes('--behavior');
const pinnedCargo = resolve(root, 'rust', 'toolchain-cargo.sh');

const resourceDefaults = {
    maxInputBytes: 20 * 1024 * 1024,
    maxDocumentNodes: 100_000,
    maxDocumentDepth: 256,
    maxSchemaNodes: 1_024,
    maxSchemaExpressionBytes: 64 * 1024,
    maxCollaborationMessageBytes: 10 * 1024 * 1024,
    maxEncodedStateBytes: 50 * 1024 * 1024,
};
const resourceCeilings = {
    maxInputBytes: 64 * 1024 * 1024,
    maxDocumentNodes: 1_000_000,
    maxDocumentDepth: 1_024,
    maxSchemaNodes: 10_000,
    maxSchemaExpressionBytes: 1024 * 1024,
    maxCollaborationMessageBytes: 64 * 1024 * 1024,
    maxEncodedStateBytes: 256 * 1024 * 1024,
};
const imageDefaults = {
    maxSourceBytes: 10 * 1024 * 1024,
    connectTimeoutMs: 10_000,
    readTimeoutMs: 20_000,
    requestTimeoutMs: 60_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 64,
    maxDecodeDimensionPx: 2_048,
};
const imageCeilings = {
    maxSourceBytes: 64 * 1024 * 1024,
    connectTimeoutMs: 600_000,
    readTimeoutMs: 600_000,
    requestTimeoutMs: 600_000,
    maxConcurrentRequests: 16,
    maxPendingRequests: 512,
    maxDecodeDimensionPx: 8_192,
};

function evaluateInteger(expression) {
    assert.match(expression, /^[\d_\s*+()]+$/, `unsafe integer expression: ${expression}`);
    // Expressions come from checked-in policy constants and contain integers/operators only.
    return Function(`"use strict"; return (${expression.replaceAll('_', '')});`)();
}

function evaluateRustInteger(source, expression) {
    if (/^[A-Z][A-Z0-9_]*$/.test(expression)) {
        const match = source.match(
            new RegExp(`(?:pub\\(crate\\))?const${expression}:usize=([\\d_*+]+);`)
        );
        assert.ok(match, `Rust integer constant missing for ${expression}`);
        expression = match[1];
    }
    return evaluateInteger(expression);
}

function objectAssignments(source, anchor, mapping = {}) {
    const start = source.indexOf(anchor);
    assert.notEqual(start, -1, `missing contract anchor: ${anchor}`);
    const body = source.slice(start, source.indexOf('\n}', start) + 2);
    const values = {};
    for (const [sourceName, resultName] of Object.entries(mapping)) {
        const match = body.match(new RegExp(`${sourceName}\\s*[:=]\\s*([\\d_\\s*+()]+?)(?:,|\\n)`));
        assert.ok(match, `missing ${sourceName} after ${anchor}`);
        values[resultName] = evaluateInteger(match[1].trim());
    }
    return values;
}

function typescriptObject(path, name) {
    const source = read(path);
    return objectAssignments(
        source,
        `export const ${name}`,
        Object.fromEntries(
            [...Object.keys(resourceDefaults), ...Object.keys(imageDefaults)]
                .filter((key) => source.includes(`${key}:`))
                .map((key) => [key, key])
        )
    );
}

const tsResourceDefaults = typescriptObject(
    'src/ResourceLimits.ts',
    'DEFAULT_EDITOR_RESOURCE_LIMITS'
);
const tsResourceCeilings = typescriptObject('src/ResourceLimits.ts', 'HARD_EDITOR_RESOURCE_LIMITS');
const tsImageDefaults = typescriptObject(
    'src/ImageLoadingPolicy.ts',
    'DEFAULT_EDITOR_IMAGE_LOADING_POLICY'
);
const tsImageCeilings = typescriptObject(
    'src/ImageLoadingPolicy.ts',
    'HARD_EDITOR_IMAGE_LOADING_POLICY'
);
assert.deepEqual(tsResourceDefaults, resourceDefaults);
assert.deepEqual(tsResourceCeilings, resourceCeilings);
assert.deepEqual(tsImageDefaults, imageDefaults);
assert.deepEqual(tsImageCeilings, imageCeilings);

const rust = read('rust/editor-core/src/boundary.rs');
const compactRust = rust.replaceAll(/\s/g, '');
const rustNames = Object.fromEntries(
    Object.keys(resourceDefaults).map((name) => [
        name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`),
        name,
    ])
);
assert.deepEqual(
    objectAssignments(rust, 'impl Default for ResourceLimits', rustNames),
    resourceDefaults
);
for (const [name, ceiling] of Object.entries(resourceCeilings)) {
    const snake = name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
    const match = compactRust.match(
        new RegExp(`\\("${name}",(?:self|limits)\\.${snake},([A-Z][A-Z0-9_]*|[\\d_*+]+),?\\)`)
    );
    assert.ok(match, `Rust ceiling missing for ${name}`);
    assert.equal(evaluateRustInteger(compactRust, match[1]), ceiling, `Rust ceiling drift for ${name}`);
}

const android = read('android/src/main/java/com/apollohg/editor/SharedNativeImagePipeline.kt');
assert.deepEqual(
    objectAssignments(
        android,
        'val DEFAULT = ImageLoadingPolicy(',
        Object.fromEntries(Object.keys(imageDefaults).map((key) => [key, key]))
    ),
    imageDefaults
);
for (const [name, ceiling] of Object.entries(imageCeilings)) {
    const match = android
        .replaceAll(/\s/g, '')
        .match(new RegExp(`boundedPositiveInt\\("${name}",DEFAULT\\.${name},([\\d_*+]+)\\)`));
    assert.ok(match, `Android ceiling missing for ${name}`);
    assert.equal(evaluateInteger(match[1]), ceiling, `Android ceiling drift for ${name}`);
}

const ios = read('ios/SharedNativeImagePipeline.swift');
const iosDefaults = objectAssignments(ios, 'static let `default` = ImageLoadingPolicy(', {
    maxSourceBytes: 'maxSourceBytes',
    connectTimeout: 'connectTimeoutMs',
    readTimeout: 'readTimeoutMs',
    requestTimeout: 'requestTimeoutMs',
    maxConcurrentRequests: 'maxConcurrentRequests',
    maxPendingRequests: 'maxPendingRequests',
    maxDecodeDimension: 'maxDecodeDimensionPx',
});
for (const timeout of ['connectTimeoutMs', 'readTimeoutMs', 'requestTimeoutMs'])
    iosDefaults[timeout] *= 1000;
assert.deepEqual(iosDefaults, imageDefaults);
for (const [name, ceiling] of Object.entries(imageCeilings)) {
    const field = name === 'maxDecodeDimensionPx' ? 'maxDecodeDimension' : name.replace(/Ms$/, '');
    const start = ios.indexOf(`"${name}"`);
    assert.notEqual(start, -1, `iOS ceiling missing for ${name}`);
    const nearby = ios.slice(start, start + 400);
    const match = nearby.match(/ceiling:\s*([\d_\s*+]+)/);
    assert.ok(match, `iOS ceiling missing for ${field}`);
    assert.equal(evaluateInteger(match[1].trim()), ceiling, `iOS ceiling drift for ${name}`);
}

const fixturePath = process.env.SECURITY_FIXTURE_PATH
    ? resolve(process.env.SECURITY_FIXTURE_PATH)
    : resolve(root, 'scripts/tests/security-contract-fixtures.json');
const fixtures = JSON.parse(readFileSync(fixturePath, 'utf8'));
assert.deepEqual(
    Object.keys(fixtures).sort(),
    [
        'customArticleRoot',
        'ffiV2ErrorContract',
        'missingImageSource',
        'oversizedSchema',
        'schemaNormalizationParity',
        'trickleDeadline',
        'unknownScriptMark',
        'whitespaceBase64',
    ].sort()
);
assert.equal(fixtures.oversizedSchema.nodeCount, resourceDefaults.maxSchemaNodes + 1);
assert.equal(
    fixtures.trickleDeadline.expectedTerminalMs,
    fixtures.trickleDeadline.requestTimeoutMs
);
assert.ok(/\s/.test(fixtures.whitespaceBase64.source.slice('data:image/png;base64,'.length)));
assert.ok(Array.isArray(fixtures.schemaNormalizationParity.missingFields?.nodes));
assert.ok(Array.isArray(fixtures.schemaNormalizationParity.missingFields?.marks));
assert.equal(typeof fixtures.schemaNormalizationParity.invalidNodeTag, 'string');
assert.equal(typeof fixtures.schemaNormalizationParity.invalidAttribute, 'string');

const ffiV2 = fixtures.ffiV2ErrorContract;
assert.deepEqual(ffiV2.domains, [
    'boundary',
    'document',
    'operation',
    'lifecycle',
    'snapshot',
    'transport',
]);
assert.deepEqual(ffiV2.operationCodes, [
    'ENGINE_NOT_READY',
    'REVISION_MISMATCH',
    'POSITION_INVALID',
    'TRANSACTION_INVALID',
    'OPERATION_INVALID',
    'OPERATION_LIMIT_EXCEEDED',
    'OPERATION_RESOURCE_EXHAUSTED',
    'DOCUMENT_INVALID',
    'DOCUMENT_LIMIT_EXCEEDED',
    'ENGINE_INVARIANT_FAILED',
]);
assert.equal(new Set(ffiV2.domains).size, 6);
assert.equal(new Set(ffiV2.operationCodes).size, 10);
assert.ok(ffiV2.goldenErrors.every((error) => ffiV2.domains.includes(error.domain)));
assert.ok(ffiV2.goldenErrors.every((error) => typeof error.code === 'string'));
for (const code of ffiV2.operationCodes) {
    const golden = ffiV2.goldenErrors.filter((error) => error.code === code);
    assert.equal(golden.length, 1, `expected one golden FFI v2 error for ${code}`);
    assert.equal(golden[0].domain, ffiV2.operationCodeDomains[code]);
    assert.match(golden[0].requestId, /^(0|[1-9]\d*)$/);
}
assert.ok(ffiV2.invalidRequestIds.every((requestId) => !/^(0|[1-9]\d*)$/.test(requestId)));
assert.deepEqual(
    ffiV2.deterministicMappings.map(({ expectedCode }) => expectedCode),
    ['OPERATION_LIMIT_EXCEEDED', 'DOCUMENT_LIMIT_EXCEEDED', 'OPERATION_RESOURCE_EXHAUSTED']
);

const session = read('rust/editor-core/src/session.rs');
const collaborationNames = Object.fromEntries(
    Object.keys(ffiV2.collaborationLimits.defaults).map((name) => [
        name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`),
        name,
    ])
);
assert.deepEqual(
    objectAssignments(session, 'impl Default for CollaborationLimits', collaborationNames),
    ffiV2.collaborationLimits.defaults
);
assert.deepEqual(
    objectAssignments(session, 'pub(crate) const fn hard_ceiling()', collaborationNames),
    ffiV2.collaborationLimits.ceilings
);

const ffiTypes = read('rust/editor-core/src/ffi_v2/types.rs');
for (const domain of ffiV2.domains) assert.ok(ffiTypes.includes(`"${domain}"`));
for (const code of ffiV2.operationCodes) assert.ok(ffiTypes.includes(`"${code}"`));
assert.match(ffiTypes, /Some\(true\)/, 'unit success must cross UniFFI as Some(true)');

// Android production keeps JNA in its Android AAR form. The host JVM test
// runtime resolves the ordinary JAR in an isolated configuration for JNA's
// platform-specific jnidispatch resource. It must neither publish that JAR
// nor permit the Android AAR variant to collapse the host runtime.
const androidBuild = read('android/build.gradle');
assert.match(androidBuild, /^\s*api "net\.java\.dev\.jna:jna:5\.18\.1@aar"\s*$/m);
assert.match(
    androidBuild,
    /hostTestJna\s*\{\s*canBeConsumed\s*=\s*false\s*canBeResolved\s*=\s*true\s*transitive\s*=\s*false\s*\}/s,
    'host JNA must resolve in a non-consumable, non-transitive test-only configuration'
);
assert.match(androidBuild, /^\s*hostTestJna "net\.java\.dev\.jna:jna:5\.18\.1@jar"\s*$/m);
assert.doesNotMatch(
    androidBuild,
    /^\s*testRuntimeOnly files\(configurations\.hostTestJna\)\s*$/m,
    'the raw host JNA JAR must bypass AGP dependency transformation'
);
assert.match(
    androidBuild,
    /name\.endsWith\('UnitTestRuntimeClasspath'\)[\s\S]*?exclude group:\s*'net\.java\.dev\.jna', module:\s*'jna'/,
    'JVM unit-test runtime must exclude the Android JNA component before receiving the raw host JAR'
);
assert.match(
    androidBuild,
    /def pinnedCargoScript = new File\(repositoryRoot, 'rust\/toolchain-cargo\.sh'\)\.absolutePath/,
    'the host editor_core build must pin Cargo to the checked-in wrapper'
);
assert.match(
    androidBuild,
    /def hostCargoCommand = hostOsName\.contains\('windows'\) \? \['bash', pinnedCargoScript\] : \[pinnedCargoScript\]/,
    'the host editor_core command must execute the pinned Cargo wrapper on every host platform'
);
assert.match(
    androidBuild,
    /tasks\.register\('buildHostEditorCore', Exec\)\s*\{[\s\S]*?environment 'CARGO_TARGET_DIR', hostRustTargetDirectory\.absolutePath[\s\S]*?commandLine\(\*\(hostCargoCommand \+ \[[\s\S]*?'build',[\s\S]*?'--manifest-path', new File\(repositoryRoot, 'rust\/editor-core\/Cargo\.toml'\)\.absolutePath,[\s\S]*?'--release',[\s\S]*?\]\)\)/,
    'JVM tests must build editor_core through the pinned Cargo command into an isolated release target with the editor-core manifest'
);
assert.match(
    androidBuild,
    /def hostTestJnaClasspath = files\(configurations\.hostTestJna\)[\s\S]*?tasks\.withType\(Test\)\.configureEach\s*\{[\s\S]*?dependsOn tasks\.named\('buildHostEditorCore'\)[\s\S]*?inputs\.files\(hostTestJnaClasspath\)\.withPropertyName\('hostTestJnaClasspath'\)[\s\S]*?uniffi\.component\.editor_core\.libraryOverride[\s\S]*?doFirst\s*\{\s*classpath = hostTestJnaClasspath\.plus\(classpath\)/,
    'only JVM Test tasks must prepend the raw host JNA JAR after AGP creates the runtime classpath and set the UniFFI host-library override'
);
assert.doesNotMatch(
    androidBuild,
    /^\s*(?:api|implementation|runtimeOnly|testRuntimeOnly) "net\.java\.dev\.jna:jna:5\.18\.1(?:@jar)?"\s*$/m,
    'the ordinary JNA JAR must remain exclusive to the JVM test runtime'
);

// This captures the actual command passed to spawn, rather than examining this
// file's text. It is intentionally part of every security-validation entry.
assertPinnedCargoBehaviorSpawnFixture({ root, pinnedCargo });

// Task 16C: the legacy UDL and the legacy editor_*/collaboration_session_*
// exports were deleted; the production surface is the 31 editor_v2_*
// UniFFI functions plus editor_core_version.
const V2_EXPORT_COUNT = 31;
if (releaseMode) {
    const androidModule = read('android/src/main/java/com/apollohg/editor/NativeEditorModule.kt');
    const iosModule = read('ios/NativeEditorModule.swift');
    assert.match(androidModule, /Function\("editorV2Create"\)/);
    assert.doesNotMatch(androidModule, /Function\("editorCreate"\)/);
    assert.doesNotMatch(androidModule, /\beditorCreate\(/);
    assert.doesNotMatch(androidModule, /\bcollaborationSession[A-Z]/);
    assert.match(iosModule, /Function\("editorV2Create"\)/);
    assert.doesNotMatch(iosModule, /Function\("editorCreate"\)/);
    assert.doesNotMatch(iosModule, /\beditorCreate\(/);
    assert.doesNotMatch(iosModule, /\bcollaborationSession[A-Z]/);
    assert.doesNotMatch(read('src/NativeEditorBridge.ts'), /\beditorCreate\(/);
    const productionTransportSources = [
        read('src/NativeEditorBridge.ts'),
        read('src/YjsCollaboration.ts'),
        read('src/NativeRichTextEditor.tsx'),
        read('src/useNativeEditor.ts'),
    ].join('\n');
    for (const obsolete of [
        'createWebSocket',
        'collaborationTakeOutbound',
        'drainOutbound',
        'collaborationGeneration',
        'outboundFrameSink',
        'onLocalDocumentCommit',
    ]) {
        assert.ok(
            !productionTransportSources.includes(obsolete),
            `production TypeScript must not contain obsolete collaboration API ${obsolete}`
        );
    }
    assert.doesNotMatch(
        productionTransportSources,
        /\bnew\s+WebSocket\s*\(/,
        'production TypeScript must not construct the collaboration WebSocket'
    );
    assert.match(androidModule, /Function\("editorV2CollaborationConfigureTransport"\)/);
    assert.match(androidModule, /Events\("onCollaborationTransportEvent"\)/);
    assert.match(iosModule, /Function\("editorV2CollaborationConfigureTransport"\)/);
    assert.match(iosModule, /Events\("onCollaborationTransportEvent"\)/);
    const kotlinBindings = read('rust/bindings/kotlin/uniffi/editor_core/editor_core.kt');
    assert.match(kotlinBindings, /fun `?editorV2Create`?\(/);
    assert.doesNotMatch(kotlinBindings, /uniffi_editor_core_fn_func_editor_create/);
    assert.doesNotMatch(kotlinBindings, /uniffi_editor_core_fn_func_collaboration_session/);
    const swiftBindings = read('ios/Generated_editor_core.swift');
    assert.match(swiftBindings, /func editorV2Create\(/);
    assert.doesNotMatch(swiftBindings, /uniffi_editor_core_fn_func_editor_create/);
    assert.doesNotMatch(swiftBindings, /uniffi_editor_core_fn_func_collaboration_session/);
    const ffiHeader = read('ios/editor_coreFFI/editor_coreFFI.h');
    assert.equal(
        (ffiHeader.match(/uniffi_editor_core_fn_func_editor_v2_/g) ?? []).length,
        V2_EXPORT_COUNT,
        'the FFI header must expose exactly 31 editor_v2_* symbols'
    );
    assert.match(ffiHeader, /uniffi_editor_core_fn_func_editor_core_version/);
    assert.doesNotMatch(ffiHeader, /uniffi_editor_core_fn_func_collaboration_session/);
    for (const obsoleteExport of [
        'editor_v2_collaboration_begin_connect',
        'editor_v2_collaboration_take_outbound',
        'editor_v2_collaboration_tick',
    ]) {
        assert.ok(
            !ffiHeader.includes(`uniffi_editor_core_fn_func_${obsoleteExport}`),
            `FFI header must not expose ${obsoleteExport}`
        );
    }
}

if (behaviorMode) {
    const environment = { ...process.env, SECURITY_FIXTURE_PATH: fixturePath };
    const selectedTargets = new Set(
        (process.env.SECURITY_BEHAVIOR_TARGETS ?? 'rust,typescript,android,ios').split(',')
    );
    runSecurityBehaviorCommands({
        commands: securityBehaviorCommands({ root, pinnedCargo }),
        selectedTargets,
        environment,
        spawn: spawnSync,
    });
}

const validationScope = behaviorMode
    ? 'Security contracts and hostile fixture behavior are consistent across TypeScript, Rust, Android, and iOS'
    : 'Security contract source constants and hostile fixture definitions are consistent';
console.log(`${validationScope}${releaseMode ? ', including release artifacts' : ''}.`);
