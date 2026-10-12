import { describe, expect, it } from 'vitest';
import fixture from './fixture.json' with { type: 'json' };
import { safeUrl, moveGame } from '../src/shared/lib/api';
import { matchesCampaign, campaignOrder } from '../src/features/campaigns/Campaigns';
import { plainText, translator } from '../src/shared/lib/i18n';
import { groupHistory, matchesHistory, historyOrder } from '../src/features/campaigns/History';
import type { HistoryEntry, Snapshot } from '../src/shared/lib/types';
const snapshot: Snapshot = {
  ...(fixture as Snapshot),
  settings: { ...fixture.settings, mining_priority_mode: 'manual' },
};
it('orders active confirmed progress before other available campaigns', () => {
  const progress = snapshot.campaigns[0]!;
  const untouched = {
    ...progress,
    id: 'untouched',
    drops: progress.drops.map((drop) => ({ ...drop, confirmed_minutes: 0 })),
  };
  const upcoming = { ...untouched, id: 'upcoming', active: false, upcoming: true };
  const claimed = {
    ...untouched,
    id: 'claimed',
    drops: [{ ...untouched.drops[0]!, is_claimed: true }],
  };
  expect([upcoming, untouched, progress, claimed].sort(campaignOrder).map((c) => c.id)).toEqual([
    progress.id,
    'claimed',
    'untouched',
    'upcoming',
  ]);
});

it('sorts campaigns by start, end, total drops or name with deterministic default ties', () => {
  const original = snapshot.campaigns[0]!;
  const campaigns = [
    {
      ...original,
      id: 'a',
      name: 'Zeta',
      starts_at: '2026-09-01T00:00:00Z',
      ends_at: '2026-09-09T00:00:00Z',
      total_drops: 2,
    },
    {
      ...original,
      id: 'b',
      name: 'Alpha',
      starts_at: '2026-09-02T02:00:00+02:00',
      ends_at: '2026-09-08T00:00:00Z',
      total_drops: 5,
    },
    {
      ...original,
      id: 'c',
      name: 'beta',
      starts_at: '2026-09-02T01:00:00Z',
      ends_at: '2026-09-10T00:00:00Z',
      total_drops: 3,
    },
  ];
  const order = (sort: Parameters<typeof campaignOrder>[2]) =>
    [...campaigns].sort((a, b) => campaignOrder(a, b, sort)).map((campaign) => campaign.id);
  expect(order('default')).toEqual(['b', 'a', 'c']);
  expect(order('newest')).toEqual(['c', 'b', 'a']);
  expect(order('ending')).toEqual(['b', 'a', 'c']);
  expect(order('drops')).toEqual(['b', 'c', 'a']);
  expect(order('name')).toEqual(['b', 'c', 'a']);
  const tied = campaigns.map((campaign) => ({ ...campaign, total_drops: 5 }));
  expect(
    tied
      .reverse()
      .sort((a, b) => campaignOrder(a, b, 'drops'))
      .map((campaign) => campaign.id),
  ).toEqual(['b', 'a', 'c']);
  expect(campaigns.map((campaign) => campaign.id)).toEqual(['a', 'b', 'c']);
});
describe('boundary behavior', () => {
  it('rejects executable and credential-bearing URLs', () => {
    for (const url of [
      'javascript:alert(1)',
      'data:text/html,test',
      'https://user:secret@example.com',
      '/relative',
    ])
      expect(safeUrl(url)).toBeUndefined();
    expect(safeUrl('https://twitch.tv/test')).toBe('https://twitch.tv/test');
  });
  it('moves priorities without losing or duplicating games, rejecting invalid ranks', () => {
    expect(moveGame(['A', 'B', 'C'], 0, 99)).toEqual(['B', 'C', 'A']);
    expect(moveGame(['A', 'B'], 1, -99)).toEqual(['B', 'A']);
    expect(moveGame(['A', 'B'], 0, 1.5)).toEqual(['A', 'B']);
  });
  it('localizes with English fallback and preserves literal user text', () => {
    expect(translator({})('watching', { channel: '<b>name</b>' })).toBe('Watching <b>name</b>');
    expect(plainText('🎮 Active ✔')).toBe('Active');
  });
  it('combines campaign statuses with OR and link filtering with AND', () => {
    const campaign = snapshot.campaigns[0]!;
    const filters = snapshot.settings.inventory_filters;
    expect(matchesCampaign(campaign, { ...filters, show_upcoming: true }, '')).toBe(true);
    expect(matchesCampaign(campaign, { ...filters, show_only_not_linked: true }, '')).toBe(false);
    expect(
      matchesCampaign(
        { ...campaign, linked: null },
        { ...filters, show_only_not_linked: true },
        '',
      ),
    ).toBe(false);
    expect(matchesCampaign({ ...campaign, finished: true }, filters, '')).toBe(false);
    expect(matchesCampaign({ ...campaign, mining_finished: true }, filters, '')).toBe(true);
    expect(matchesCampaign({ ...campaign, drops: [] }, filters, '')).toBe(true);
    expect(matchesCampaign(campaign, { ...filters, game_name_search: ['RUST'] }, '')).toBe(true);
    expect(
      matchesCampaign(
        campaign,
        {
          ...filters,
          show_benefit_badge: false,
          show_benefit_emote: false,
          show_benefit_item: false,
          show_benefit_other: false,
        },
        '',
      ),
    ).toBe(false);
  });
  it('excludes expired campaigns even with legacy saved or shared filters', () => {
    const campaign = snapshot.campaigns[0]!;
    for (const show_expired of [true, false]) {
      const filters = {
        ...snapshot.settings.inventory_filters,
        show_active: false,
        show_upcoming: false,
        show_expired,
      };
      expect(matchesCampaign({ ...campaign, expired: true }, filters, '')).toBe(false);
      expect(matchesCampaign(campaign, filters, '')).toBe(true);
      expect(matchesCampaign({ ...campaign, active: false, upcoming: true }, filters, '')).toBe(
        true,
      );
    }
  });
});

