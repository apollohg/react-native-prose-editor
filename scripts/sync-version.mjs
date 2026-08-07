import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

function writeJson(root, relativePath, updater) {
  const filePath = path.join(root, relativePath);
  const json = JSON.parse(readFileSync(filePath, 'utf8'));
  updater(json);
  writeFileSync(filePath, `${JSON.stringify(json, null, 2)}\n`);
}

function replaceText(root, relativePath, updater) {
  const filePath = path.join(root, relativePath);
  const current = readFileSync(filePath, 'utf8');
  const next = updater(current);
  if (next !== current) {
    writeFileSync(filePath, next);
  }
}

export function synchronizeVersion(root, version) {
  writeJson(root, 'package-lock.json', (json) => {
    json.version = version;
    if (json.packages?.['']) {
      json.packages[''].version = version;
    }
  });

  writeJson(root, 'example/package.json', (json) => {
    json.version = version;
  });

  writeJson(root, 'example/package-lock.json', (json) => {
    json.version = version;
    if (json.packages?.['']) {
      json.packages[''].version = version;
    }
    if (json.packages?.['..']) {
      json.packages['..'].version = version;
    }
  });

  replaceText(root, 'rust/editor-core/Cargo.toml', (text) =>
    text.replace(
      /(\[package\][\s\S]*?^version = ")([^"]+)(")/m,
      `$1${version}$3`
    )
  );

  replaceText(root, 'rust/editor-core/Cargo.lock', (text) =>
    text.replace(
      /(\[\[package\]\]\nname = "editor-core"\nversion = ")([^"]+)(")/,
      `$1${version}$3`
    )
  );

}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const rootPackagePath = path.join(repoRoot, 'package.json');
  const rootPackage = JSON.parse(readFileSync(rootPackagePath, 'utf8'));
  synchronizeVersion(repoRoot, rootPackage.version);
}
