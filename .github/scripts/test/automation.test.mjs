import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { bumpVersion, readVersion, validateVersion, isPrerelease, releaseImages, releaseManifest, releaseNotes, writeReleaseArtifacts } from '../release.mjs';

test('release versions use SemVer precedence and reject shell syntax and noncanonical input', () => {
  assert.equal(validateVersion('2.0.0-rc.10', '2.0.0-rc.2'), '2.0.0-rc.10');
  assert.equal(validateVersion('2.0.0+build.1', '1.0.0'), '2.0.0+build.1');
  assert.equal(validateVersion('2.0.0-rc.1+build.1', '1.0.0'), '2.0.0-rc.1+build.1');
  for (const version of ['2.0.0', '2.0.0+build-name']) assert.equal(isPrerelease(version), false);
  for (const version of ['2.0.0-rc.1', '2.0.0-rc.1+build-name']) assert.equal(isPrerelease(version), true);
  for (const version of ['v2.0.0', '02.0.0', '2.0', '2.0.0\n', '2.0.0;true', '$(id)', '1.0.0', '0.9.9']) {
    assert.throws(() => validateVersion(version, '1.0.0'));
  }
});

test('Cargo metadata validates and updates both version files without changing dependencies', () => {
  const directory = mkdtempSync(join(tmpdir(), 'tdm-release-'));
  try {
    mkdirSync(join(directory, 'src'));
    writeFileSync(join(directory, 'src/main.rs'), 'fn main() {}\n');
    writeFileSync(join(directory, 'Cargo.toml'), '[package]\nname = "twitch-drops-miner"\nversion.workspace = true\nedition = "2024"\n\n[workspace.package]\nversion = "1.0.0"\n');
    execFileSync('cargo', ['generate-lockfile', '--offline'], { cwd: directory });
    assert.equal(readVersion(directory), '1.0.0');
    writeFileSync(join(directory, 'CHANGELOG.md'), releaseEntry('1.0.0'));
    writeReleaseArtifacts('1.0.0', directory);
    assert.deepEqual(JSON.parse(readFileSync(join(directory, 'dist/release/latest.json'), 'utf8')), releaseManifest('1.0.0'));
    assert.match(readFileSync(join(directory, 'dist/release/notes.md'), 'utf8'), /commits\/v1\.0\.0/);
    assert.throws(() => writeReleaseArtifacts('2.0.0', directory));
    assert.equal(bumpVersion('1.1.0-rc.1+build.1', directory), '1.1.0-rc.1+build.1');
    assert.match(readFileSync(join(directory, 'Cargo.lock'), 'utf8'), /version = "1.1.0-rc.1\+build.1"/);
    const lock = readFileSync(join(directory, 'Cargo.lock'), 'utf8');
    writeFileSync(join(directory, 'Cargo.lock'), lock.replace('1.1.0-rc.1', '0.0.1'));
    assert.throws(() => readVersion(directory));
    assert.equal(readFileSync(join(directory, 'Cargo.lock'), 'utf8'), lock.replace('1.1.0-rc.1', '0.0.1'));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

const releaseEntry = version => `## [v${version}](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v${version}) - 2026-09-26\n\n- a reviewed change\n`;
test('primary release images use GHCR and encode SemVer build metadata in Docker tags', () => {
  assert.deepEqual(releaseImages('0.1.0'), ['ghcr.io/ohne-b/twitch-drops-miner:0.1.0']);
  assert.deepEqual(releaseImages('0.2.0-rc.1+build.1'), [
    'ghcr.io/ohne-b/twitch-drops-miner:0.2.0-rc.1_build.1',
  ]);
  assert.throws(() => releaseImages('0.1.0;bad'));
  const tags = execFileSync(process.execPath, [fileURLToPath(new URL('../release.mjs', import.meta.url)), 'images', '0.1.0'], {
    encoding: 'utf8', env: { ...process.env, DOCKERHUB_IMAGE: 'example/legacy-image' },
  });
  assert.equal(tags.trim(), 'ghcr.io/ohne-b/twitch-drops-miner:0.1.0');
});
test('every release has matching metadata and concise reviewed notes, compatible metadata for server update notices', () => {
  assert.deepEqual(releaseManifest('0.1.0'), {
    schemaVersion: 1,
    version: '0.1.0',
    notes: '[Check release notes on GitHub](https://github.com/ohne-b/twitch-drops-miner/releases/tag/v0.1.0)',
  });
  assert.equal(releaseManifest('0.2.0-rc.1').version, '0.2.0-rc.1');
  const notes = releaseNotes('0.2.0', releaseEntry('0.2.0') + '\n' + releaseEntry('0.1.0'));
  assert.match(notes, /^- a reviewed change\n/);
  assert.match(notes, /^Changelog: https:\/\/github\.com\/ohne-b\/twitch-drops-miner\/compare\/v0\.1\.0\.\.\.v0\.2\.0$/m);
  assert.match(notes, /issues\/new/);
  assert.equal(notes.match(/a reviewed change/g).length, 1);
  for (const changelog of ['', releaseEntry('0.1.0').repeat(2), releaseEntry('0.1.0').replace('- a reviewed change', ''), releaseEntry('0.1.0') + releaseEntry('1.0.0')]) {
    assert.throws(() => releaseNotes('0.1.0', changelog));
  }
});

test('repository release notes contain one generated changelog link per version', () => {
  const changelog = readFileSync(new URL('../../../CHANGELOG.md', import.meta.url), 'utf8');
  const versions = [...changelog.matchAll(/^## \[v([^\]]+)\]/gm)].map(match => match[1]);
  assert.ok(versions.length > 0);
  for (const [index, version] of versions.entries()) {
    const notes = releaseNotes(version, changelog);
    const previous = versions[index + 1];
    const path = previous ? `compare/v${previous}...v${version}` : `commits/v${version}`;
    const link = `https://github.com/ohne-b/twitch-drops-miner/${path}`;
    assert.ok(notes.includes(`\nChangelog: ${link}\n`), version);
    assert.equal(notes.split(link).length - 1, 1, version);
    assert.doesNotMatch(notes, /\[Compare |Full Changelog:/);
  }
});

test('English catalog covers production message keys and contains plain text', () => {
  const root = new URL('../../../', import.meta.url);
  const dictionary = JSON.parse(readFileSync(new URL('lang/English.json', root), 'utf8'));
  const lookup = key => key.split('.').reduce((value, part) => value?.[part], dictionary);
  for (const folder of ['src/', 'crates/core/src/', 'desktop/src/', 'frontend/src/']) {
    const base = new URL(folder, root);
    for (const name of readdirSync(base, { recursive: true }).filter(name => /\.(rs|tsx?)$/.test(name))) {
      const source = readFileSync(new URL(name.replaceAll('\\', '/'), base), 'utf8');
      for (const match of source.matchAll(/['"]((?:gui|login|status)\.[a-z_]+(?:\.[a-z_]+)*)['"]/g)) {
        assert.equal(typeof lookup(match[1]), 'string', `${name}: missing ${match[1]}`);
      }
    }
  }
  function check(value) {
    for (const text of Object.values(value)) {
      if (typeof text === 'object') check(text);
      else assert.ok(typeof text === 'string' && !/[<>]|\p{Extended_Pictographic}/u.test(text));
    }
  }
  check(dictionary);
});

test('edge publishing is manual, validated-main-only and cannot advance release tags', () => {
  const workflows = new URL('../../workflows/', import.meta.url);
  const edge = readFileSync(new URL('docker-edge.yml', workflows), 'utf8');
  const validation = readFileSync(new URL('validation.yml', workflows), 'utf8');
  const release = readFileSync(new URL('docker-release.yml', workflows), 'utf8');
  assert.match(edge, /on:\n  workflow_dispatch:\n\npermissions:/);
  assert.match(edge, /if: github\.ref == 'refs\/heads\/main'/);
  assert.match(edge, /ref: \$\{\{ github\.sha \}\}/);
  assert.match(edge, /run: npm --prefix frontend ci/);
  assert.match(edge, /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(edge, /node \.github\/scripts\/publish-images\.mjs "\$VERSION" ghcr\.io\/ohne-b\/twitch-drops-miner:edge/);
  assert.doesNotMatch(edge, /contents: write|:latest|gh release|git push/);
  assert.doesNotMatch(edge.split('\n  mirror:')[0], /secrets\./);
  const buildx = /docker\/setup-buildx-action@[a-f0-9]+/;
  assert.equal(edge.match(buildx)?.[0], validation.match(buildx)?.[0]);
  assert.equal(edge.match(buildx)?.[0], release.match(buildx)?.[0]);
  assert.doesNotMatch(edge + release, /build-push-action|setup-qemu-action|cargo build/);
  assert.match(release, /node \.github\/scripts\/publish-images\.mjs "\$RELEASE_VERSION" "\$IMAGE_TAGS"/);
  assert.match(validation, /outputs: type=oci,dest=\$\{\{ runner.temp \}\}\/image.tar/);
  assert.match(validation, /skopeo --override-arch "\$ARCH" copy "oci-archive:\$RUNNER_TEMP\/image.tar" docker-daemon:twitch-drops-miner:test/);
  assert.ok(validation.indexOf('Check runtime, ownership and health') < validation.indexOf('Retain the tested image'));
  assert.match(validation, /if: github.ref == 'refs\/heads\/main' && github.event_name != 'pull_request'/);
  assert.match(validation, /retention-days: 7/);
  assert.doesNotMatch(validation, /needs: test|packages: write|push: true/);
});


test('release attribution uses the verified owner token while registry credentials stay separate', () => {
  const workflow = readFileSync(new URL('../../workflows/docker-release.yml', import.meta.url), 'utf8');
  const verify = workflow.split('- name: Verify release publisher')[1].split('- uses:')[0];
  const images = workflow.split('- name: Publish the validated images')[1].split('- name:')[0];
  const release = workflow.split('- name: Publish release with manifest and reviewed notes')[1];
  assert.ok(verify.includes('GH_TOKEN: ${{ secrets.RELEASE_TOKEN }}'));
  assert.ok(verify.includes('test -n "$GH_TOKEN"'));
  assert.ok(verify.includes('publisher=$(gh api user --jq .login)'));
  assert.ok(verify.includes('test "$publisher" = "$RELEASE_OWNER"'));
  assert.ok(workflow.indexOf('Verify release publisher') < workflow.indexOf('Publish the validated images'));
  assert.ok(images.includes('GH_TOKEN: ${{ github.token }}'));
  assert.ok(release.includes('GH_TOKEN: ${{ secrets.RELEASE_TOKEN }}'));
  assert.doesNotMatch(images, /secrets\./);
  assert.doesNotMatch(workflow, /contents: write/);
});
