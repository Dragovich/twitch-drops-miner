import { test, expect, type APIRequestContext } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };
import type { WantedGame } from '../src/shared/lib/types';

const headers = { 'X-TDM-Request': '1' };
test.use({ hasTouch: true });
function game(id: number, name: string): WantedGame {
  const original = structuredClone(fixture.wanted_items[0]!) as WantedGame;
  return {
    ...original,
    game_id: id,
    game_key: name.toLowerCase(),
    game_name: name,
    saved_rank: id,
    campaigns: [{ ...original.campaigns[0]!, id: `campaign-${id}` }],
  };
}
async function publish(request: APIRequestContext, games: WantedGame[]) {
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: { event: 'wanted_items_update', data: games },
      })
    ).ok(),
  ).toBeTruthy();
}
test.beforeEach(async ({ request }) => {
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
});

test('Up next opens each new first game without overriding manual choices or moving focus', async ({
  page,
  request,
}) => {
  const games = [game(1, 'Rust'), game(2, 'Tarkov'), game(3, 'Siege')];
  await publish(request, games);
  const settings = await (await request.get('/api/settings')).json();
  await page.goto('/');
  const queue = page.getByRole('region', { name: 'Up next', exact: true });
  const rust = queue.getByRole('button', { name: 'Rust', exact: true });
  const tarkov = queue.getByRole('button', { name: 'Tarkov', exact: true });
  const siege = queue.getByRole('button', { name: 'Siege', exact: true });
  await expect(rust).toHaveAttribute('aria-expanded', 'true');
  await expect(tarkov).toHaveAttribute('aria-expanded', 'false');
  await expect(tarkov).toHaveAccessibleDescription(/2 rewards.*Ends:/);
  await rust.focus();
  await rust.press('Space');
  await expect(rust).toHaveAttribute('aria-expanded', 'false');
  games[0]!.campaigns[0]!.drops[0]!.name = 'Updated reward';
  await publish(request, games);
  await expect(rust).toHaveAttribute('aria-expanded', 'false');
  await tarkov.focus();
  await tarkov.press('Enter');
  await expect(tarkov).toHaveAttribute('aria-expanded', 'true');
  await siege.focus();
  await publish(request, [games[1]!, games[0]!, games[2]!]);
  await expect(tarkov).toHaveAttribute('aria-expanded', 'true');
  await expect(siege).toBeFocused();
  await tarkov.click();
  await publish(request, [games[1]!, games[2]!, games[0]!]);
  await expect(tarkov).toHaveAttribute('aria-expanded', 'false');
  await publish(request, [games[0]!, games[2]!, games[1]!]);
  await expect(rust).toHaveAttribute('aria-expanded', 'true');
  await expect(tarkov).toHaveAttribute('aria-expanded', 'false');
  await publish(request, [games[2]!, games[1]!]);
  await expect(siege).toHaveAttribute('aria-expanded', 'true');
  await siege.click();
  await publish(request, []);
  await expect(queue.getByRole('button')).toHaveCount(0);
  await publish(request, [games[2]!]);
  await expect(siege).toHaveAttribute('aria-expanded', 'true');
  expect(await (await request.get('/api/settings')).json()).toEqual(settings);
});

test('Up next keeps expanded later games and return focus through campaign navigation', async ({
  page,
  request,
}) => {
  const games = [game(1, 'Rust'), game(2, 'Tarkov')];
  games[1]!.campaigns[0]!.drops = Array.from({ length: 24 }, (_, index) => ({
    ...games[1]!.campaigns[0]!.drops[0]!,
    id: `reward-${index + 1}`,
    name: `Reward ${index + 1}`,
  }));
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: {
        ...fixture,
        wanted_items: games,
        campaigns: [
          fixture.campaigns[0],
          {
            ...fixture.campaigns[0],
            id: 'campaign-2',
            drops: games[1]!.campaigns[0]!.drops.map((drop) => ({
              ...fixture.campaigns[0]!.drops[0],
              id: drop.id,
              name: drop.name,
            })),
          },
        ],
      },
    },
  });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 700 });
    await page.goto('/');
    const queue = page.locator('#up-next-list');
    const later = queue.getByRole('button', { name: 'Tarkov', exact: true });
    await later.click();
    const link = page.locator('#up-next-drop-campaign-2-reward-20');
    await link.scrollIntoViewIfNeeded();
    const top = await queue.evaluate((element) => element.scrollTop);
    expect(top).toBeGreaterThan(0);
    await link.click();
    await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(later).toHaveAttribute('aria-expanded', 'true');
    await expect(link).toBeFocused();
    expect(await queue.evaluate((element) => element.scrollTop)).toBeCloseTo(top, 0);
    await page.getByRole('link', { name: 'Campaigns', exact: true }).click();
    await publish(request, [games[1]!, games[0]!]);
    await page.getByRole('link', { name: 'Mining', exact: true }).click();
    await expect(later).toHaveAttribute('aria-expanded', 'true');
    await publish(request, games);
  }
});

