import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const repoRoot = path.resolve(import.meta.dirname, '../..');
const packageJson = JSON.parse(await readFile(path.join(repoRoot, 'package.json'), 'utf8'));
const babelConfigSource = await readFile(path.join(repoRoot, 'babel.config.js'), 'utf8');
const jestConfigSource = await readFile(path.join(repoRoot, 'jest.config.cjs'), 'utf8');
const publishWorkflow = await readFile(path.join(repoRoot, '.github/workflows/publish.yml'), 'utf8');
const packedFixtureSource = await readFile(
  path.join(repoRoot, 'scripts/tests/validate-packed-package.test.mjs'),
  'utf8',
);
const packedValidatorSource = await readFile(
  path.join(repoRoot, 'scripts/validate-packed-package.sh'),
  'utf8',
);
const distTagResolver = path.join(repoRoot, 'scripts/resolve-npm-dist-tag.mjs');

const jobsSource = publishWorkflow.slice(publishWorkflow.search(/^jobs:\s*$/m));
const jobIndent = jobsSource.match(/^(\s+)[a-z0-9-]+:\s*$/m)?.[1];
assert.ok(jobIndent, 'publish workflow must contain jobs');
const jobHeaders = [
  ...jobsSource.matchAll(new RegExp(`^${jobIndent}([a-z0-9-]+):\\s*$`, 'gm')),
];
const requireJob = (jobName) => {
  const index = jobHeaders.findIndex((match) => match[1] === jobName);
  assert.notEqual(index, -1, `publish workflow must define ${jobName}`);
  const start = jobHeaders[index].index;
  const end = jobHeaders[index + 1]?.index ?? publishWorkflow.length;
  return jobsSource.slice(start, end);
};

assert.match(
  publishWorkflow,
  /^permissions:\s*\n\s+contents:\s*read\s*$/m,
  'workflow-level permissions must be read-only',
);

const buildJob = requireJob('build-package');
assert.match(buildJob, /runs-on:\s*macos-[^\s]+/);
assert.match(buildJob, /cargo install cargo-ndk --version 4\.1\.2 --locked/);
assert.match(buildJob, /sdkmanager --install ['"]ndk;27\.1\.12297006['"]/);
assert.match(buildJob, /actions\/upload-artifact@v4/);
assert.match(buildJob, /release-artifact\/\*\.tgz/);

for (const jobName of [
  'package-contracts',
  'security-rust-typescript',
  'security-ios',
  'security-android',
  'ios-consumer-positive',
  'ios-consumer-negative',
  'android-consumer-positive',
  'android-consumer-negative',
]) {
  const job = requireJob(jobName);
  assert.match(
    job,
    /actions\/download-artifact@v4/,
    `${jobName} must download the release artifact`,
  );
}

for (const jobName of [
  'security-android',
  'android-consumer-positive',
  'android-consumer-negative',
]) {
  const job = requireJob(jobName);
  assert.match(job, /android-actions\/setup-android@v4/);
  assert.match(job, /sdkmanager --install ['"]ndk;27\.1\.12297006['"]/);
}

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
]) {
  assert.match(
    publishJob,
    new RegExp(`- ${dependency}\\b`),
    `publish must require ${dependency}`,
  );
}
assert.match(publishJob, /id-token:\s*write/);
assert.match(publishJob, /actions\/download-artifact@v4/);
assert.match(publishJob, /npm publish "\$tarball" --ignore-scripts --tag/);
assert.doesNotMatch(publishJob, /npm run (?:build|validate:package|build:rust)/);

assert.match(packedFixtureSource, /VALIDATE_PACKED_PACKAGE_GROUP/);
assert.match(packedFixtureSource, /ios-consumer/);
assert.match(packedFixtureSource, /android-consumer/);
assert.match(packedValidatorSource, /--validate-packed-tarball/);
assert.match(packedValidatorSource, /--validate-android-tarball/);
for (const script of [
  'validate:package:contracts',
  'validate:package:ios:positive',
  'validate:package:ios:negative',
  'validate:package:android:positive',
  'validate:package:android:negative',
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

const packageTagResult = spawnSync(process.execPath, [distTagResolver], {
  cwd: repoRoot,
  encoding: 'utf8',
});
assert.equal(packageTagResult.status, 0, packageTagResult.stderr);
assert.equal(packageTagResult.stdout.trim(), 'alpha', 'the current package must publish with the alpha tag');

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
