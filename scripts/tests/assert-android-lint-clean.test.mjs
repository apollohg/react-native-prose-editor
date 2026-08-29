import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const repositoryRoot = path.resolve(import.meta.dirname, '../..');
const scriptPath = path.join(repositoryRoot, 'scripts/assert-android-lint-clean.mjs');

function runLintAssertion(xml) {
    const directory = mkdtempSync(path.join(tmpdir(), 'native-editor-lint-'));
    const report = path.join(directory, 'lint-results.xml');
    writeFileSync(report, xml);
    const result = spawnSync(process.execPath, [scriptPath, report], {
        cwd: repositoryRoot,
        encoding: 'utf8',
    });
    rmSync(directory, { recursive: true, force: true });
    return result;
}

test('fails for lint errors and reports their first location', () => {
    const result = runLintAssertion(`<?xml version="1.0" encoding="UTF-8"?>
<issues>
    <issue id="NewApi" severity="Error" message="Call requires API 34">
        <location file="android/src/main/java/Editor.kt" line="12" column="8" />
        <location file="android/src/main/java/Other.kt" line="4" />
    </issue>
</issues>`);

    assert.equal(result.status, 1);
    assert.match(result.stderr, /NewApi/);
    assert.match(result.stderr, /Editor\.kt:12:8/);
    assert.doesNotMatch(result.stderr, /Other\.kt/);
});

test('allows warnings while reporting them', () => {
    const result = runLintAssertion(`<?xml version="1.0" encoding="UTF-8"?>
<issues>
    <issue id="ObsoleteSdkInt" severity="Warning" message="Unnecessary SDK check">
        <location file="android/src/main/java/Compat.kt" line="9" />
    </issue>
</issues>`);

    assert.equal(result.status, 0);
    assert.match(result.stdout, /ObsoleteSdkInt/);
    assert.match(result.stdout, /Compat\.kt:9/);
});
