import { readFileSync } from 'node:fs';

const version =
  process.argv[2] ?? JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;
const semver =
  /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-([0-9A-Za-z-]+)(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const match = semver.exec(version);

if (!match) {
  throw new Error(`Cannot resolve an npm dist-tag from invalid version: ${version}`);
}

const prereleaseChannel = match[1];
const distTag = prereleaseChannel == null ? 'latest' : /^\d+$/.test(prereleaseChannel) ? 'next' : prereleaseChannel;

process.stdout.write(`${distTag}\n`);
