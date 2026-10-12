import { afterEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { connectDesktop, desktopRequest, nativeRequest } from '../src/shared/lib/desktop';
import fixture from './fixture.json' with { type: 'json' };
import { newerUpdate, type UpdateStatus } from '../src/features/settings/DesktopUpdates';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(() => vi.clearAllMocks());

it('keeps completed update status when an earlier progress response arrives late', () => {
  const ready = { revision: 8, phase: 'ready' } as UpdateStatus;
  expect(newerUpdate(ready, { ...ready, revision: 7, phase: 'downloading' })).toBe(ready);
  expect(newerUpdate(ready, { ...ready, revision: 9, phase: 'installing' }).phase).toBe(
    'installing',
  );
});

it('routes only supported native requests and preserves history filters', () => {
  expect(nativeRequest('/api/oauth/confirm', {}, 'POST')).toEqual({ kind: 'confirm_oauth' });
  expect(nativeRequest('/api/settings', { mining_paused: true }, 'POST')).toEqual({
    kind: 'save_settings',
    value: { mining_paused: true },
  });
  expect(nativeRequest('/api/history?campaign_id=one&limit=25', undefined, 'GET')).toEqual({
    kind: 'history',
    value: { campaign_id: 'one', limit: 25 },
  });
  expect(nativeRequest('/api/telegram', undefined, 'GET')).toEqual({ kind: 'telegram' });
  expect(nativeRequest('/api/telegram/bot', undefined, 'GET')).toEqual({ kind: 'telegram_bot' });
  expect(nativeRequest('/api/telegram', { revision: 'r', token: 'fake' }, 'POST')).toEqual({
    kind: 'save_telegram',
    value: { revision: 'r', token: 'fake' },
  });
  expect(nativeRequest('/api/telegram/test', {}, 'POST')).toEqual({ kind: 'test_telegram' });
  expect(() => nativeRequest('/api/auth/settings', {}, 'POST')).toThrow();
  expect(() => nativeRequest('/api/history?limit=NaN', undefined, 'GET')).toThrow();
});

it('cancels native requests and rejects late search results', async () => {
  let resolve!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'app_request'
      ? new Promise((done) => {
          resolve = done;
        })
      : Promise.resolve(),
  );
  const controller = new AbortController();
  const pending = desktopRequest('/api/games', { query: 'Rust' }, 'POST', controller.signal);
  controller.abort();
  resolve([]);
  await expect(pending).rejects.toHaveProperty('name', 'AbortError');
  expect(invoke).toHaveBeenCalledWith(
    'cancel_request',
    expect.objectContaining({ id: expect.any(String) }),
  );
});

it('does not let a closed subscription replace the remounted dashboard', async () => {
  let finish!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation((command) => {
    if (command === 'state_open')
      return new Promise((done) => {
        finish = done;
      });
    if (command === 'state_next') return new Promise(() => {});
    return Promise.resolve();
  });
  const first = vi.fn();
  const second = vi.fn();
  const old = connectDesktop(first);
  await vi.waitFor(() => expect(finish).toBeTypeOf('function'));
  old.close();
  const current = connectDesktop(second);
  finish({ type: 'snapshot', value: fixture });
  await vi.waitFor(() =>
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'state_open')).toHaveLength(2),
  );
  finish({ type: 'snapshot', value: fixture });
  await vi.waitFor(() => expect(second).toHaveBeenCalledTimes(1));
  expect(first).not.toHaveBeenCalled();
  current.close();
});