it('keeps claim history independent of completion, preserving metadata-free entries and deterministic sorting', () => {
  const campaign = snapshot.campaigns[0]!;
  const entries: HistoryEntry[] = [
    {
      id: 'old',
      campaign_id: '',
      campaign: 'Legacy',
      game: 'Old game',
      drop_name: 'Badge',
      benefits: ['Founder'],
      required_minutes: 15,
      claimed_at: '2026-09-27T10:00:00Z',
      claimed_at_is_observed: true,
    },
    {
      id: 'recent',
      campaign_id: campaign.id,
      campaign: campaign.name,
      game: campaign.game_name,
      drop_name: 'Reward',
      benefits: ['Coat'],
      required_minutes: 30,
      claimed_at: '2026-09-28T10:00:00Z',
    },
    {
      id: 'another',
      campaign_id: campaign.id,
      campaign: campaign.name,
      game: campaign.game_name,
      drop_name: 'Second',
      benefits: ['Boots'],
      required_minutes: 60,
      claimed_at: '2026-09-26T10:00:00Z',
    },
    {
      id: 'different',
      campaign_id: '',
      campaign: 'Other legacy',
      game: 'Old game',
      drop_name: 'Emote',
      benefits: ['Wave'],
      required_minutes: 15,
      claimed_at: '2026-09-25T10:00:00Z',
    },
  ];
  const groups = groupHistory(entries, [campaign]);
  expect(groups).toHaveLength(3);
  const legacy = groups[0]!;
  expect(legacy.metadata).toBeUndefined();
  expect(matchesHistory(legacy, ['OLD GAME'], 'founder')).toBe(true);
  expect(matchesHistory(legacy, ['Rust'], '')).toBe(false);
  for (const sort of ['default', 'newest', 'ending', 'drops', 'name'] as const) {
    expect([...groups].reverse().sort((a, b) => historyOrder(a, b, sort))[0]!.id).toBe(campaign.id);
  }
  expect(groupHistory([], [campaign])).toEqual([]); // Catalog/archive state cannot resurrect cleared claims.
  expect(groups[1]!.entries.map((entry) => entry.id)).toEqual(['recent', 'another']);
  expect(entries.map((entry) => entry.id)).toEqual(['old', 'recent', 'another', 'different']);
});

it('sorts history chronologically when timestamps have different fractional precision', () => {
  const entry: HistoryEntry = {
    id: 'whole',
    campaign_id: 'same',
    campaign: 'Season',
    game: 'Rust',
    drop_name: 'Reward',
    benefits: [],
    required_minutes: 30,
    claimed_at: '2026-09-28T10:00:00Z',
  };
  const fractional = { ...entry, id: 'fractional', claimed_at: '2026-09-28T10:00:00.100Z' };
  expect(groupHistory([entry, fractional], [])[0]!.entries.map((item) => item.id)).toEqual([
    'fractional',
    'whole',
  ]);
  const groups = groupHistory([entry, { ...fractional, campaign_id: 'different' }], []);
  for (const sort of ['default', 'newest', 'ending', 'drops', 'name'] as const) {
    expect([...groups].sort((a, b) => historyOrder(a, b, sort)).map((item) => item.id)).toEqual([
      'different',
      'same',
    ]);
  }
});
