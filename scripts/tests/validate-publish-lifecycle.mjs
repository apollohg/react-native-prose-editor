import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const packageJson = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
const publishWorkflow = await readFile(path.join(repoRoot, '.github/workflows/publish.yml'), 'utf8');

const requireWorkflow = (pattern, message) => {
  assert.match(publishWorkflow, pattern, message);
};

requireWorkflow(
  /publish:\s*\n\s*runs-on:\s*macos-[^\s]+/,
  'publish job must run on a macOS runner so it can build iOS package consumers',
);
requireWorkflow(
  /publish:\s*\n(?:.*\n)*?\s+timeout-minutes:\s*(?:6[0-9]|[7-9][0-9]|[1-9][0-9]{2,})\b/,
  'publish job must allow at least 60 minutes for native build and package-consumer validation',
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
const podInstallRunPattern = /^(\s*)run:\s*pod\s+install(?:\s+[^\r\n]*)?$/;
const npmPublishRunPattern = /^\s*run:\s*npm\s+publish(?:\s+[^\r\n]*)?$/;
const podInstallLineIndex = workflowLines.findIndex((line) => podInstallRunPattern.test(line));
const publishLineIndex = workflowLines.findIndex((line) => npmPublishRunPattern.test(line));
assert.notEqual(podInstallLineIndex, -1, 'publish job must install CocoaPods dependencies for example/ios');
assert.notEqual(publishLineIndex, -1, 'publish job must publish the package to npm');

const podInstallIndent = podInstallRunPattern.exec(workflowLines[podInstallLineIndex])[1].length;
let podInstallStepStart = -1;
let podInstallStepIndent = -1;
for (let lineIndex = podInstallLineIndex - 1; lineIndex >= 0; lineIndex -= 1) {
  const listItem = /^(\s*)-\s+/.exec(workflowLines[lineIndex]);
  if (listItem && listItem[1].length < podInstallIndent) {
    podInstallStepStart = lineIndex;
    podInstallStepIndent = listItem[1].length;
    break;
  }
}
assert.notEqual(podInstallStepStart, -1, 'CocoaPods install run line must belong to a YAML step');

let podInstallStepEnd = workflowLines.length;
for (let lineIndex = podInstallLineIndex + 1; lineIndex < workflowLines.length; lineIndex += 1) {
  const listItem = /^(\s*)-\s+/.exec(workflowLines[lineIndex]);
  if (listItem && listItem[1].length <= podInstallStepIndent) {
    podInstallStepEnd = lineIndex;
    break;
  }
}
const podInstallStep = workflowLines.slice(podInstallStepStart, podInstallStepEnd).join('\n');
assert.match(
  podInstallStep,
  /^\s*working-directory:\s*example\/ios\s*$/m,
  'CocoaPods install must run in example/ios',
);
assert.ok(podInstallLineIndex < publishLineIndex, 'example/ios CocoaPods install must run before npm publish');

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
