#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const releaseMode = process.argv.includes('--release');
const behaviorMode = process.argv.includes('--behavior');

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
    const match = rust
        .replaceAll(/\s/g, '')
        .match(new RegExp(`\\("${name}",limits\\.${snake},([\\d_*+]+),?\\)`));
    assert.ok(match, `Rust ceiling missing for ${name}`);
    assert.equal(evaluateInteger(match[1]), ceiling, `Rust ceiling drift for ${name}`);
}

const android = read('android/src/main/java/com/apollohg/editor/RenderBridge.kt');
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

const ios = read('ios/RenderBridge.swift');
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
        'missingImageSource',
        'oversizedSchema',
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

const udl = read('rust/editor-core/src/editor_core.udl');
assert.match(udl, /string editor_create_result\(string config_json\);/);
assert.match(
    udl,
    /u64 editor_create\(string config_json\);/,
    'legacy Rust export must remain for one release'
);
if (releaseMode) {
    const androidModule = read('android/src/main/java/com/apollohg/editor/NativeEditorModule.kt');
    const iosModule = read('ios/NativeEditorModule.swift');
    assert.match(androidModule, /Function\("editorCreateResult"\)/);
    assert.doesNotMatch(androidModule, /Function\("editorCreate"\)/);
    assert.doesNotMatch(androidModule, /\beditorCreate\(/);
    assert.match(iosModule, /Function\("editorCreateResult"\)/);
    assert.doesNotMatch(iosModule, /Function\("editorCreate"\)/);
    assert.doesNotMatch(iosModule, /\beditorCreate\(/);
    assert.doesNotMatch(read('src/NativeEditorBridge.ts'), /\beditorCreate\(/);
    assert.match(
        read('rust/bindings/kotlin/uniffi/editor_core/editor_core.kt'),
        /fun `?editorCreateResult`?\(/
    );
    assert.match(read('ios/Generated_editor_core.swift'), /func editorCreateResult\(/);
    assert.match(
        read('ios/editor_coreFFI/editor_coreFFI.h'),
        /uniffi_editor_core_fn_func_editor_create_result/
    );
}

if (behaviorMode) {
    const environment = { ...process.env, SECURITY_FIXTURE_PATH: fixturePath };
    const selectedTargets = new Set(
        (process.env.SECURITY_BEHAVIOR_TARGETS ?? 'rust,typescript,android,ios').split(',')
    );
    const commands = [
        [
            'rust',
            'cargo',
            [
                'test',
                '--manifest-path',
                'rust/editor-core/Cargo.toml',
                '--test',
                'security_contract_fixture_test',
            ],
            root,
        ],
        [
            'typescript',
            'npx',
            ['jest', 'src/__tests__/securityContracts.test.ts', '--runInBand', '--watchman=false'],
            root,
        ],
        [
            'android',
            './gradlew',
            [
                ':apollohg-react-native-prose-editor:testDebugUnitTest',
                '--tests',
                'com.apollohg.editor.RenderImageLoaderPolicyTest.shared whitespace base64 and trickle fixtures execute against Android boundary',
            ],
            resolve(root, 'example/android'),
        ],
        [
            'ios',
            'bash',
            [
                'scripts/run-ios-tests.sh',
                '-only-testing:NativeEditorTests/RenderBridgeTests/testSharedWhitespaceBase64AndTrickleFixturesExecuteAgainstIOSBoundary',
            ],
            root,
        ],
    ];
    for (const [target, command, args, cwd] of commands) {
        if (!selectedTargets.has(target)) continue;
        const result = spawnSync(command, args, { cwd, env: environment, stdio: 'inherit' });
        assert.equal(result.status, 0, `behavior harness failed: ${command} ${args.join(' ')}`);
    }
}

const validationScope = behaviorMode
    ? 'Security contracts and hostile fixture behavior are consistent across TypeScript, Rust, Android, and iOS'
    : 'Security contract source constants and hostile fixture definitions are consistent';
console.log(`${validationScope}${releaseMode ? ', including release artifacts' : ''}.`);
