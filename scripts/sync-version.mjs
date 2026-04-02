import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

const rootPackagePath = path.join(repoRoot, 'package.json');
const rootPackage = JSON.parse(readFileSync(rootPackagePath, 'utf8'));
const version = rootPackage.version;

function writeJson(relativePath, updater) {
  const filePath = path.join(repoRoot, relativePath);
  const json = JSON.parse(readFileSync(filePath, 'utf8'));
  updater(json);
  writeFileSync(filePath, `${JSON.stringify(json, null, 2)}\n`);
}

function writeText(relativePath, updater) {
  const filePath = path.join(repoRoot, relativePath);
  const current = readFileSync(filePath, 'utf8');
  const next = updater(current);
  if (next !== current) {
    writeFileSync(filePath, next);
  }
}

writeJson('package-lock.json', (json) => {
  json.version = version;
  if (json.packages?.['']) {
    json.packages[''].version = version;
  }
});

writeJson('example/package.json', (json) => {
  json.version = version;
});

writeJson('example/package-lock.json', (json) => {
  json.version = version;
  if (json.packages?.['']) {
    json.packages[''].version = version;
  }
  if (json.packages?.['..']) {
    json.packages['..'].version = version;
  }
});

writeText('rust/editor-core/Cargo.toml', (text) =>
  text.replace(
    /(\[package\][\s\S]*?^version = ")([^"]+)(")/m,
    `$1${version}$3`
  )
);

writeText('rust/editor-core/Cargo.lock', (text) =>
  text.replace(
    /(\[\[package\]\]\nname = "editor-core"\nversion = ")([^"]+)(")/,
    `$1${version}$3`
  )
);

writeText('example/ios/NativeEditorExample.xcodeproj/project.pbxproj', (text) =>
  text.replace(/MARKETING_VERSION = [^;]+;/g, `MARKETING_VERSION = ${version};`)
);

writeText('example/ios/Podfile.lock', (text) =>
  text.replace(
    /ReactNativeProseEditor \([^)]+\)/g,
    `ReactNativeProseEditor (${version})`
  )
);
