import assert from 'node:assert/strict';
import { resolve } from 'node:path';

export const SECURITY_RUST_BEHAVIOR_ARGS = Object.freeze([
    'test',
    '--manifest-path',
    'rust/editor-core/Cargo.toml',
    '--test',
    'security_contract_fixture_test',
]);

export function securityBehaviorCommands({ root, pinnedCargo }) {
    return [
        [
            'rust',
            pinnedCargo,
            [...SECURITY_RUST_BEHAVIOR_ARGS],
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
}

export function runSecurityBehaviorCommands({ commands, selectedTargets, environment, spawn }) {
    for (const [target, command, args, cwd] of commands) {
        if (!selectedTargets.has(target)) continue;
        const result = spawn(command, args, { cwd, env: environment, stdio: 'inherit' });
        assert.equal(result.status, 0, `behavior harness failed: ${command} ${args.join(' ')}`);
    }
}

/**
 * Captures the actual spawn boundary rather than inspecting the validator's
 * source. This fails if a future refactor resolves pinnedCargo but invokes
 * PATH cargo instead, without constructing a second toolchain fixture.
 */
export function assertPinnedCargoBehaviorSpawnFixture({ root, pinnedCargo }) {
    const environment = { SECURITY_FIXTURE_PATH: '/fixture/security-contracts.json' };
    const commands = securityBehaviorCommands({ root, pinnedCargo });
    const calls = [];
    const capture = (command, args, options) => {
        calls.push({ command, args, options });
        return { status: 0 };
    };

    runSecurityBehaviorCommands({
        commands,
        selectedTargets: new Set(['rust']),
        environment,
        spawn: capture,
    });

    assert.deepEqual(calls, [
        {
            command: pinnedCargo,
            args: [...SECURITY_RUST_BEHAVIOR_ARGS],
            options: { cwd: root, env: environment, stdio: 'inherit' },
        },
    ]);
    assert.throws(
        () => runSecurityBehaviorCommands({
            commands,
            selectedTargets: new Set(['rust']),
            environment,
            spawn: () => ({ status: 23 }),
        }),
        /behavior harness failed: .*toolchain-cargo\.sh test --manifest-path rust\/editor-core\/Cargo\.toml/
    );
}
