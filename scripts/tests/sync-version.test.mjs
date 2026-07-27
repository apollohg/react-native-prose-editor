import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { synchronizeVersion } from '../sync-version.mjs';

const fixture = await mkdtemp(path.join(tmpdir(), 'native-editor-sync-version-'));

async function write(relativePath, contents) {
  const target = path.join(fixture, relativePath);
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, contents);
}

try {
  await write('package-lock.json', '{"version":"1.0.0","packages":{"":{"version":"1.0.0"}}}\n');
  await write('example/package.json', '{"version":"1.0.0"}\n');
  await write('example/package-lock.json', '{"version":"1.0.0","packages":{"":{"version":"1.0.0"},"..":{"version":"1.0.0"}}}\n');
  await write('rust/editor-core/Cargo.toml', '[package]\nname = "editor-core"\nversion = "1.0.0"\n');
  await write('rust/editor-core/Cargo.lock', '[[package]]\nname = "editor-core"\nversion = "1.0.0"\n');
  await write('example/ios/NativeEditorExample.xcodeproj/project.pbxproj', 'MARKETING_VERSION = 1.0.0;\n');
  await write(
    'example/ios/Podfile.lock',
    [
      'PODS:',
      '  - ReactNativeProseEditor (1.0.0):',
      '    - ExpoModulesCore',
      'DEPENDENCIES:',
      '  - ReactNativeProseEditor (from `../../ios`)',
      'EXTERNAL SOURCES:',
      '  ReactNativeProseEditor:',
      '    :path: "../../ios"',
      'SPEC CHECKSUMS:',
      '  ReactNativeProseEditor: preserved-checksum',
    ].join('\n'),
  );

  synchronizeVersion(fixture, '3.0.0');

  const podfileLock = await readFile(path.join(fixture, 'example/ios/Podfile.lock'), 'utf8');
  assert.match(podfileLock, /^  - ReactNativeProseEditor \(3\.0\.0\):$/m);
  assert.match(podfileLock, /^  - ReactNativeProseEditor \(from `\.\.\/\.\.\/ios`\)$/m);
  assert.match(podfileLock, /^    :path: "\.\.\/\.\.\/ios"$/m);
  assert.match(podfileLock, /^  ReactNativeProseEditor: preserved-checksum$/m);
} finally {
  await rm(fixture, { recursive: true, force: true });
}

console.log('Version synchronization preserves the local CocoaPods dependency record.');
