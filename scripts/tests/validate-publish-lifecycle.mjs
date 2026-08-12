import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const packageJson = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
const publishWorkflow = await readFile(path.join(repoRoot, '.github/workflows/publish.yml'), 'utf8');
const distTagResolver = path.join(repoRoot, 'scripts/resolve-npm-dist-tag.mjs');

const requireWorkflow = (pattern, message) => {
  assert.match(publishWorkflow, pattern, message);
};

requireWorkflow(
  /publish:\s*\n\s*runs-on:\s*macos-[^\s]+/,
  'publish job must run on a macOS runner so it can build iOS package consumers',
);
const publishTimeout = publishWorkflow.match(/publish:\s*\n(?:.*\n)*?\s+timeout-minutes:\s*([0-9]+)\b/);
assert.ok(publishTimeout, 'publish job must define a timeout');
assert.ok(
  Number(publishTimeout[1]) >= 90,
  'publish job must allow at least 90 minutes for native build and package-consumer validation',
);
requireWorkflow(
  /uses:\s*actions\/setup-java@v5\s*\n\s+with:\s*\n\s+distribution:\s*temurin\s*\n\s+java-version:\s*17\b/,
  'publish job must install Temurin Java 17 for Android package consumers',
);
requireWorkflow(
  /uses:\s*dtolnay\/rust-toolchain@1\.95\.0\s*\n\s+with:\s*\n\s+targets:\s*aarch64-apple-ios,\s*aarch64-apple-ios-sim,\s*x86_64-apple-ios,\s*aarch64-linux-android,\s*armv7-linux-androideabi,\s*i686-linux-android,\s*x86_64-linux-android\b/,
  'publish job must install Rust 1.95.0 with every iOS and Android build target',
);
requireWorkflow(
  /cargo install cargo-ndk --version 4\.1\.2 --locked/,
  'publish job must install the pinned cargo-ndk version',
);
requireWorkflow(
  /uses:\s*android-actions\/setup-android@v4/,
  'publish job must configure the Android SDK',
);
requireWorkflow(
  /sdkmanager --install ["']ndk;27\.1\.12297006["']/,
  'publish job must install Android NDK 27.1.12297006',
);
requireWorkflow(
  /ANDROID_NDK_HOME=\$\{ANDROID_HOME\}\/ndk\/27\.1\.12297006/,
  'publish job must export ANDROID_NDK_HOME for cargo-ndk',
);

const workflowLines = publishWorkflow.split(/\r?\n/);
const npmTagStepPattern = /^\s*id:\s*npm-dist-tag\s*$/;
const npmPublishRunPattern =
  /^\s*run:\s*npm\s+publish\s+--ignore-scripts\s+--tag\s+"\$\{\{\s*steps\.npm-dist-tag\.outputs\.tag\s*\}\}"\s*$/;
const packageValidateRunPattern = /^\s*run:\s*npm\s+run\s+validate:package\s*$/;
const packageValidateLineIndex = workflowLines.findIndex((line) => packageValidateRunPattern.test(line));
const npmTagStepLineIndex = workflowLines.findIndex((line) => npmTagStepPattern.test(line));
const publishLineIndex = workflowLines.findIndex((line) => npmPublishRunPattern.test(line));
assert.notEqual(packageValidateLineIndex, -1, 'publish job must validate the generated native consumers');
assert.notEqual(npmTagStepLineIndex, -1, 'publish job must resolve an explicit npm dist-tag');
assert.notEqual(
  publishLineIndex,
  -1,
  'publish job must use the resolved dist-tag with lifecycle scripts disabled after the explicit release gate',
);
assert.ok(packageValidateLineIndex < npmTagStepLineIndex, 'native package validation must run before tag resolution');
assert.ok(npmTagStepLineIndex < publishLineIndex, 'npm dist-tag resolution must run before npm publish');

for (const [version, expectedTag] of [
  ['1.0.0-alpha', 'alpha'],
  ['1.0.0-alpha.4', 'alpha'],
  ['1.0.0-beta.2', 'beta'],
  ['1.0.0-0', 'next'],
  ['1.0.0', 'latest'],
]) {
  const result = spawnSync(process.execPath, [distTagResolver, version], {
    cwd: repoRoot,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), expectedTag, `${version} must publish with the ${expectedTag} tag`);
}

const packageTagResult = spawnSync(process.execPath, [distTagResolver], {
  cwd: repoRoot,
  encoding: 'utf8',
});
assert.equal(packageTagResult.status, 0, packageTagResult.stderr);
assert.equal(packageTagResult.stdout.trim(), 'alpha', 'the current package must publish with the alpha tag');

assert.equal(
  packageJson.scripts?.['prepare:example:native'],
  'npm run prebuild:example && npm run install:example:pods',
  'native package validation must generate both example projects before installing pods',
);
assert.equal(
  packageJson.scripts?.['install:example:pods'],
  'cd example/ios && pod install --no-repo-update',
  'native package validation must install the generated example CocoaPods project',
);
assert.equal(
  packageJson.scripts?.['install:ios-test-pods'],
  'cd ios-tests && pod install --no-repo-update',
  'native package validation must install the iOS test workspace without updating spec repositories',
);
assert.match(
  packageJson.scripts?.['validate:package'] ?? '',
  /^npm run prepare:example:native && npm run install:ios-test-pods && /,
  'package validation must prepare generated native projects and the iOS test workspace before consuming them',
);

assert.equal(
  packageJson.scripts?.prepublishOnly,
  'npm run prepare:publish',
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
