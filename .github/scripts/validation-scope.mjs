import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { releaseNotes, validateVersion } from './release.mjs';

const documentation = new Set(['README.md', 'CONTRIBUTING.md', 'AGENTS.md']);
const release = new Set(['Cargo.toml', 'Cargo.lock', 'CHANGELOG.md']);

export function needsFullValidation(event, paths, read) {
  if (event !== 'pull_request' || paths.length === 0) return true;
  if (paths.every(path => documentation.has(path))) return false;
  if (!read || paths.some(path => !release.has(path)) ||
      !['Cargo.toml', 'Cargo.lock'].every(path => paths.includes(path))) return true;

  const before = read('HEAD^', 'Cargo.toml');
  const after = read('HEAD', 'Cargo.toml');
  const pattern = /(^\[workspace\.package\]\nversion = ")([^"\n]+)("$)/m;
  const previous = before.match(pattern)?.[2];
  const version = after.match(pattern)?.[2];
  if (!previous || !version ||
      after !== before.replace(pattern, (_, prefix, _version, suffix) => `${prefix}${version}${suffix}`)) return true;

  let lock = read('HEAD^', 'Cargo.lock');
  for (const name of ['twitch-drops-miner', 'twitch-drops-miner-core', 'twitch-drops-miner-desktop']) {
    const entry = `[[package]]\nname = "${name}"\nversion = "${previous}"\n`;
    if (lock.split(entry).length !== 2) return true;
    lock = lock.replace(entry, () => `[[package]]\nname = "${name}"\nversion = "${version}"\n`);
  }
  if (read('HEAD', 'Cargo.lock') !== lock) return true;
  validateVersion(version, previous);
  releaseNotes(version, read('HEAD', 'CHANGELOG.md'));
  return false;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  // Compare the tested PR merge with its base. Disabling rename detection checks both paths.
  const paths = process.env.GITHUB_EVENT_NAME === 'pull_request'
    ? execFileSync('git', ['diff', '--name-only', '--no-renames', '-z', 'HEAD^', 'HEAD', '--'], {
      encoding: 'utf8',
    }).split('\0').filter(Boolean)
    : [];
  const read = (revision, path) => execFileSync('git', ['show', `${revision}:${path}`], { encoding: 'utf8' });
  // File additions, deletions and mode changes are not version-only edits.
  const summary = paths.length ? execFileSync('git', ['diff', '--summary', '--no-renames', 'HEAD^', 'HEAD', '--'], { encoding: 'utf8' }) : '';
  console.log(`full=${needsFullValidation(process.env.GITHUB_EVENT_NAME, paths, summary ? undefined : read)}`);
}
