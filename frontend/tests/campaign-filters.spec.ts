import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
test.beforeEach(async ({ request }) => {
  await request.post('/__test/reset', { headers, data: {} });
});

test.describe('phone game filters', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  for (const [search, fallback] of [
    [false, false],
    [true, false],
    [false, true],
    [true, true],
  ] as const) {
    test(`All games works on the first tap (search ${search}, fallback ${fallback})`, async ({
      page,
      request,
    }) => {
      if (fallback)
        await page.addInitScript(() => {
          delete (HTMLElement.prototype as Partial<HTMLElement>).showPopover;
          delete (HTMLElement.prototype as Partial<HTMLElement>).hidePopover;
        });
      const campaigns = Array.from({ length: 40 }, (_, index) => ({
        ...fixture.campaigns[0]!,
        id: `campaign-${index}`,
        name: `Campaign ${index}`,
        game_name: `Game ${String(index).padStart(2, '0')}`,
      }));
      await request.post('/__test/event', {
        headers,
        data: { event: 'inventory_batch_update', data: { campaigns } },
      });
      await page.goto('/campaigns');
      await page.getByRole('button', { name: 'Games', exact: true }).tap();
      const popup = page.getByRole('group', { name: 'Games', exact: true });
      if (search) {
        await popup.getByRole('searchbox').tap();
        await popup.getByRole('searchbox').fill('Game 0');
      }
      await popup.getByRole('checkbox', { name: 'Game 00', exact: true }).tap();
      await popup.getByRole('checkbox', { name: 'Game 01', exact: true }).tap();
      await expect(popup.getByRole('checkbox', { checked: true })).toHaveCount(2);
      await expect
        .poll(
          async () =>
            (await (await request.get('/api/settings')).json()).inventory_filters.game_name_search,
        )
        .toEqual(['Game 00', 'Game 01']);
      await popup.getByRole('button', { name: 'All games', exact: true }).tap();
      await expect(popup).toBeVisible();
      await expect(popup.getByRole('checkbox', { checked: true })).toHaveCount(0);
      await expect(popup.getByRole('button', { name: 'All games', exact: true })).toBeDisabled();
      await expect
        .poll(
          async () =>
            (await (await request.get('/api/settings')).json()).inventory_filters.game_name_search,
        )
        .toEqual([]);
      await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).tap();
      await expect(popup).toBeHidden();
      await page.getByRole('button', { name: 'Filters', exact: true }).tap();
      const filters = page.getByRole('group', { name: 'Filters', exact: true });
      await filters.getByRole('checkbox', { name: 'Hide selected games' }).tap();
      await filters.getByRole('button', { name: 'Clear filters', exact: true }).tap();
      await expect(filters).toBeVisible();
      await expect(
        filters.getByRole('checkbox', { name: 'Hide selected games' }),
      ).not.toBeChecked();
      await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).focus();
      await expect(filters).toBeHidden();
    });
  }
});