test('Up next resets disclosure choices when the Twitch account changes', async ({
  page,
  request,
}) => {
  const games = [game(1, 'Rust'), game(2, 'Tarkov')];
  await publish(request, games);
  await page.goto('/');
  const queue = page.locator('#up-next-list');
  await queue.getByRole('button', { name: 'Rust', exact: true }).click();
  await queue.getByRole('button', { name: 'Tarkov', exact: true }).click();
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: { ...fixture, wanted_items: games, login: { ...fixture.login, user_id: 789 } },
    },
  });
  await expect(queue.getByRole('button', { name: 'Rust', exact: true })).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  await expect(queue.getByRole('button', { name: 'Tarkov', exact: true })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});

test('Up next aligns a single reward window with its campaign and wraps long names on phones', async ({
  page,
  request,
}) => {
  const games = [game(1, 'Rainbow Six Siege'), game(2, 'Escape from Tarkov')];
  games[0]!.campaigns[0]!.name = 'R6S S2 2026 17';
  games[0]!.campaigns[0]!.drops = [games[0]!.campaigns[0]!.drops[0]!];
  await publish(request, games);
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    const queue = page.locator('#up-next-list');
    const campaign = queue.getByRole('link', { name: 'R6S S2 2026 17', exact: true });
    const date = campaign.locator('..').getByText(/^Ends:/);
    const nameBox = (await campaign.boundingBox())!;
    const dateBox = (await date.boundingBox())!;
    if (width >= 390) {
      expect(Math.abs(nameBox.y - dateBox.y)).toBeLessThan(4);
      expect(dateBox.x).toBeGreaterThan(nameBox.x + nameBox.width);
    } else {
      expect(dateBox.y).toBeGreaterThanOrEqual(nameBox.y + nameBox.height);
    }
    await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(0);
    const toggle = queue.getByRole('button', { name: 'Rainbow Six Siege', exact: true });
    expect((await toggle.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await toggle.tap();
    await expect(campaign).toBeHidden();
    await expect(toggle).toHaveAccessibleDescription(/1 reward.*Ends:/);
    await toggle.tap();
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.screenshot({ path: `../artifacts/up-next-${width}.png`, fullPage: true });
  }
  games[0]!.game_name = 'VeryLongGameName'.repeat(10);
  games[0]!.campaigns[0]!.name = 'VeryLongCampaignName'.repeat(10);
  await publish(request, games);
  await expect(page.locator('#up-next-campaign-campaign-1')).toHaveText(
    games[0]!.campaigns[0]!.name,
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(320);
  expect(
    await page.locator('#up-next-list').evaluate((el) => el.scrollWidth <= el.clientWidth),
  ).toBe(true);
});

test('completed watch time remains unclaimed in campaign details with one connection action', async ({
  page,
  request,
}) => {
  const campaign = structuredClone(fixture.campaigns[0]!);
  campaign.linked = false;
  campaign.drops[0]!.eligibility = 'awaiting_claim';
  campaign.drops[0]!.current_minutes = campaign.drops[0]!.required_minutes;
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, campaigns: [campaign] } },
  });
  await page.goto('/campaigns?campaign=campaign-1');
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  await expect(detail.getByText('Watch time complete', { exact: true })).toBeVisible();
  await expect(detail.getByRole('link', { name: 'Connect game account', exact: true })).toHaveCount(
    1,
  );
  await expect(detail.getByText('Game account not linked', { exact: true })).toBeVisible();
  await expect(detail.getByText(/waiting for claim confirmation/)).toHaveCount(0);
});
