import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const packageJson = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));

assert.equal(
  packageJson.scripts?.prepublishOnly,
  'npm run publish:prepare',
  'standard npm publish must invoke the complete release gate through prepublishOnly',
);

const fixture = await mkdtemp(path.join(tmpdir(), 'native-editor-publish-lifecycle-'));
try {
  await writeFile(
    path.join(fixture, 'package.json'),
    JSON.stringify({
      name: 'native-editor-publish-lifecycle-fixture',
      version: '1.0.0',
      scripts: {
        prepublishOnly: 'node fail-gate.mjs',
        postpublish: 'node publish-reached.mjs',
      },
    }),
  );
  await writeFile(path.join(fixture, 'fail-gate.mjs'), 'process.exit(23);\n');
  await writeFile(
    path.join(fixture, 'publish-reached.mjs'),
    "import { writeFileSync } from 'node:fs'; writeFileSync('publish-reached', 'yes');\n",
  );

  const result = spawnSync('npm', ['publish', '--dry-run', '--ignore-scripts=false'], {
    cwd: fixture,
    encoding: 'utf8',
  });
  assert.notEqual(result.status, 0, 'npm publish must fail when prepublishOnly fails');
  const marker = spawnSync('test', ['-e', path.join(fixture, 'publish-reached')]);
  assert.notEqual(marker.status, 0, 'npm publish must not advance beyond a failed release gate');
} finally {
  await rm(fixture, { recursive: true, force: true });
}

console.log('npm publish lifecycle validation passed.');