for (const [width, height, fallback] of [
  [1440, 900, false],
  [1280, 360, false],
  [390, 844, false],
  [320, 640, true],
] as const) {
  test(`campaign popovers overlay stationary results at ${width}px${fallback ? ' without native popovers' : ''}`, async ({
    page,
    request,
  }) => {
    if (fallback)
      await page.addInitScript(() => {
        Object.defineProperty(HTMLElement.prototype, 'showPopover', {
          value: undefined,
          configurable: true,
        });
        Object.defineProperty(HTMLElement.prototype, 'hidePopover', {
          value: undefined,
          configurable: true,
        });
      });
    const campaigns = Array.from({ length: 60 }, (_, index) => ({
      ...fixture.campaigns[0]!,
      id: `game-${index}`,
      name: `Campaign ${index}`,
      game_name: `Game ${String(index).padStart(2, '0')}`,
    }));
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_batch_update', data: { campaigns } },
    });
    await page.setViewportSize({ width, height });
    await page.goto('/campaigns');
    const results = page.getByRole('region', { name: 'Campaigns', exact: true });
    const before = await results.boundingBox();
    const scroll = await results.evaluate((element) => element.scrollTop);
    const games = page.getByRole('button', { name: 'Games', exact: true });
    await games.click();
    const popup = page.getByRole('group', { name: 'Games', exact: true });
    await expect(popup).toBeVisible();
    const bounds = (await popup.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(8);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width - 8);
    expect(bounds.y).toBeGreaterThanOrEqual(8);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(height - 8);
    expect(bounds.height).toBeLessThanOrEqual(360);
    expect(await results.boundingBox()).toEqual(before);
    await popup.getByRole('checkbox').last().scrollIntoViewIfNeeded();
    await expect(popup.getByRole('checkbox').last()).toBeInViewport();
    expect(await popup.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
      false,
    );
    expect(await results.evaluate((element) => element.scrollTop)).toBe(scroll);
    await popup.getByRole('searchbox').fill('Game 59');
    await expect(popup.getByRole('checkbox')).toHaveCount(1);
    await popup.getByRole('checkbox').check();
    await expect(page.getByRole('button', { name: 'Games (1)', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Open Campaign 59', exact: true })).toBeVisible();
    await popup.getByRole('button', { name: 'All games', exact: true }).click();
    await popup.getByRole('searchbox').fill('');
    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
    await expect(games).toBeFocused();
    await games.press('Enter');
    await games.press('Tab');
    await expect(popup.getByRole('searchbox')).toBeFocused();
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    await expect(popup).toBeHidden();
    const filters = page.getByRole('group', { name: 'Filters', exact: true });
    await expect(filters.getByRole('checkbox', { name: 'Hide selected games' })).toBeVisible();
    expect(await results.boundingBox()).toEqual(before);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/campaign-filters-${width}.png` });
    await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).click();
    await expect(filters).toBeHidden();
    await page.getByRole('link', { name: 'History', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Filters', exact: true })).toHaveCount(0);
    await expect(games).toBeVisible();
  });
}

test('hiding selected games follows Mine, retains details and history, and can be cleared', async ({
  page,
  request,
}) => {
  const campaigns = [
    { ...fixture.campaigns[0]!, id: 'a', name: 'First campaign', game_name: 'New game' },
    { ...fixture.campaigns[0]!, id: 'b', name: 'Second campaign', game_name: 'New game' },
    { ...fixture.campaigns[0]!, id: 'c', name: 'Third campaign', game_name: 'Next game' },
  ];
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns } },
  });
  await request.post('/api/settings', {
    headers,
    data: { mining_paused: true, inventory_filters: { hide_selected_games: true } },
  });
  await page.goto('/campaigns');
  await page.getByRole('button', { name: 'Mine New game', exact: true }).first().click();
  await expect(page.getByRole('button', { name: /^Open (First|Second) campaign$/ })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Mine Next game', exact: true })).toBeFocused();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toContain('New game');
  expect((await (await request.get('/api/settings')).json()).mining_paused).toBe(true);
  await page.goto('/campaigns?campaign=a');
  await expect(
    page
      .getByRole('complementary', { name: 'Campaign details' })
      .getByRole('heading', { name: 'First campaign', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Close details' }).click();
  await page.getByRole('button', { name: 'Mine Next game', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Campaigns', exact: true })).toBeFocused();
  await page.getByRole('button', { name: 'Show selected games' }).click();
  await expect(
    page.getByRole('button', { name: 'Open First campaign', exact: true }),
  ).toBeVisible();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).inventory_filters.hide_selected_games,
    )
    .toBe(false);
  await page.goto('/campaigns');
  await expect(
    page.getByRole('button', { name: 'Open First campaign', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Hide selected games' }).check();
  await page.getByRole('link', { name: 'History', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Open Autumn expedition', exact: true }),
  ).toBeVisible();
});

test('filter save failures remain visible and recover through the existing retry', async ({
  page,
  request,
}) => {
  await page.goto('/campaigns');
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() === 'POST')
      await route.fulfill({ status: 409, json: { detail: 'settings_conflict' } });
    else await route.continue();
  });
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Hide selected games' }).check();
  await expect(page.getByRole('alert')).toBeVisible();
  expect(
    (await (await request.get('/api/settings')).json()).inventory_filters.hide_selected_games,
  ).toBe(false);
  await page.keyboard.press('Escape');
  await page.unroute('**/api/settings');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).inventory_filters.hide_selected_games,
    )
    .toBe(true);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Show selected games' })).toBeVisible();
});
