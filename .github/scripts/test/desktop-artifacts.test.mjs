import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assets, packDesktop, targets, verifyDesktop } from '../desktop-artifacts.mjs';
import { releaseManifest } from '../release.mjs';

test('the complete desktop set keeps one universal Mac installer and platform-specific updates', () => {
  const all = targets.flatMap(target => assets('1.7.0', target));
  assert.deepEqual(all.map(asset => asset.name), [
    'twitch-drops-miner-1.7.0-windows-x64-setup.exe',
    'twitch-drops-miner-1.7.0-macos.dmg',
    'twitch-drops-miner-1.7.0-macos-universal.app.tar.gz',
    'twitch-drops-miner-1.7.0-linux-x64.AppImage',
    'twitch-drops-miner-1.7.0-linux-x64.deb',
  ]);
  const platforms = Object.fromEntries(all.flatMap(asset => asset.platforms.map(key => [key, {
    url: `https://github.com/ohne-b/twitch-drops-miner/releases/download/v1.7.0/${asset.name}`,
    signature: Buffer.from('signature fixture'.repeat(8)).toString('base64'),
  }])));
  const manifest = releaseManifest('1.7.0', platforms, '2026-10-08T12:00:00Z');
  assert.equal(manifest.schemaVersion, 1);
  assert.match(manifest.notes, /release notes/);
  assert.equal(manifest.platforms['darwin-x86_64-app'].url, manifest.platforms['darwin-aarch64-app'].url);
  assert.notEqual(manifest.platforms['linux-x86_64-appimage'].url, manifest.platforms['linux-x86_64-deb'].url);
  assert.throws(() => releaseManifest('1.7.0', { ...platforms, extra: platforms['linux-x86_64-deb'] }, '2026-10-08'));
  assert.throws(() => releaseManifest('1.7.1', platforms, '2026-10-08'));
  assert.throws(() => releaseManifest('1.7.0', platforms, 'bad date'));
});

test('Windows packaging requires exactly one NSIS installer', () => {
  const directory = mkdtempSync(join(tmpdir(), 'tdm-nsis-'));
  try {
    const bundle = join(directory, 'bundle');
    mkdirSync(join(bundle, 'nsis'), { recursive: true });
    writeFileSync(join(bundle, 'nsis', 'Drops Miner.exe'), 'fixture bundle');
    const output = join(directory, 'artifacts');
    const sha = 'a'.repeat(40);
    packDesktop('windows-x64', bundle, output, sha, '1.7.0');
    assert.equal(verifyDesktop(output, 'windows-x64', '1.7.0', sha).length, 1);
    writeFileSync(join(bundle, 'nsis', 'another.exe'), 'duplicate');
    assert.throws(() => packDesktop('windows-x64', bundle, output, sha, '1.7.0'));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('promotion rejects stale identities, tampered bytes and incomplete or duplicate packages independently', () => {
  const directory = mkdtempSync(join(tmpdir(), 'tdm-bundles-'));
  const sha = 'a'.repeat(40);
  try {
    const bundle = join(directory, 'bundle');
    for (const [folder, name] of [['appimage', 'Drops Miner.AppImage'], ['deb', 'Drops Miner.deb']]) {
      mkdirSync(join(bundle, folder), { recursive: true });
      writeFileSync(join(bundle, folder, name), `fixture ${name}`);
    }
    const output = join(directory, 'artifacts');
    packDesktop('linux-x64', bundle, output, sha, '1.7.0');
    const verify = () => verifyDesktop(output, 'linux-x64', '1.7.0', sha);
    const verified = verify();
    assert.equal(verified.length, 2);
    assert.throws(() => verifyDesktop(output, 'linux-x64', '1.7.0', 'b'.repeat(40)));
    assert.throws(() => verifyDesktop(output, 'linux-x64', '1.7.1', sha));

    const file = verified[0].file;
    const original = readFileSync(file);
    for (const [label, bytes] of [
      ['same-size tampering', Buffer.alloc(original.length, 'x')],
      ['changed size', Buffer.concat([original, Buffer.from('extra')])],
    ]) {
      writeFileSync(file, bytes);
      assert.throws(verify, label);
    }
    rmSync(file);
    assert.throws(verify, 'missing installer');
    writeFileSync(file, original);

    const manifest = join(output, 'build.json');
    const metadata = JSON.parse(readFileSync(manifest, 'utf8'));
    for (const [label, files] of [
      ['duplicate package', [metadata.files[0], metadata.files[0]]],
      ['missing package', [metadata.files[0], { ...metadata.files[1], name: 'unrelated.deb' }]],
      ['extra package', [...metadata.files, { ...metadata.files[0], name: 'unrelated.deb' }]],
    ]) {
      writeFileSync(manifest, JSON.stringify({ ...metadata, files }));
      assert.throws(verify, label);
    }
    writeFileSync(manifest, JSON.stringify(metadata));
    assert.deepEqual(verify(), verified);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
