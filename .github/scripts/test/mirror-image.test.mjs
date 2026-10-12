import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { mirrorImage } from '../mirror-image.mjs';

const image = 'ghcr.io/ohne-b/twitch-drops-miner';
const raw = JSON.stringify({ manifests: ['amd64', 'arm64'].map(architecture => ({ platform: { os: 'linux', architecture } })) });
const digest = `sha256:${createHash('sha256').update(raw).digest('hex')}`;

function commands(t, version, failure) {
  const environment = { GITHUB_REPOSITORY: 'ohne-b/twitch-drops-miner', GITHUB_REF: 'refs/heads/main',
    DOCKERHUB_IMAGE: 'test-user/twitch-drops-miner', DOCKERHUB_USERNAME: 'test-user', DOCKERHUB_TOKEN: 'secret-token' };
  for (const [key, value] of Object.entries(environment)) {
    const previous = process.env[key];
    process.env[key] = value;
    t.after(() => { if (previous === undefined) delete process.env[key]; else process.env[key] = previous; });
  }
  const calls = [];
  t.mock.method(childProcess, 'execFileSync', (command, args, options) => {
    calls.push({ command, args, options });
    if (command === 'gh') return JSON.stringify({ tag_name: args.at(-1).endsWith('/latest') && failure === 'older release' ? 'v9.0.0' : `v${version}`,
      draft: failure === 'draft', prerelease: version.includes('-') });
    if (args[0] === 'inspect') {
      if (failure === 'missing platform') return '{}';
      if (failure === 'mirror mismatch' && args.at(-1).includes('docker.io/')) return '{}';
      if (failure === 'latest moved' && args.at(-1) === `docker://${image}:latest`) return '{}';
      return raw;
    }
    if (args[0] === 'copy' && failure === 'copy failed') throw new Error('copy failed');
    return '';
  });
  return calls;
}

test('stable mirrors preserve the full image digest and promote latest only after verification', t => {
  const calls = commands(t, '1.4.2');
  mirrorImage('1.4.2');
  const copies = calls.filter(call => call.args[0] === 'copy');
  assert.deepEqual(copies.map(call => call.args), ['1.4.2', 'latest'].map(tag => [
    'copy', '--all', '--preserve-digests', '--retry-times', '3',
    `docker://${image}@${digest}`, `docker://docker.io/test-user/twitch-drops-miner:${tag}`,
  ]));
  const login = calls.find(call => call.args[0] === 'login');
  assert.equal(login.options.input, 'secret-token');
  assert.ok(calls.every(call => !call.args.includes('secret-token') && ['gh', 'skopeo'].includes(call.command)));
  const verified = calls.findIndex(call => call.args[0] === 'inspect' && call.args.at(-1).endsWith('docker.io/test-user/twitch-drops-miner:1.4.2'));
  assert.ok(verified < calls.indexOf(copies[1]));
});

for (const [version, failure] of [['1.4.1', 'older release'], ['1.5.0-rc.1+build.2'], ['edge'], ['1.4.2', 'latest moved']]) {
  test(`${version} (${failure ?? 'no promotion'}) cannot advance latest`, t => {
    const calls = commands(t, version, failure);
    mirrorImage(version);
    const copies = calls.filter(call => call.args[0] === 'copy');
    assert.equal(copies.length, 1);
    assert.ok(copies[0].args.at(-1).endsWith(`:${version.replace('+', '_')}`));
  });
}

for (const failure of ['draft', 'missing platform', 'copy failed', 'mirror mismatch']) {
  test(`${failure} fails without promoting latest`, t => {
    const calls = commands(t, '1.4.2', failure);
    assert.throws(() => mirrorImage('1.4.2'));
    assert.ok(calls.every(call => call.args[0] !== 'copy' || !call.args.at(-1).endsWith(':latest')));
    if (failure === 'draft' || failure === 'missing platform') assert.ok(calls.every(call => call.args[0] !== 'login'));
  });
}

test('mirror rejects invalid input, destinations, missing secrets and non-main execution before commands', t => {
  const calls = commands(t, '1.4.2');
  for (const version of ['latest', 'v1.4.2', '1.4.2;bad', undefined]) assert.throws(() => mirrorImage(version));
  for (const image of ['example.com/user/repo', '-bad/repo', 'user/repo:latest', 'user/repo\n']) {
    process.env.DOCKERHUB_IMAGE = image;
    assert.throws(() => mirrorImage('1.4.2'));
  }
  process.env.DOCKERHUB_IMAGE = 'test-user/twitch-drops-miner';
  process.env.GITHUB_REF = 'refs/pull/1/merge';
  assert.throws(() => mirrorImage('1.4.2'));
  process.env.GITHUB_REF = 'refs/heads/main';
  delete process.env.DOCKERHUB_TOKEN;
  assert.throws(() => mirrorImage('1.4.2'));
  assert.equal(calls.length, 0);
});

test('mirroring runs after publication or manually on main, with isolated credentials and no build', () => {
  const workflows = new URL('../../workflows/', import.meta.url);
  const mirror = readFileSync(new URL('docker-mirror.yml', workflows), 'utf8');
  for (const name of ['docker-release.yml', 'docker-edge.yml']) {
    const caller = readFileSync(new URL(name, workflows), 'utf8');
    assert.match(caller, /mirror:\n    needs: publish\n    uses: \.\/\.github\/workflows\/docker-mirror.yml/);
    const mirrorJob = caller.split('\n  mirror:')[1];
    assert.match(mirrorJob, /\n    secrets:\n      DOCKERHUB_TOKEN: \$\{\{ secrets\.DOCKERHUB_TOKEN \}\}/);
    assert.doesNotMatch(caller.split('\n  mirror:')[0], /DOCKERHUB_TOKEN/);
    assert.doesNotMatch(mirrorJob, /secrets: inherit|RELEASE_TOKEN|TAURI_SIGNING_PRIVATE_KEY/);
  }
  assert.match(mirror, /workflow_dispatch:/);
  assert.match(mirror, /workflow_call:/);
  assert.match(mirror.split('workflow_call:')[1], /\n    secrets:\n      DOCKERHUB_TOKEN:\n        required: true/);
  assert.match(mirror, /github\.repository == 'ohne-b\/twitch-drops-miner' && github\.ref == 'refs\/heads\/main'/);
  assert.match(mirror, /environment: prod/);
  assert.match(mirror, /secrets\.DOCKERHUB_TOKEN/);
  assert.match(mirror, /group: docker-hub-mirror\n  cancel-in-progress: false/);
  assert.doesNotMatch(mirror, /contents: write|packages: write|build-push-action|cargo |docker build|gh release create/);
});
