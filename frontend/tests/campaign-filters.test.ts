import { describe, expect, it } from 'vitest';
import fixture from './fixture.json' with { type: 'json' };
import { matchesCampaign } from '../src/features/campaigns/Campaigns';
import type { Campaign, Eligibility, Filters } from '../src/shared/lib/types';

const campaign = fixture.campaigns[0] as Campaign;
const defaults = fixture.settings.inventory_filters as Filters;
const reward = campaign.drops[0]!;
const badgeOnly = { ...defaults, show_benefit_emote: false, show_benefit_item: false };

describe('campaign reward filters', () => {
  it('matches Twitch reward types rather than names, and includes unknown kinds only with all types', () => {
    for (let selection = 0; selection < 8; selection++) {
      const filters = {
        ...defaults,
        show_benefit_badge: !!(selection & 1),
        show_benefit_emote: !!(selection & 2),
        show_benefit_item: !!(selection & 4),
      };
      for (const [type, bit] of [
        ['BADGE', 1],
        ['EMOTE', 2],
        ['DIRECT_ENTITLEMENT', 4],
        ['UNKNOWN', 0],
        ['FUTURE_TYPE', 0],
      ] as const) {
        const value = {
          ...campaign,
          drops: [{ ...reward, benefits: [{ ...reward.benefits[0]!, name: 'Badge emote', type }] }],
        };
        expect(matchesCampaign(value, filters, '')).toBe(selection === 7 || !!(selection & bit));
      }
    }
  });

  it('does not match a campaign solely through a claimed or expired reward', () => {
    const badge = { ...reward, benefits: [{ ...reward.benefits[0]!, type: 'BADGE' }] };
    for (const ended of [
      { ...badge, is_claimed: true, eligibility: 'claimed' as const },
      { ...badge, eligibility: 'expired' as const },
      { ...badge, required_minutes: 0 },
    ]) {
      const mixed = { ...campaign, drops: [ended, reward] };
      expect(matchesCampaign(mixed, badgeOnly, '')).toBe(false);
      expect(matchesCampaign(mixed, defaults, '')).toBe(true);
    }
    for (const eligibility of [
      'ready',
      'upcoming',
      'prerequisite',
      'unselected',
      'filtered',
      'ignored',
      'awaiting_claim',
    ] satisfies Eligibility[]) {
      expect(
        matchesCampaign({ ...campaign, drops: [{ ...badge, eligibility }] }, badgeOnly, ''),
      ).toBe(true);
    }
  });
});
