import { expect, it } from 'vitest';
import fixture from './fixture.json' with { type: 'json' };
import type { Snapshot } from '../src/shared/lib/types';
import { translator } from '../src/shared/lib/i18n';
import { miningTitle } from '../src/app/title';
import { confirmedPercent } from '../src/shared/lib/progress';

it('uses confirmed progress and replaces stale titles for pause, idle, disconnect and logout', () => {
  const data = structuredClone(fixture) as Snapshot;
  const t = translator({});
  const title = () => miningTitle(data, true, t);
  expect(title()).toBe('70% Rust - Drops Miner');
  data.current_drop!.current_minutes = 59;
  expect(title()).toBe('70% Rust - Drops Miner');
  data.current_drop!.confirmed_at = null;
  expect(title()).toBe('Rust - Drops Miner');
  data.settings.mining_paused = true;
  expect(title()).toBe('Paused - Drops Miner');
  expect(miningTitle(data, false, t)).toBe('Disconnected - Drops Miner');
  data.settings.mining_paused = false;
  data.mining!.state = 'paused';
  expect(title()).toBe('Paused - Drops Miner');
  data.mining!.state = 'waiting_channel';
  expect(title()).toBe('Idle - Drops Miner');
  data.mining!.state = 'manual_watching';
  data.current_drop = null;
  expect(title()).toBe('Watching northwind - Drops Miner');
  data.login.user_id = null;
  expect(title()).toBe('Drops Miner');
  expect(miningTitle(null, false, t)).toBe('Drops Miner');
});

it('keeps exact integer percentages without rounding incomplete progress up', () => {
  const data = structuredClone(fixture) as Snapshot;
  const t = translator({});
  data.current_drop!.required_minutes = 100;
  for (const minutes of [0, 29, 58, 99, 100]) {
    data.current_drop!.confirmed_minutes = minutes;
    expect(miningTitle(data, true, t)).toBe(`${minutes}% Rust - Drops Miner`);
  }
  data.current_drop!.confirmed_minutes = 59;
  data.current_drop!.required_minutes = 60;
  expect(miningTitle(data, true, t)).toBe('98% Rust - Drops Miner');
});

it('omits percentages without confirmed minutes and a valid watch requirement', () => {
  expect(confirmedPercent(null)).toBeNull();
  for (const patch of [
    { confirmed_at: null },
    { confirmed_minutes: undefined },
    { confirmed_minutes: Number.NaN },
    { required_minutes: 0 },
    { required_minutes: Number.POSITIVE_INFINITY },
  ])
    expect(confirmedPercent({ ...fixture.current_drop, ...patch })).toBeNull();
});
