#!/usr/bin/env node

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = resolve(import.meta.dirname, '../..');
const temporary = mkdtempSync(join(tmpdir(), 'native-editor-security-negative-'));
try {
    const fixture = JSON.parse(
        readFileSync(resolve(root, 'scripts/tests/security-contract-fixtures.json'), 'utf8')
    );
    fixture.trickleDeadline.expectedOutcome = 'success';
    fixture.trickleDeadline.expectedTerminalMs += 1;
    const mutatedPath = join(temporary, 'mutated-fixtures.json');
    writeFileSync(mutatedPath, JSON.stringify(fixture));

    const result = spawnSync(
        process.execPath,
        ['scripts/tests/validate-security-contracts.mjs', '--behavior'],
        {
            cwd: root,
            env: {
                ...process.env,
                SECURITY_FIXTURE_PATH: mutatedPath,
                SECURITY_BEHAVIOR_TARGETS: 'android',
            },
            encoding: 'utf8',
        }
    );
    assert.notEqual(result.status, 0, 'security validator accepted mutated trickle outcomes');
    const output = `${result.stdout}\n${result.stderr}`;
    assert.match(output, /shared whitespace base64 and trickle fixtures.*FAILED|AssertionError/is);
    assert.doesNotMatch(output, /gradle-8\.14\.3-bin\.zip\.lck/);
    console.log('Security behavior gate negative fixture was rejected.');
} finally {
    rmSync(temporary, { recursive: true, force: true });
}
