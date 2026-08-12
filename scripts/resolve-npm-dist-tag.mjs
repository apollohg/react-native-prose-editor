import { readFileSync } from 'node:fs';

const packageJsonUrl = new URL('../package.json', import.meta.url);
const packageVersion = JSON.parse(readFileSync(packageJsonUrl, 'utf8')).version;
const version = process.argv[2] ?? packageVersion;
const semver =
  /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-([0-9A-Za-z-]+)(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const match = semver.exec(version);

if (!match) {
  throw new Error(`Cannot resolve an npm dist-tag from invalid version: ${version}`);
}

const prereleaseChannel = match[1];
const distTag = prereleaseChannel == null ? 'latest' : /^\d+$/.test(prereleaseChannel) ? 'next' : prereleaseChannel;

process.stdout.write(`${distTag}\n`);
