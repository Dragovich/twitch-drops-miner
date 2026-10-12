import { expect, it } from 'vitest';
import fixture from './fixture.json' with { type: 'json' };
import { reducer, initialState } from '../src/app/reducer';
import { mergeDraft, remainingChanges } from '../src/app/useAutosave';
import { displayFilters, writeFilters } from '../src/features/campaigns/query';
import type { Snapshot, StatePatch } from '../src/shared/lib/types';

const snapshot = {
  ...fixture,
  protocol: 2,
  instance: 'process-a',
  revision: 10,
  history_revision: 2,
  history_clear_revision: 1,
} as Snapshot;
it('replaces channel collections, keeps untouched campaigns and clears nullable progress', () => {
  const current = reducer(initialState, { type: 'snapshot', value: snapshot });
  const patch: StatePatch = {
    protocol: 2,
    instance: 'process-a',
    base_revision: 10,
    revision: 15,
    changes: { channels: [{ ...snapshot.channels[0]!, viewers: 777 }], current_drop: null },
  };
  const next = reducer(current, { type: 'patch', value: patch });
  expect(next.hydrated).toBe(true);
  expect(next.data?.campaigns).toBe(current.data?.campaigns);
  expect(next.data?.channels).toEqual([{ ...snapshot.channels[0]!, viewers: 777 }]);
  expect(next.data?.current_drop).toBeNull();
  expect(reducer(next, { type: 'patch', value: patch })).toBe(next);
});
it('requires a fresh snapshot after a revision gap, disconnect or process restart', () => {
  const current = reducer(initialState, { type: 'snapshot', value: snapshot });
  for (const patch of [
    { protocol: 2, instance: 'process-a', base_revision: 9, revision: 11, changes: {} },
    { protocol: 2, instance: 'process-b', base_revision: 1, revision: 2, changes: {} },
  ]) {
    const rejected = reducer(current, { type: 'patch', value: patch });
    expect(rejected.hydrated).toBe(false);
    expect(rejected.resync).toBe(true);
    expect(rejected.data).toBe(snapshot);
    expect(
      reducer(rejected, {
        type: 'snapshot',
        value: { ...snapshot, instance: 'process-b', revision: 2 },
      }).hydrated,
    ).toBe(true);
  }
  expect(reducer(current, { type: 'disconnect' }).hydrated).toBe(false);
  expect(
    reducer(current, { type: 'snapshot', value: { ...snapshot, protocol: 3 } }).incompatible,
  ).toBe(true);
});
it('merges only edited nested settings and retains edits made during an in-flight save', () => {
  const sent = { inventory_filters: { show_active: false }, mining_benefits: { BADGE: false } };
  const queued = {
    inventory_filters: { show_active: false, show_upcoming: false },
    mining_benefits: { BADGE: true },
    proxy: 'http://example.test',
  };
  expect(remainingChanges(queued, sent)).toEqual({
    inventory_filters: { show_upcoming: false },
    mining_benefits: { BADGE: true },
    proxy: 'http://example.test',
  });
  const merged = mergeDraft(snapshot.settings, sent);
  expect(merged.inventory_filters.show_active).toBe(false);
  expect(merged.inventory_filters.show_expired).toBe(
    snapshot.settings.inventory_filters.show_expired,
  );
  expect(merged.mining_benefits.EMOTE).toBe(snapshot.settings.mining_benefits.EMOTE);
});
it('round-trips shared campaign filters without losing search, sorting or detail links', () => {
  const params = new URLSearchParams(
    'q=rust&sort=ending&view=list&campaign=one&drop=reward&page=3&show_expired=1',
  );
  const filters = {
    ...snapshot.settings.inventory_filters,
    show_active: false,
    game_name_search: [],
  };
  const shared = writeFilters(params, filters);
  expect(shared.has('page')).toBe(false);
  expect(shared.has('show_expired')).toBe(false);
  expect(displayFilters(params, filters).show_expired).toBe(filters.show_expired);
  for (const key of ['q', 'sort', 'view', 'campaign', 'drop'])
    expect(shared.get(key)).toBe(params.get(key));
  expect(
    displayFilters(shared, { ...filters, show_active: true, game_name_search: ['Other'] }),
  ).toEqual(filters);
});
