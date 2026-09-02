import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const packageJson = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
const babelConfigSource = await readFile(path.join(repoRoot, 'babel.config.js'), 'utf8');
const jestConfigSource = await readFile(path.join(repoRoot, 'jest.config.cjs'), 'utf8');
const ciWorkflow = await readFile(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8');
const publishWorkflow = await readFile(path.join(repoRoot, '.github/workflows/publish.yml'), 'utf8');
const packedFixtureSource = await readFile(
  path.join(repoRoot, 'scripts/tests/validate-packed-package.test.mjs'),
  'utf8',
);
const packedValidatorSource = await readFile(
  path.join(repoRoot, 'scripts/validate-packed-package.sh'),
  'utf8',
);
const rn076ValidatorSource = await readFile(
  path.join(repoRoot, 'scripts/validate-android-rn076-consumer.sh'),
  'utf8',
);
const rn076ConsumerManifest = JSON.parse(
  await readFile(
    path.join(repoRoot, 'scripts/tests/android-rn076-consumer/package.json'),
    'utf8',
  ),
);
const distTagResolver = path.join(repoRoot, 'scripts/resolve-npm-dist-tag.mjs');

const workflowJob = (workflow, jobName, workflowName) => {
  const jobsSource = workflow.slice(workflow.search(/^jobs:\s*$/m));
  const jobIndent = jobsSource.match(/^(\s+)[a-z0-9-]+:\s*$/m)?.[1];
  assert.ok(jobIndent, `${workflowName} workflow must contain jobs`);
  const jobHeaders = [
    ...jobsSource.matchAll(new RegExp(`^${jobIndent}([a-z0-9-]+):\\s*$`, 'gm')),
  ];
  const index = jobHeaders.findIndex((match) => match[1] === jobName);
  assert.notEqual(index, -1, `${workflowName} workflow must define ${jobName}`);
  const start = jobHeaders[index].index;
  const end = jobHeaders[index + 1]?.index ?? jobsSource.length;
  return jobsSource.slice(start, end);
};
const requireJob = (jobName) => workflowJob(publishWorkflow, jobName, 'publish');
const requireCiJob = (jobName) => workflowJob(ciWorkflow, jobName, 'CI');

const assertRustCache = (job, jobName) => {
  assert.match(
    job,
    /uses: Swatinem\/rust-cache@v2/,
    `${jobName} must use the dependency-aware Rust cache`,
  );
  assert.match(
    job,
    /workspaces:\s*rust\/editor-core/,
    `${jobName} must cache the editor-core workspace`,
  );
  assert.match(
    job,
    /shared-key:\s*editor-core/,
    `${jobName} must share sanitized editor-core dependencies`,
  );
  assert.match(
    job,
    /cache-on-failure:\s*true/,
    `${jobName} must preserve warmed Rust dependencies after failures`,
  );
};

const assertGradleCache = (job, jobName, readOnlyExpression) => {
  assert.match(
    job,
    /uses: gradle\/actions\/setup-gradle@v5/,
    `${jobName} must use Gradle's rolling commit-aware cache`,
  );
  assert.match(
    job,
    new RegExp(`cache-read-only:\\s*${readOnlyExpression}`),
    `${jobName} must use the expected Gradle cache write policy`,
  );
};

assert.match(
  publishWorkflow,
  /^permissions:\s*\n\s+contents:\s*read\s*$/m,
  'workflow-level permissions must be read-only',
);
assert.doesNotMatch(
  `${ciWorkflow}\n${publishWorkflow}`,
  /actions\/cache(?:\/(?:restore|save))?@v4/,
  'workflow caches must not use the deprecated Node.js 20 action major',
);
assert.doesNotMatch(
  publishWorkflow,
  /actions\/(?:upload-artifact|download-artifact)@v4/,
  'release artifacts must not use deprecated Node.js 20 action majors',
);

const buildJob = requireJob('build-package');
assert.match(buildJob, /runs-on:\s*macos-[^\s]+/);
assert.match(buildJob, /cargo install cargo-ndk --version 4\.1\.2 --locked/);
assert.match(buildJob, /sdkmanager --install ['"]ndk;27\.1\.12297006['"]/);
assert.match(
  buildJob,
  /- name: Restore release build cache\s+id: release-build-cache\s+uses: actions\/cache\/restore@v5/,
  'release builds must restore exact previously rehearsed outputs',
);
assert.match(
  buildJob,
  /key: release-build-v1-\$\{\{ runner\.os \}\}-rust-1\.95\.0-ndk-4\.1\.2-android-27\.1\.12297006-\$\{\{ github\.sha \}\}/,
  'release build caches must be isolated by toolchain and exact commit',
);
for (const stepName of [
  'Setup Node',
  'Setup Java',
  'Setup Rust',
  'Cache Cargo',
  'Install cargo-ndk',
  'Setup Android SDK',
  'Setup Android NDK',
  'Install dependencies',
  'Build editor-core for all shipping targets',
  'Verify generated bindings are current',
  'Build package',
  'Pack release artifact',
]) {
  assert.match(
    buildJob,
    new RegExp(
      `- name: ${stepName}\\s+if: steps\\.release-build-cache\\.outputs\\.cache-hit != 'true'`,
    ),
    `${stepName} must be skipped when exact release outputs are restored`,
  );
}
assert.match(
  buildJob,
  /- name: Save release build cache\s+if: steps\.release-build-cache\.outputs\.cache-hit != 'true'\s+uses: actions\/cache\/save@v5/,
  'new release outputs must populate the exact commit cache',
);
assert.match(
  buildJob,
  /key: \$\{\{ steps\.release-build-cache\.outputs\.cache-primary-key \}\}/,
  'release output saves must reuse the restore primary key',
);
assert.match(
  buildJob,
  /- name: Upload release artifact\s+uses: actions\/upload-artifact@v7/,
  'restored and freshly built outputs must both be uploaded for validation',
);
assert.match(buildJob, /release-artifact\/\*\.tgz/);
assertRustCache(buildJob, 'build-package');

for (const jobName of ['js-and-android', 'ios']) {
  assertRustCache(requireCiJob(jobName), `CI ${jobName}`);
}
for (const jobName of [
  'package-contracts',
  'security-rust-typescript',
  'android-release-validation',
]) {
  assertRustCache(requireJob(jobName), jobName);
}

for (const jobName of [
  'package-contracts',
  'security-rust-typescript',
  'security-ios',
  'security-android',
  'ios-consumer-positive',
  'ios-consumer-negative',
  'android-consumer-positive',
  'android-consumer-negative',
  'android-release-validation',
]) {
  const job = requireJob(jobName);
  assert.match(
    job,
    /actions\/download-artifact@v8/,
    `${jobName} must download the release artifact`,
  );
}

const androidReleaseJob = requireJob('android-release-validation');
const ciAndroidApi24Job = requireCiJob('android-api-24');
const hostCompatibleEmulatorArchitecture =
  /arch:\s*\$\{\{\s*runner\.arch == 'ARM64' && 'arm64-v8a' \|\| 'x86_64'\s*\}\}/;
assert.match(androidReleaseJob, /runs-on:\s*macos-14/);
assert.match(androidReleaseJob, /timeout-minutes:\s*60/);
assert.match(androidReleaseJob, /sdkmanager --install ['"]ndk;27\.1\.12297006['"]/);
assert.match(androidReleaseJob, /npm run test:android/);
assert.match(androidReleaseJob, /npm run lint:android/);
assert.match(androidReleaseJob, /:apollohg_react-native-prose-editor:assembleRelease/);
assert.match(androidReleaseJob, /npm run validate:package:android:rn076/);
assert.match(androidReleaseJob, /reactivecircus\/android-emulator-runner@v2\.38\.0/);
assert.match(androidReleaseJob, /api-level:\s*24/);
assert.match(
  androidReleaseJob,
  hostCompatibleEmulatorArchitecture,
  'publish API 24 emulator architecture must match the runner host',
);
assert.match(
  ciAndroidApi24Job,
  hostCompatibleEmulatorArchitecture,
  'CI API 24 emulator architecture must match the runner host',
);
assert.match(
  ciWorkflow,
  /RELEASE_TARBALL="\$tarball" npm run validate:package:android:rn076/,
  'CI must validate the exact packed artifact against React Native 0.76',
);

for (const jobName of [
  'security-android',
  'android-consumer-positive',
  'android-consumer-negative',
]) {
  const job = requireJob(jobName);
  assert.match(job, /android-actions\/setup-android@v4/);
  assert.match(job, /sdkmanager --install ['"]ndk;27\.1\.12297006['"]/);
  assertGradleCache(job, jobName, 'false');
}

assertGradleCache(
  requireCiJob('js-and-android'),
  'CI js-and-android',
  "\\$\\{\\{ github\\.event_name == 'pull_request' \\}\\}",
);
assertGradleCache(
  ciAndroidApi24Job,
  'CI android-api-24',
  "\\$\\{\\{ github\\.event_name == 'pull_request' \\}\\}",
);
assertGradleCache(androidReleaseJob, 'android-release-validation', 'false');
assert.doesNotMatch(
  `${ciWorkflow}\n${publishWorkflow}`,
  /key: gradle-(?:publish|android-release)-/,
  'manual immutable Gradle cache keys must not return',
);

const publishJob = requireJob('publish');
for (const dependency of [
  'build-package',
  'package-contracts',
  'security-rust-typescript',
  'security-ios',
  'security-android',
  'ios-consumer-positive',
  'ios-consumer-negative',
  'android-consumer-positive',
  'android-consumer-negative',
  'android-release-validation',
]) {
  assert.match(
    publishJob,
    new RegExp(`- ${dependency}\\b`),
    `publish must require ${dependency}`,
  );
}
assert.match(publishJob, /id-token:\s*write/);
assert.match(publishJob, /actions\/download-artifact@v8/);
assert.match(
  publishJob,
  /- name: Publish to npm\s+if: github\.event_name == 'release' && github\.event\.action == 'published'/,
  'real npm publishing must only run for a published GitHub release',
);
assert.match(
  publishJob,
  /- name: Dry-run npm publish\s+if: github\.event_name == 'workflow_dispatch'/,
  'manual workflow dispatches must use the npm publish dry run',
);
assert.match(publishJob, /npm publish "\$tarball" --ignore-scripts --tag/);
assert.match(
  publishJob,
  /npm publish "\$tarball" --dry-run --ignore-scripts --tag/,
  'the manual publish rehearsal must dry-run the exact release tarball and dist-tag',
);
assert.doesNotMatch(publishJob, /npm run (?:build|validate:package|build:rust)/);

assert.match(packedFixtureSource, /VALIDATE_PACKED_PACKAGE_GROUP/);
assert.match(packedFixtureSource, /ios-consumer/);
assert.match(packedFixtureSource, /android-consumer/);
assert.match(packedValidatorSource, /--validate-packed-tarball/);
assert.match(packedValidatorSource, /--validate-android-tarball/);
assert.match(packedValidatorSource, /validate-android-rn076-consumer\.sh/);
assert.equal(rn076ConsumerManifest.dependencies?.['react-native'], '0.76.9');
assert.equal(rn076ConsumerManifest.dependencies?.react, '18.3.1');
assert.equal(rn076ConsumerManifest.dependencies?.expo, '~52.0.49');
assert.match(rn076ValidatorSource, /npm ci --ignore-scripts/);
assert.match(rn076ValidatorSource, /generate-codegen-artifacts\.js/);
assert.match(rn076ValidatorSource, /:app:assembleRelease/);
assert.match(rn076ValidatorSource, /-PnewArchEnabled=true/);
assert.match(rn076ValidatorSource, /-PreactNativeArchitectures=x86_64/);
for (const script of [
  'validate:package:contracts',
  'validate:package:ios:positive',
  'validate:package:ios:negative',
  'validate:package:android:positive',
  'validate:package:android:negative',
  'validate:package:android:rn076',
]) {
  assert.equal(
    typeof packageJson.scripts?.[script],
    'string',
    `package.json must define ${script}`,
  );
}
assert.match(
  packageJson.scripts['validate:package:android:positive'],
  /--validate-android-tarball \"\$RELEASE_TARBALL\"/,
  'positive Android validation must consume the exact release tarball',
);

for (const dependency of [
  '@expo/vector-icons',
  'babel-preset-expo',
  'expo',
  'expo-modules-core',
  'react',
  'react-native',
]) {
  assert.equal(
    typeof packageJson.devDependencies?.[dependency],
    'string',
    `clean package builds must install ${dependency}`,
  );
}
assert.match(babelConfigSource, /require\.resolve\(['"]babel-preset-expo['"]\)/);
assert.doesNotMatch(
  babelConfigSource,
  /example\/node_modules/,
  'root tests must not depend on an example app install',
);
assert.doesNotMatch(
  jestConfigSource,
  /EXAMPLE_MODULES/,
  'Jest must resolve runtime modules from the root install',
);

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

const artifactOnlyResolver = await mkdtemp(path.join(tmpdir(), 'native-editor-dist-tag-'));
try {
  const artifactScripts = path.join(artifactOnlyResolver, 'scripts');
  const artifactResolver = path.join(artifactScripts, 'resolve-npm-dist-tag.mjs');
  await mkdir(artifactScripts);
  await copyFile(distTagResolver, artifactResolver);
  const result = spawnSync(process.execPath, [artifactResolver, '1.0.0-beta.2'], {
    cwd: artifactOnlyResolver,
    encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), 'beta');
} finally {
  await rm(artifactOnlyResolver, { recursive: true, force: true });
}

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
