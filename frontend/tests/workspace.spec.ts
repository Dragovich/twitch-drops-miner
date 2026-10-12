import { test, expect, type WebSocketRoute } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
test.beforeEach(async ({ request }) => {
  expect((await request.get('/__test/health')).ok()).toBeTruthy();
  const reset = await request.post('/__test/reset', { headers, data: {} });
  expect(await reset.json()).toEqual({ ok: true });
});

for (const [locale, date] of [
  ['en-US', 'Oct 4, 2026'],
  ['de-DE', '04.10.2026'],
]) {
  test.describe(`24-hour time in ${locale}`, () => {
    test.use({ locale, timezoneId: 'Europe/Berlin' });
    test('campaign cards use short local dates while details retain the year', async ({
      page,
      request,
    }) => {
      const campaign = {
        ...fixture.campaigns[0]!,
        starts_at: '2026-10-03T22:05:00Z',
        ends_at: '2026-10-04T21:58:00Z',
      };
      await request.post('/__test/event', {
        headers,
        data: {
          event: 'inventory_batch_update',
          data: { campaigns: [campaign] },
        },
      });
      await page.goto('/campaigns');
      const row = page.locator('.campaign-summary');
      await expect(row.locator('.campaign-info > :last-child')).toHaveText(
        locale === 'en-US' ? 'Ends Oct 4, 23:58' : 'Ends 4. Okt., 23:58',
      );
      await row.getByRole('button', { name: 'Campaign details', exact: true }).click();
      const detail = page.getByRole('complementary', { name: 'Campaign details' });
      await expect(
        detail.getByText(`${date}, 00:05 — ${date}, 23:58`, { exact: true }),
      ).toBeVisible();
      await page.getByRole('button', { name: 'Close details' }).click();
      await expect(
        row.getByRole('button', { name: 'Campaign details', exact: true }),
      ).toBeFocused();
      await request.post('/__test/event', {
        headers,
        data: {
          event: 'inventory_batch_update',
          data: { campaigns: [{ ...campaign, active: false, upcoming: true }] },
        },
      });
      await page.getByRole('button', { name: 'Filters', exact: true }).click();
      await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
      await expect(row.locator('.campaign-info > :last-child')).toHaveText(
        locale === 'en-US' ? 'Starts Oct 4, 00:05' : 'Starts 4. Okt., 00:05',
      );
    });
    test('preserves local dates and timezone at midnight, noon and night', async ({
      page,
      request,
    }) => {
      const times = [
        { utc: '2026-10-03T22:05:06Z', clock: '00:05:06' },
        { utc: '2026-10-04T10:30:00Z', clock: '12:30:00' },
        { utc: '2026-10-04T21:59:59Z', clock: '23:59:59' },
      ];
      const activity = times.map(({ utc }, id) => ({
        id: id + 1,
        first_at: times[0]!.utc,
        last_at: utc,
        category: 'mining',
        severity: 'info',
        code: 'status.message',
        args: {},
        message: `Event ${id + 1}`,
        campaign_id: null,
        drop_id: null,
        channel_id: null,
        count: 2,
        recovered: false,
      }));
      expect(
        (
          await request.post('/__test/event', {
            headers,
            data: { event: 'initial_state', data: { ...fixture, activity } },
          })
        ).ok(),
      ).toBeTruthy();
      await page.goto('/activity');
      for (const [index, { clock, utc }] of times.entries()) {
        const row = page.getByRole('article').nth(index);
        await expect(row.locator('time')).toHaveText(clock);
        await expect(row.locator('time')).toHaveAttribute('datetime', utc);
        await expect(row.locator('time')).toHaveAttribute('title', `${date}, ${clock.slice(0, 5)}`);
        await expect(row.getByText('×2')).toHaveAttribute(
          'title',
          `2 occurrences, ${date}, 00:05 to ${date}, ${clock.slice(0, 5)}`,
        );
      }
    });
  });
}

for (const width of [1440, 390, 320]) {
  test(`workspace and details at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 960 });
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Refresh inventory', exact: true }),
    ).toBeEnabled();
    if (width >= 1024)
      await expect(page.getByRole('link', { name: 'GitHub repository' }).locator('..')).toHaveCSS(
        'border-top-width',
        '0px',
      );
    await page.screenshot({ path: `../artifacts/workspace-mining-${width}.png`, fullPage: true });
    await page.getByRole('link', { name: 'Edit', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Mining preferences' })).toBeVisible();
    await expect(page.getByRole('combobox', { name: 'Mining priority' })).toBeEnabled();
    await expect(page.getByRole('heading', { name: 'Now mining' })).toHaveCount(0);
    const search = page.getByRole('combobox', { name: 'Search games...' });
    const games = page.getByRole('region', { name: 'Game priorities', exact: true });
    const gamesTop = (await games.boundingBox())!.y;
    await search.fill('Elder');
    const results = page.getByRole('listbox', { name: 'Search games...' });
    await expect(results.getByRole('option', { name: 'The Elder Scrolls Online' })).toBeVisible();
    const bounds = (await results.boundingBox())!;
    expect(bounds.y).toBeGreaterThan((await search.boundingBox())!.y);
    expect((await games.boundingBox())!.y).toBe(gamesTop);
    await search.fill('');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({
      path: `../artifacts/workspace-preferences-${width}.png`,
      fullPage: true,
    });
    await page.getByRole('link', { name: 'Back to Mining' }).click();
    await expect(page.getByRole('heading', { name: 'Now mining' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Edit', exact: true })).toBeFocused();
    await page.goto('/campaigns?q=Autumn&view=list');
    await page.getByRole('button', { name: 'Open Autumn expedition', exact: true }).click();
    await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Explorer jacket' })).toBeVisible();
    await page.screenshot({ path: `../artifacts/workspace-details-${width}.png`, fullPage: true });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(page.getByRole('searchbox', { name: 'Search campaigns' })).toHaveValue('Autumn');
    await expect(
      page.getByRole('button', { name: 'Open Autumn expedition', exact: true }),
    ).toBeFocused();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
  });
}

test('only the game list scrolls while desktop preferences controls stay in place', async ({
  page,
  request,
}) => {
  const games = Array.from({ length: 30 }, (_, index) => `Game ${index + 1}`);
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 1280, height: 480 },
    { width: 1024, height: 360 },
  ]) {
    expect(
      (
        await request.post('/api/settings', {
          headers,
          data: { games_to_watch: games, mining_priority_mode: 'short_events' },
        })
      ).ok(),
    ).toBeTruthy();
    await page.setViewportSize(viewport);
    await page.goto('/?edit=priorities');
    const panel = page.getByRole('region', { name: 'Mining preferences', exact: true });
    const list = page.getByRole('region', { name: 'Game priorities', exact: true });
    const search = page.getByRole('combobox', { name: 'Search games...' });
    const rewards = page
      .getByRole('group', { name: 'Also mine from other games', exact: true })
      .getByRole('checkbox', { name: 'Badges' });
    const back = page.getByRole('link', { name: 'Back to Mining' });
    await expect(back).toBeVisible();
    await expect(page.getByRole('button', { name: 'Reorder Game 1', exact: true })).toBeEnabled();
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(
      viewport.height,
    );
    expect(await list.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(
      true,
    );
    const searchTop = (await search.boundingBox())!.y;
    const rewardsTop = (await rewards.boundingBox())!.y;
    await list.focus();
    await page.keyboard.press('PageDown');
    await expect(list).toHaveCSS('background-color', 'rgb(36, 36, 36)');
    await expect(list.getByRole('list', { name: 'Game priorities', exact: true })).toHaveCSS(
      'background-color',
      'rgb(36, 36, 36)',
    );
    await list.hover();
    await page.mouse.wheel(0, 350);
    await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await list.evaluate((element) => {
      element.scrollTop = 0;
    });
    const handle = await page
      .getByRole('button', { name: 'Reorder Game 1', exact: true })
      .boundingBox();
    const bounds = (await list.boundingBox())!;
    await page.mouse.move(handle!.x + 10, handle!.y + 10);
    await page.mouse.down();
    for (let step = 0; step < 10; step++) {
      await page.mouse.move(handle!.x + 12 + step, bounds.y + bounds.height - 15);
    }
    await expect.poll(() => list.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await page.keyboard.press('Escape');
    await page.mouse.up();
    await list.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await expect(
      page.getByRole('button', { name: 'Reorder Game 30', exact: true }),
    ).toBeInViewport();
    expect((await search.boundingBox())!.y).toBe(searchTop);
    expect((await rewards.boundingBox())!.y).toBe(rewardsTop);
    expect(await panel.evaluate((element) => element.scrollTop)).toBe(0);
    await page.getByLabel('Ignore rewards by name', { exact: true }).scrollIntoViewIfNeeded();
    await expect(page.getByLabel('Ignore rewards by name', { exact: true })).toBeInViewport();
    expect((await search.boundingBox())!.y).toBe(searchTop);
    await expect(back).toBeInViewport();
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await list.evaluate((element) => {
      element.scrollTop = 0;
    });
    const first = page.getByRole('button', { name: 'Reorder Game 1', exact: true });
    await first.focus();
    for (let step = 0; step < 15; step++) await first.press('ArrowDown');
    await expect(first).toBeFocused();
    await expect(first).toBeInViewport({ ratio: 0.99 });
    expect((await search.boundingBox())!.y).toBe(searchTop);
    expect(await panel.evaluate((element) => element.scrollTop)).toBe(0);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await page.screenshot({ path: `../artifacts/preferences-list-scroll-${viewport.width}.png` });
    const listBeforeSearch = await list.boundingBox();
    const scrollBeforeSearch = await list.evaluate((element) => element.scrollTop);
    await search.fill('e');
    const results = page.getByRole('listbox', { name: 'Search games...' });
    const match = results.getByRole('option', { name: 'Sea of Thieves', exact: true });
    await expect(match).toBeInViewport({ ratio: 0.99 });
    const resultBounds = (await results.boundingBox())!;
    expect(resultBounds.y).toBeGreaterThan(searchTop);
    expect(resultBounds.y + resultBounds.height).toBeLessThanOrEqual(viewport.height - 8);
    expect(await list.boundingBox()).toEqual(listBeforeSearch);
    expect(await list.evaluate((element) => element.scrollTop)).toBe(scrollBeforeSearch);
    const firstMatchHandle = list.getByRole('button', { name: /^Reorder/ }).first();
    await firstMatchHandle.focus();
    await expect(results).toBeHidden();
    await expect(firstMatchHandle).toBeInViewport({ ratio: 0.99 });
    await page.screenshot({ path: `../artifacts/preferences-search-${viewport.width}.png` });
    await page.getByRole('button', { name: 'Add Game', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('Choose a game from the results.');
    await match.scrollIntoViewIfNeeded();
    await expect(match).toBeInViewport({ ratio: 0.99 });
    await match.click();
    await expect(search).toHaveValue('');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
      .toContain('Sea of Thieves');
    expect((await search.boundingBox())!.y).toBe(searchTop);
    expect(await panel.evaluate((element) => element.scrollTop)).toBe(0);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await back.click();
    await expect(page.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
  }
});

test('short preferences with save conflicts and reconnects keep controls reachable', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', {
    headers,
    data: {
      games_to_watch: Array.from({ length: 30 }, (_, index) => `Game ${index + 1}`),
      mining_priority_mode: 'short_events',
    },
  });
  await page.setViewportSize({ width: 1024, height: 360 });
  await page.goto('/?edit=priorities');
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() === 'POST')
      await route.fulfill({ status: 409, json: { detail: 'settings_conflict' } });
    else await route.continue();
  });
  await page
    .getByRole('group', { name: 'Also mine from other games', exact: true })
    .getByRole('checkbox', { name: 'Badges' })
    .check();
  await expect(page.getByRole('alert')).toBeVisible();
  await page.getByRole('combobox', { name: 'Search games...' }).fill('e');
  const results = page.getByRole('listbox', { name: 'Search games...' });
  const resultBounds = (await results.boundingBox())!;
  expect(resultBounds.y).toBeGreaterThanOrEqual(8);
  expect(resultBounds.y + resultBounds.height).toBeLessThanOrEqual(352);
  const match = results.getByRole('option', { name: 'Sea of Thieves', exact: true });
  await match.scrollIntoViewIfNeeded();
  await expect(match).toBeInViewport({ ratio: 0.99 });
  const last = page.getByRole('button', { name: 'Reorder Game 30', exact: true });
  await last.focus();
  await expect(last).toBeInViewport({ ratio: 0.99 });
  await expect(page.getByRole('link', { name: 'Back to Mining' })).toBeInViewport();
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(360);
  await page.screenshot({ path: '../artifacts/preferences-conflict-short.png' });
  await page.context().setOffline(true);
  try {
    await request.post('/__test/reconnect', { headers, data: {} });
    await expect(page.getByRole('combobox', { name: 'Mining priority' })).toBeDisabled();
    const column = page.getByRole('group', { name: 'Game priorities', exact: true });
    await column.focus();
    await column.press('End');
    await expect(page.getByRole('listbox')).toBeHidden();
    await last.scrollIntoViewIfNeeded();
    await expect(last).toBeInViewport({ ratio: 0.99 });
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(360);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: '../artifacts/preferences-disconnected-short.png' });
  } finally {
    await page.unroute('**/api/settings');
    await page.context().setOffline(false);
  }
  await expect(page.getByRole('combobox', { name: 'Mining priority' })).toBeEnabled();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).auto_mine_badges)
    .toBe(true);
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('leaving preferences preserves a conflicting draft for retry', async ({ page }) => {
  await page.goto('/?edit=priorities');
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() === 'POST') {
      await route.fulfill({ status: 409, json: { detail: 'settings_conflict' } });
    } else await route.continue();
  });
  await page
    .getByRole('group', { name: 'Allowed reward types', exact: true })
    .getByRole('checkbox', { name: 'Badges', exact: true })
    .uncheck();
  await expect(page.getByRole('alert')).toBeVisible();
  await page.getByRole('link', { name: 'Back to Mining' }).click();
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toBeEnabled();
  await page.getByRole('link', { name: 'Edit', exact: true }).click();
  await expect(
    page
      .getByRole('group', { name: 'Allowed reward types', exact: true })
      .getByRole('checkbox', { name: 'Badges', exact: true }),
  ).not.toBeChecked();
  await page.unroute('**/api/settings');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('simplified captions and settings retain progress, recovery and refresh controls', async ({
  page,
  request,
}) => {
  const mining = {
    state: 'watching',
    channel_id: null,
    campaign_id: 'campaign-1',
    drop_id: 'reward-1',
    priority: { reason: 'saved_order', deadline: null, target_ids: [] },
  };
  const activity = [
    {
      id: 1,
      first_at: '2026-10-03T12:00:00Z',
      last_at: '2026-10-03T12:00:10Z',
      category: 'mining',
      severity: 'warning',
      code: 'watch_failed',
      args: {},
      message: 'Watch connection interrupted',
      campaign_id: null,
      drop_id: null,
      channel_id: null,
      count: 2,
      recovered: true,
    },
  ];
  activity.push({ ...activity[0]!, id: 2, message: 'Watching: Example channel', count: 1 });
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, mining, activity } },
  });
  await page.goto('/');
  await expect(page.getByRole('progressbar', { name: 'Explorer jacket' })).toBeVisible();
  await expect(page.getByText('Watching for this reward', { exact: true })).toHaveCount(0);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: { ...fixture, mining: { ...mining, state: 'awaiting_claim' }, activity },
    },
  });
  await expect(
    page.getByText('Watch time complete; waiting for claim confirmation', { exact: true }),
  ).toBeVisible();
  await page.goto('/campaigns?tab=history');
  await expect(page.getByRole('button', { name: 'Refresh inventory', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Refresh inventory', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
  await page.goto('/activity');
  await expect(page.getByText('This session', { exact: true })).toHaveCount(0);
  const event = page.getByRole('article').first();
  await expect(event.getByText('Watch connection interrupted', { exact: true })).toBeVisible();
  await expect(event.getByText('Mining', { exact: true })).toHaveCount(0);
  await expect(event.getByRole('paragraph').filter({ hasText: /^Recovered$/ })).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const panel = (await page.locator('.activity-list').boundingBox())!;
    const row = (await event.boundingBox())!;
    const inset = width >= 1024 ? 16 : 12;
    expect(row.x - panel.x).toBeCloseTo(inset + 1);
    expect(panel.x + panel.width - row.x - row.width).toBeCloseTo(inset + 1);
    await expect(event).toHaveCSS('border-bottom-width', '1px');
    await expect(page.getByRole('article').last()).toHaveCSS('border-bottom-width', '0px');
    await page.screenshot({ path: `../artifacts/activity-inset-${width}.png`, fullPage: true });
  }
  await page.getByRole('combobox', { name: 'Filter category' }).selectOption('claims');
  await expect(page.getByRole('article')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Filter category' }).selectOption('mining');
  await expect(event).toBeVisible();
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    let contentOffset: number | undefined;
    for (const section of ['account', 'access', 'connection', 'maintenance']) {
      await page.goto(`/settings#${section}`);
      const content = page.locator(`#${section}`);
      await expect(content).toBeVisible();
      await expect(content).toHaveCSS('border-bottom-width', '0px');
      await expect(
        page.getByRole('button', { name: /^(Refresh inventory|Refreshing\.\.\.)$/ }),
      ).toHaveCount(0);
      const tabs = (await page
        .getByRole('navigation', { name: 'Settings sections' })
        .boundingBox())!;
      await expect(content.getByRole('heading')).toHaveText(
        section === 'account' ? ['Badges'] : [],
      );
      const first = (await content.locator(':scope > :first-child').boundingBox())!;
      const offset = first.y - tabs.y - tabs.height;
      contentOffset ??= offset;
      expect(offset).toBeCloseTo(contentOffset, 0);
      expect(first.x).toBeCloseTo(tabs.x, 0);
      await page.screenshot({
        path: `../artifacts/settings-${section}-${width}.png`,
        fullPage: true,
      });
    }
  }
});

test('campaign details show shared dates once and preserve distinct reward windows', async ({
  page,
  request,
}) => {
  const base = fixture.campaigns[0]!;
  const reward = base.drops[0]!;
  const drops = [
    { ...reward, id: 'shared-raw' },
    {
      ...reward,
      id: 'shared-effective',
      starts_at: '2026-09-01T00:00:00Z',
      ends_at: '2026-11-01T00:00:00Z',
      effective_starts_at: '2026-09-20T02:00:00+02:00',
      effective_ends_at: '2026-10-02T02:00:00+02:00',
    },
    { ...reward, id: 'later-start', effective_starts_at: '2026-09-21T00:00:00Z' },
    { ...reward, id: 'earlier-end', effective_ends_at: '2026-10-01T00:00:00Z' },
    { ...reward, id: 'distinct-raw', starts_at: '2026-09-22T00:00:00Z' },
  ];
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: [{ ...base, total_drops: drops.length, drops }] },
    },
  });
  await page.goto('/campaigns?campaign=campaign-1');
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  const windows = await page.evaluate(
    (ranges) =>
      ranges.map((range) =>
        range
          .map((date) =>
            new Date(date).toLocaleString(undefined, {
              dateStyle: 'medium',
              timeStyle: 'short',
              hourCycle: 'h23',
            }),
          )
          .join(' — '),
      ),
    [
      [base.starts_at, base.ends_at],
      ['2026-09-21T00:00:00Z', base.ends_at],
      [base.starts_at, '2026-10-01T00:00:00Z'],
      ['2026-09-22T00:00:00Z', base.ends_at],
    ],
  );
  await expect(detail.getByText(windows[0]!, { exact: true })).toHaveCount(1);
  for (const id of ['shared-raw', 'shared-effective'])
    await expect(page.locator(`#drop-${id}`).getByText(/ — /)).toHaveCount(0);
  for (const [index, id] of ['later-start', 'earlier-end', 'distinct-raw'].entries())
    await expect(
      page.locator(`#drop-${id}`).getByText(windows[index + 1]!, { exact: true }),
    ).toBeVisible();
  // The separate recorded claim is not appended to Available's live reward list.
  await expect(detail.getByText(/^Claimed /)).toHaveCount(0);
  await expect(detail.getByRole('heading', { name: 'History', exact: true })).toHaveCount(0);
});

test('History details contain only recorded claims even with live campaign metadata', async ({
  page,
  request,
}) => {
  const saved = await (await request.get('/api/history')).json();
  const artwork = 'https://static-cdn.jtvnw.net/claimed-reward.png';
  await page.route(artwork, (route) =>
    route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" />',
    }),
  );
  const base = fixture.campaigns[0]!;
  const campaign = {
    ...base,
    name: 'Live campaign renamed',
    linked: null,
    allowed_channels: [{ login: 'northwind', name: 'northwind' }],
    priority: { reason: 'saved_order', deadline: null, target_ids: [] },
    drops: base.drops.map((drop) => ({
      ...drop,
      benefits: drop.benefits.map((benefit) => ({ ...benefit, image_url: artwork })),
    })),
  };
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: [campaign, { ...campaign, id: 'no-claims' }] },
    },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: [
          ...saved.entries,
          {
            ...saved.entries[0],
            id: 'reward-1',
            drop_name: 'Recorded jacket',
            benefits: ['Recorded jacket', 'Recorded bonus'],
            claimed_at_is_observed: true,
          },
        ],
      },
    }),
  );
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/campaigns?tab=history');
    const open = page.getByRole('button', { name: 'Open Autumn expedition', exact: true });
    await open.click();
    await expect(detail.getByRole('heading', { name: '2 claimed' })).toBeVisible();
    await expect(detail.getByRole('heading', { name: 'Autumn expedition' })).toBeVisible();
    await expect(detail.locator('.reward-detail')).toHaveCount(2);
    await expect(detail.getByText('Canvas pack', { exact: true })).toHaveCount(1);
    await expect(detail.getByText('Recorded jacket', { exact: true })).toHaveCount(1);
    await expect(detail.getByText('Recorded bonus', { exact: true })).toBeVisible();
    await expect(detail.getByText(/^Claimed /)).toBeVisible();
    await expect(detail.getByText(/^First observed /)).toBeVisible();
    await expect(detail.locator(`img[src="${artwork}"]`)).toBeVisible();
    await expect(detail.getByRole('link')).toHaveCount(0);
    await expect(detail.getByRole('progressbar')).toHaveCount(0);
    await expect(detail).not.toContainText('Account linking');
    await expect(detail).not.toContainText('Eligible channels');
    await expect(detail).not.toContainText('Saved game order');
    await expect(detail).not.toContainText('Trail companion');
    await expect(detail).not.toContainText('Explorer jacket');
    await expect(detail).not.toContainText('Requires these claims');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/history-detail-${width}.png`, fullPage: true });
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(open).toBeFocused();
  }
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=reward-1');
  await expect(page.locator('#history-drop-reward-1')).toHaveClass(/selected/);
  await expect(page.locator('#drop-reward-1')).toHaveCount(0);
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=reward-2');
  await expect(detail.getByRole('status')).toHaveText('No recorded claim for this reward.');
  await expect(detail).not.toContainText('Trail companion');
  await page.goto('/campaigns?tab=history&campaign=no-claims');
  await expect(detail.getByText('No recorded claims for this campaign.')).toBeVisible();
  await expect(detail.locator('.reward-detail')).toHaveCount(0);
  await page.goto('/campaigns?campaign=campaign-1');
  await expect(detail.getByRole('heading', { name: 'Live campaign renamed' })).toBeVisible();
  await expect(detail.getByText('Account linking has not been confirmed.')).toBeVisible();
  await expect(detail.getByRole('heading', { name: 'Trail companion' })).toBeVisible();
  for (const drop of ['missing', 'reward-2']) {
    await page.route('**/api/history', (route) => route.fulfill({ status: 500, json: {} }), {
      times: 1,
    });
    const failed = page.waitForResponse('**/api/history');
    await page.goto(`/campaigns?campaign=campaign-1&drop=${drop}`);
    expect((await failed).status()).toBe(500);
    if (drop === 'missing') {
      await expect(detail.getByRole('alert')).toContainText('Could not load history. Try again.');
      await expect(detail.getByRole('status')).toHaveCount(0);
      await detail.getByRole('button', { name: 'Try again', exact: true }).click();
      await expect(detail.getByRole('status')).toHaveText(
        'This reward is no longer in the campaign.',
      );
    } else {
      await expect(page.locator('#drop-reward-2')).toHaveClass(/selected/);
      await expect(detail.getByRole('alert')).toHaveCount(0);
    }
  }
});

test('History details stay claims-only while loading, retry errors and clear recorded claims', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  let finishLoad!: () => void;
  const loading = new Promise<void>((resolve) => {
    finishLoad = resolve;
  });
  await page.route(
    '**/api/history',
    async (route) => {
      await loading;
      await route.fulfill({ status: 500, json: {} });
    },
    { times: 1 },
  );
  const started = page.waitForRequest('**/api/history');
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=reward-1');
  await started;
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  try {
    await expect(detail).toBeVisible();
    await expect(detail.locator('.reward-detail')).toHaveCount(0);
    await expect(detail).not.toContainText('Game account linked');
  } finally {
    finishLoad();
  }
  await expect(detail.getByRole('alert')).toContainText('Could not load history. Try again.');
  await expect(detail.getByText('No recorded claims for this campaign.')).toHaveCount(0);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: '../artifacts/history-detail-error-phone.png', fullPage: true });
  await detail.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(detail.getByText('Canvas pack', { exact: true })).toBeVisible();
  await expect(detail.getByRole('status')).toHaveText('No recorded claim for this reward.');
  await page.route('**/api/history', (route) => route.fulfill({ status: 500, json: {} }), {
    times: 1,
  });
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: [{ ...fixture.campaigns[0], claimed_drops: 1 }] },
    },
  });
  await expect(detail.getByRole('alert')).toContainText('Could not load history. Try again.');
  await expect(detail.getByText('Canvas pack', { exact: true })).toBeVisible();
  await expect(detail.getByRole('status')).toHaveCount(0);
  await detail.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(detail.getByRole('status')).toHaveText('No recorded claim for this reward.');
  expect((await request.post('/api/cache/clear', { headers, data: {} })).ok()).toBeTruthy();
  await expect(detail.getByText('No recorded claims for this campaign.')).toBeVisible();
  await expect(detail.locator('.reward-detail')).toHaveCount(0);
});

test('detail focus returns to the actual opener when pointer activation leaves focus elsewhere', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: {
        campaigns: [
          fixture.campaigns[0]!,
          { ...fixture.campaigns[0]!, id: 'other', name: 'Other campaign' },
        ],
      },
    },
  });
  await page.goto('/campaigns');
  for (const opener of ['#campaign-detail-other', '#campaign-open-other']) {
    await page.locator('#campaign-open-campaign-1').focus();
    await page.locator(opener).dispatchEvent('click');
    await expect(page.getByRole('heading', { name: 'Other campaign', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(page.locator(opener)).toBeFocused();
  }
});

test('Available excludes expired campaigns from totals, pagination and game choices', async ({
  page,
  request,
}) => {
  const base = fixture.campaigns[0]!;
  const expired = Array.from({ length: 30 }, (_, index) => ({
    ...base,
    id: `expired-${index}`,
    name: `Expired ${index}`,
    game_name: 'Expired game',
    active: false,
    expired: true,
  }));
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: [base, ...expired] },
    },
  });
  await page.goto('/campaigns?show_expired=1&show_active=0&show_upcoming=0');
  await expect(page.locator('.campaign-summary')).toHaveCount(1);
  await expect(page.locator('.campaign-refresh')).toContainText('1 campaigns');
  await expect(page.getByRole('navigation', { name: 'Campaign pages' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Expired', exact: true })).toHaveCount(0);
  await expect(page.getByRole('checkbox', { name: 'Expired game', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
  await expect(page.locator('.campaign-summary')).toHaveCount(1);
  await expect(page).not.toHaveURL(/show_expired/);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: expired },
    },
  });
  await expect(page.getByText('No available campaigns.', { exact: true })).toBeVisible();
  await page.goto('/campaigns?campaign=expired-0');
  await expect(page.getByRole('heading', { name: 'Expired 0', exact: true })).toBeVisible();
  await expect(page.locator('.campaign-summary')).toHaveCount(0);
});

test('campaign grid aligns wrapped cards and gives narrow cards a separate action row', async ({
  page,
  request,
}) => {
  const saved = await (await request.get('/api/history')).json();
  const campaigns = [
    ['Arena Streamer Showmatch', 'Escape from Tarkov: Arena'],
    ['September 05', 'Ravendawn'],
    ['Summer Drops Fall – Community Championships Weekend', 'Coryphaeus Championships'],
    ['AOCP Flamescale 4 - #6/7', 'Albion Online'],
  ].map(([name, game_name], index) => ({
    ...fixture.campaigns[0]!,
    id: `layout-${index}`,
    name,
    game_name,
    claimed_drops: 1,
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns } },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: campaigns.map((campaign) => ({
          ...saved.entries[0],
          id: `claim-${campaign.id}`,
          campaign_id: campaign.id,
          campaign: campaign.name,
          game: campaign.game_name,
        })),
      },
    }),
  );
  for (const width of [1100, 800, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const history of [false, true]) {
      await page.goto(`/campaigns?view=grid${history ? '&tab=history' : ''}`);
      const cards = page.locator('.campaign-summary');
      await expect(cards).toHaveCount(4);
      await page.screenshot({
        path: `../artifacts/campaign-wrapped-${history ? 'history' : 'available'}-${width}.png`,
        fullPage: true,
      });
      for (const index of [0, 2]) {
        const left = (await cards.nth(index).boundingBox())!;
        const right = (await cards.nth(index + 1).boundingBox())!;
        expect(right.x).toBeGreaterThan(left.x + left.width);
        expect(right.y).toBe(left.y);
        expect(right.height).toBe(left.height);
      }
      if (width < 1200) {
        for (const card of await cards.all()) {
          const info = (await card.locator('.campaign-info').boundingBox())!;
          const count = (await card.locator('.campaign-count').boundingBox())!;
          const detail = (await card.locator('.campaign-detail-icon').boundingBox())!;
          expect(info.width).toBeGreaterThan(250);
          expect(count.y).toBeGreaterThan(info.y + info.height);
          expect(detail.y).toBeGreaterThan(info.y + info.height);
          if (!history) {
            const mine = (await card.getByRole('button', { name: /^Mine / }).boundingBox())!;
            expect(mine.y + mine.height / 2).toBe(detail.y + detail.height / 2);
            expect(mine.x).toBeGreaterThan(detail.x + detail.width);
          }
        }
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    }
  }
});

test('campaign cards retain shared artwork and dividers with compact mobile actions', async ({
  page,
  request,
}) => {
  const saved = await (await request.get('/api/history')).json();
  const campaigns = Array.from({ length: 3 }, (_, index) => ({
    ...fixture.campaigns[0]!,
    id: `summary-${index}`,
    name:
      index === 2
        ? 'Summary 3 with a long campaign title that wraps on phones'
        : `Summary ${index + 1}`,
    game_name: index === 2 ? 'GameNameWithoutBreaks'.repeat(3) : fixture.campaigns[0]!.game_name,
    claimed_drops: 1,
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns } },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: campaigns.map((campaign) => ({
          ...saved.entries[0],
          id: `claim-${campaign.id}`,
          campaign_id: campaign.id,
          campaign: campaign.name,
          game: campaign.game_name,
        })),
      },
    }),
  );
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    for (const view of ['grid', 'list']) {
      const measurements = [];
      for (const history of [false, true]) {
        await page.goto(
          `/campaigns?view=${view}&sort=name&q=Summary${history ? '&tab=history' : ''}`,
        );
        const cards = page.locator('.campaign-summary');
        await expect(cards).toHaveCount(3);
        const open = page.getByRole('button', { name: 'Open Summary 1', exact: true });
        await expect(open).toBeVisible();
        measurements.push(
          await cards.first().evaluate((card) => {
            const element = card.querySelector('.campaign-open')!;
            const art = element.children[0]!;
            const info = element.children[1]!;
            const count = card.querySelector('.campaign-count')!;
            return {
              artWidth: art.getBoundingClientRect().width,
              artHeight: art.getBoundingClientRect().height,
              padding: getComputedStyle(element).padding,
              gap: getComputedStyle(element).gap,
              gameMargin: getComputedStyle(info.children[1]!).marginTop,
              dateSize: getComputedStyle(info.children[2]!).fontSize,
              countColor: getComputedStyle(count.lastElementChild!).color,
              height: element.getBoundingClientRect().height,
              infoWidth: info.getBoundingClientRect().width,
            };
          }),
        );
        if (width < 768) {
          const info = (await open.locator('.campaign-info').boundingBox())!;
          const count = (await cards.first().locator('.campaign-count').boundingBox())!;
          const detail = (await cards.first().locator('.campaign-detail-icon').boundingBox())!;
          if (history) {
            expect(info.width).toBeGreaterThan(180);
            expect(count.y).toBeGreaterThan(info.y + info.height);
            expect(detail.y).toBeGreaterThan(info.y + info.height);
          } else {
            const art = (await open.locator(':scope > :first-child').boundingBox())!;
            const action = (await cards.first().locator('.campaign-action').boundingBox())!;
            expect(info.width).toBeGreaterThan(100);
            expect(art.y + art.height / 2).toBeCloseTo(info.y + info.height / 2, 0);
            expect(detail.x).toBeGreaterThan(info.x + info.width);
            expect(action.x).toBeGreaterThanOrEqual(detail.x + detail.width);
            expect(action.y).toBe(detail.y);
            expect(detail.width).toBe(44);
            expect(detail.height).toBe(44);
            expect(action.height).toBe(44);
            expect(detail.y).toBeLessThan(info.y + info.height);
            expect(count.y + count.height).toBeLessThanOrEqual(detail.y);
            await expect(cards.first().locator('.campaign-count')).toHaveAttribute(
              'title',
              '1 of 2 rewards claimed',
            );
            await expect(cards.first()).not.toContainText('Active');
            for (const card of await cards.all()) {
              const text = (await card.locator('.campaign-info').boundingBox())!;
              const cover = (await card.locator('.campaign-open > :first-child').boundingBox())!;
              expect(cover.y + cover.height / 2).toBeCloseTo(text.y + text.height / 2, 0);
              expect(
                await card.evaluate((element) => element.scrollWidth <= element.clientWidth),
              ).toBe(true);
            }
            expect((await new AxeBuilder({ page }).include('main').analyze()).violations).toEqual(
              [],
            );
          }
        }
        if (view === 'list') {
          for (const card of await cards.all()) await expect(card).toHaveCSS('border-width', '0px');
          expect(
            await cards
              .first()
              .evaluate((element) => getComputedStyle(element, '::before').content),
          ).toBe('none');
          for (const card of [cards.nth(1), cards.nth(2)]) {
            expect(
              await card.evaluate((element) => {
                const style = getComputedStyle(element, '::before');
                return [
                  style.content,
                  style.height,
                  style.left,
                  style.right,
                  style.backgroundColor,
                ];
              }),
            ).toEqual(['""', '1px', '16px', '16px', 'rgb(54, 54, 54)']);
          }
        } else {
          for (const card of await cards.all()) {
            await expect(card).toHaveCSS('border-width', '1px');
            await expect(card).toHaveCSS('border-radius', '6px');
          }
        }
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        if (history) {
          await expect(cards.first()).toContainText('1 claimed');
          await expect(cards.first()).not.toContainText('Active');
          await expect(cards.first().getByRole('button', { name: /Mine|Stop mining/ })).toHaveCount(
            0,
          );
        }
        await page.screenshot({
          path: `../artifacts/campaign-summary-${history ? 'history' : 'available'}-${view}-${width}.png`,
          fullPage: true,
        });
      }
      const [
        { height: availableHeight, infoWidth: availableWidth, ...available },
        { height: historyHeight, infoWidth: historyWidth, ...history },
      ] = measurements as [(typeof measurements)[number], (typeof measurements)[number]];
      expect(available.artWidth).toBe(48);
      expect(available.artHeight).toBe(48);
      expect(history.artWidth).toBe(available.artWidth);
      expect(history.artHeight).toBe(available.artHeight);
      expect(history.countColor).toBe(available.countColor);
      if (width === 1440) {
        expect(history).toEqual(available);
        expect(historyHeight).toBe(availableHeight);
      } else {
        expect(availableHeight).toBeLessThan(130);
        expect(historyWidth).toBeGreaterThan(availableWidth);
      }
    }
  }
});

test('campaign panes fill the height below full-width controls and keep row hovers compact', async ({
  page,
  request,
}) => {
  const base = fixture.campaigns[0]!;
  const campaigns = Array.from({ length: 30 }, (_, index) => ({
    ...base,
    id: `campaign-${index}`,
    name: `Campaign ${String(index + 1).padStart(2, '0')}`,
    game_name: `Game ${index + 1}`,
    total_drops: index === 0 ? 1 : 12,
    drops: Array.from({ length: index === 0 ? 1 : 12 }, (_, reward) => ({
      ...base.drops[0]!,
      id: `reward-${index}-${reward}`,
      name: `Reward ${reward + 1}`,
    })),
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns } },
  });
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 1280, height: 480 },
    { width: 1280, height: 360 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/campaigns?view=list&sort=name');
    const list = page.getByRole('region', { name: 'Campaigns', exact: true });
    const first = page.getByRole('button', { name: 'Open Campaign 01', exact: true });
    const search = page.getByRole('searchbox', { name: 'Search campaigns and rewards' });
    await expect(first).toBeVisible();
    const toolbar = (await page.locator('.campaign-toolbar').boundingBox())!;
    const searchBounds = (await search.boundingBox())!;
    await first.hover();
    await expect(first).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await expect(first.locator('..').locator('.campaign-detail-icon')).toHaveCSS('width', '28px');
    await expect(first.locator('..').locator('.campaign-detail-icon')).toHaveCSS(
      'background-color',
      'rgb(51, 51, 51)',
    );
    await first.focus();
    await expect(first.locator('.campaign-title')).toHaveCSS('text-decoration-line', 'underline');
    await first.click();
    const detail = page.getByRole('complementary', { name: 'Campaign details' });
    await expect(detail).toBeVisible();
    const bounds = (await detail.boundingBox())!;
    expect(await page.locator('.campaign-toolbar').boundingBox()).toEqual(toolbar);
    expect(await search.boundingBox()).toEqual(searchBounds);
    expect(bounds.x + bounds.width).toBeCloseTo(toolbar.x + toolbar.width, 0);
    expect(bounds.y).toBe((await list.boundingBox())!.y);
    const columns = (await page.locator('.campaign-columns').boundingBox())!;
    const browser = (await page.locator('.campaign-browser').boundingBox())!;
    expect(browser.width).toBeCloseTo(bounds.width, 0);
    expect(browser.width * 2 + 12).toBeCloseTo(columns.width, 0);
    await expect(list).toHaveCSS('padding-inline-end', '1px');
    const card = (await first.locator('..').boundingBox())!;
    const visibleGap = bounds.x - card.x - card.width;
    expect(visibleGap).toBeGreaterThanOrEqual(12);
    expect(visibleGap).toBeLessThanOrEqual(20);
    expect(bounds.y).toBe(toolbar.y + toolbar.height + 20);
    expect(bounds.y + bounds.height).toBe(viewport.height - 12);
    const pagination = (await page
      .getByRole('navigation', { name: 'Campaign pages' })
      .boundingBox())!;
    expect(pagination.y).toBeGreaterThanOrEqual(toolbar.y);
    expect(pagination.x + pagination.width / 2).toBeCloseTo(toolbar.x + toolbar.width / 2, 0);
    expect(pagination.y + pagination.height).toBeLessThanOrEqual(toolbar.y + toolbar.height);
    const listBounds = (await list.boundingBox())!;
    expect(listBounds.y + listBounds.height).toBe(viewport.height - 12);
    await expect(first.locator('..')).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    const mine = page.getByRole('button', { name: 'Mine Game 1', exact: true });
    await mine.hover();
    await expect(mine).toHaveCSS('background-color', 'rgb(51, 51, 51)');
    await expect(mine.locator('..')).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    const filters = (await page
      .getByRole('group', { name: 'Filters', exact: true })
      .boundingBox())!;
    const filteredBounds = (await detail.boundingBox())!;
    expect(filters.x).toBe(toolbar.x);
    expect(filters.width).toBeCloseTo(toolbar.width, 0);
    expect(filteredBounds.y).toBe((await list.boundingBox())!.y);
    expect(filteredBounds.y).toBeGreaterThan(bounds.y);
    expect(filteredBounds.y + filteredBounds.height).toBe(viewport.height - 12);
    await expect(
      detail.getByRole('heading', { name: 'Campaign 01', exact: true }),
    ).toBeInViewport();
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    expect(await detail.boundingBox()).toEqual(bounds);
    await page.screenshot({
      path: `../artifacts/campaign-pane-${viewport.width}-${viewport.height}.png`,
    });
    const searchTop = (await search.boundingBox())!.y;
    const later = page.getByRole('button', { name: 'Open Campaign 20', exact: true });
    await later.click();
    const listTop = await list.evaluate((element) => element.scrollTop);
    expect(listTop).toBeGreaterThan(0);
    expect((await search.boundingBox())!.y).toBe(searchTop);
    expect((await detail.boundingBox())!.height).toBe(bounds.height);
    const body = detail.getByRole('region', { name: 'Campaign 20', exact: true });
    await body.focus();
    await body.press('End');
    await expect.poll(() => body.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await expect(
      detail.getByRole('heading', { name: 'Campaign 20', exact: true }),
    ).toBeInViewport();
    expect(await list.evaluate((element) => element.scrollTop)).toBe(listTop);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(viewport.height);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(later).toBeFocused();
    expect(await list.evaluate((element) => element.scrollTop)).toBe(listTop);
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Clear filters', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Next page', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Open Campaign 30', exact: true })).toBeVisible();
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(viewport.height);
  }
  await page.goto('/campaigns?tab=history&view=list');
  const history = page.getByRole('button', { name: 'Open Autumn expedition', exact: true });
  await history.hover();
  await expect(history).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(history.locator('..').locator('.campaign-detail-icon')).toHaveCSS('width', '28px');
  await history.click();
  const historyBounds = (await page
    .getByRole('complementary', { name: 'Campaign details' })
    .boundingBox())!;
  const historyList = (await page
    .getByRole('region', { name: 'History', exact: true })
    .boundingBox())!;
  expect(historyBounds.y).toBe(historyList.y);
  expect(historyBounds.y + historyBounds.height).toBe(348);
  expect(historyBounds.y).toBeGreaterThan(
    (await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).boundingBox())!.y,
  );
});

test('short campaign workspaces keep controls and rewards reachable with long titles and notices', async ({
  page,
  request,
}) => {
  const campaign = {
    ...fixture.campaigns[0]!,
    name: 'Campaign with a long title that wraps across several lines when viewing the campaign details',
  };
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns: [campaign] } },
  });
  await page.setViewportSize({ width: 1280, height: 360 });
  await page.goto(`/campaigns?campaign=${campaign.id}`);
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  const body = detail.getByRole('region', { name: campaign.name, exact: true });
  const reward = body.getByRole('heading', { level: 3 }).last();
  await reward.scrollIntoViewIfNeeded();
  await expect(reward).toBeInViewport({ ratio: 0.99 });
  const bodyBounds = (await body.boundingBox())!;
  const detailBounds = (await detail.boundingBox())!;
  expect(bodyBounds.y + bodyBounds.height).toBeLessThanOrEqual(
    detailBounds.y + detailBounds.height,
  );
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() === 'POST')
      await route.fulfill({ status: 409, json: { detail: 'settings_conflict' } });
    else await route.continue();
  });
  await page.getByRole('button', { name: 'Change campaign layout', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  await page.context().setOffline(true);
  try {
    await request.post('/__test/reconnect', { headers, data: {} });
    await expect(page.getByText('Connection lost.', { exact: false })).toBeVisible();
    for (const control of [
      page.getByRole('link', { name: 'History', exact: true }),
      page.getByRole('searchbox', { name: 'Search campaigns and rewards' }),
      page.getByRole('button', { name: 'Filters', exact: true }),
      detail.getByRole('button', { name: 'Close details' }),
    ]) {
      await control.focus();
      await expect(control).toBeInViewport({ ratio: 0.99 });
    }
    await reward.scrollIntoViewIfNeeded();
    await expect(reward).toBeInViewport({ ratio: 0.99 });
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(360);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  } finally {
    await page.unroute('**/api/settings');
    await page.context().setOffline(false);
  }
});

for (const view of ['grid', 'list']) {
  test(`campaign ${view} restores its position after details and offline filters remain accessible`, async ({
    page,
    request,
  }) => {
    const campaigns = Array.from({ length: 30 }, (_, index) => ({
      ...fixture.campaigns[0]!,
      id: `long-${index}`,
      name: `Campaign ${String(index + 1).padStart(2, '0')} with a long title that wraps across several lines when viewing the campaign details`,
      game_name: `Game ${index + 1}`,
    }));
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_batch_update', data: { campaigns } },
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`/campaigns?view=${view}&sort=name`);
    // Selection and return position must not depend on native scroll anchoring.
    await page.addStyleTag({ content: '.campaign-results { overflow-anchor: none; }' });
    const list = page.getByRole('region', { name: 'Campaigns', exact: true });
    const target = page.getByRole('button', { name: `Open ${campaigns[14]!.name}`, exact: true });
    await target.scrollIntoViewIfNeeded();
    const before = await list.evaluate((element) => element.scrollTop);
    await target.click();
    await expect(target).toBeInViewport({ ratio: 0.99 });
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(target).toBeFocused();
    expect(await list.evaluate((element) => element.scrollTop)).toBeCloseTo(before, 0);
    await target.click();
    const next = page.getByRole('button', { name: `Open ${campaigns[11]!.name}`, exact: true });
    await next.evaluate((element) => element.scrollIntoView({ block: 'start' }));
    await next.click();
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(next).toBeFocused();
    await expect(next).toBeInViewport({ ratio: 0.99 });
    await next.click();
    await page.getByRole('button', { name: 'Change campaign layout', exact: true }).click();
    await expect(next).toBeInViewport({ ratio: 0.99 });
    await page.getByRole('button', { name: 'Close details' }).click();
    await expect(next).toBeFocused();
    await expect(next).toBeInViewport({ ratio: 0.99 });
    await page.setViewportSize({ width: 1280, height: 360 });
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    await page.context().setOffline(true);
    try {
      await request.post('/__test/reconnect', { headers, data: {} });
      await expect(page.getByRole('checkbox').first()).toBeDisabled();
      const games = page.getByRole('group', { name: 'Game', exact: true });
      await games.focus();
      await games.press('End');
      await expect.poll(() => games.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(360);
    } finally {
      await page.context().setOffline(false);
    }
  });
}

test('mobile campaign details lock background scrolling and restore the list', async ({
  page,
  request,
}) => {
  const saved = await (await request.get('/api/history')).json();
  const campaigns = Array.from({ length: 25 }, (_, index) => ({
    ...fixture.campaigns[0]!,
    id: `scroll-${index}`,
    name: `Campaign ${String(index + 1).padStart(2, '0')}`,
    drops: Array.from({ length: 12 }, (_, reward) => ({
      ...fixture.campaigns[0]!.drops[0]!,
      id: `scroll-${index}-${reward}`,
      name: `Reward ${reward + 1}`,
    })),
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns } },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: campaigns.flatMap((campaign) =>
          campaign.drops.map((drop) => ({
            ...saved.entries[0],
            id: drop.id,
            drop_name: drop.name,
            campaign_id: campaign.id,
            campaign: campaign.name,
          })),
        ),
      },
    }),
  );
  for (const width of [390, 1100]) {
    await page.setViewportSize({ width, height: 800 });
    for (const history of [false, true]) {
      for (const view of ['grid', 'list']) {
        await page.goto(`/campaigns?view=${view}&sort=name${history ? '&tab=history' : ''}`);
        const target = page.getByRole('button', { name: 'Open Campaign 15', exact: true });
        await target.scrollIntoViewIfNeeded();
        const before = await page.evaluate(() => window.scrollY);
        expect(before).toBeGreaterThan(0);
        await target.click();
        const detail = page.getByRole('complementary', { name: 'Campaign details' });
        const body = detail.locator('.detail-body');
        await expect(detail).toBeVisible();
        await expect(page.locator('html')).toHaveCSS('overflow-y', 'hidden');
        expect(await page.evaluate(() => window.scrollY)).toBe(before);
        await detail.locator('header').hover();
        await page.mouse.wheel(0, 400);
        await body.focus();
        await body.press('PageDown');
        await expect.poll(() => body.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
        await body.evaluate((element) => {
          element.scrollTop = element.scrollHeight;
        });
        await body.hover();
        await page.mouse.wheel(0, 400);
        await expect(body).toHaveCSS('overscroll-behavior-y', 'contain');
        expect(await page.evaluate(() => window.scrollY)).toBe(before);
        if (width === 390 && view === 'grid')
          await page.screenshot({
            path: `../artifacts/mobile-detail-scroll-${history ? 'history' : 'available'}.png`,
          });
        if (history) await page.goBack();
        else await detail.getByRole('button', { name: 'Close details' }).click();
        await expect(detail).toHaveCount(0);
        await expect(page.locator('html')).toHaveCSS('overflow-y', 'visible');
        await expect(target).toBeFocused();
        expect(await page.evaluate(() => window.scrollY)).toBeCloseTo(before, 0);
        await target.hover();
        await page.mouse.wheel(0, -250);
        await expect.poll(() => page.evaluate(() => window.scrollY)).toBeLessThan(before);
      }
    }
  }
  await page.goto('/campaigns?campaign=scroll-14');
  await expect(page.locator('html')).toHaveCSS('overflow-y', 'hidden');
  await page.setViewportSize({ width: 1440, height: 800 });
  await expect(page.locator('html')).toHaveCSS('overflow-y', 'visible');
  await page.setViewportSize({ width: 390, height: 800 });
  await expect(page.locator('html')).toHaveCSS('overflow-y', 'hidden');
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Activity', exact: true })
    .click();
  await expect(page.locator('html')).toHaveCSS('overflow-y', 'visible');
});

test('switching details closes back to the list and live updates preserve reward scroll', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  const first = fixture.campaigns[0]!;
  const second = {
    ...first,
    id: 'second',
    name: 'Second campaign',
    priority: { reason: 'ending_soonest', deadline: first.ends_at, target_ids: ['second-10'] },
    drops: Array.from({ length: 12 }, (_, index) => ({
      ...first.drops[0]!,
      id: `second-${index}`,
      name: `Reward ${index}`,
    })),
  };
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns: [first, second] } },
  });
  await page.goto('/campaigns?view=list');
  await page.getByRole('button', { name: 'Open Autumn expedition', exact: true }).click();
  await page.getByRole('button', { name: 'Open Second campaign', exact: true }).click();
  await expect(page.getByText('Prioritizing', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Reward 10', exact: true }).click();
  await expect(page).toHaveURL(/drop=second-10/);
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(page.getByRole('complementary', { name: 'Campaign details' })).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Open Second campaign', exact: true }),
  ).toBeFocused();
  await page.goto('/campaigns?campaign=second&drop=second-10');
  const body = page.locator('.detail-body');
  await expect(page.locator('#drop-second-10')).toHaveClass(/selected/);
  await expect.poll(() => body.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await body.evaluate((element) => {
    element.scrollTop = 0;
  });
  await request.post('/__test/event', {
    headers,
    data: { event: 'channel_update', data: { ...fixture.channels[0], viewers: 999 } },
  });
  await expect.poll(() => body.evaluate((element) => element.scrollTop)).toBe(0);
});

test('history-only reward links resolve and stale responses cannot restore a cleared claim', async ({
  page,
  request,
}) => {
  const saved = await (await request.get('/api/history')).json();
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns: [] } },
  });
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=past-drop');
  await expect(page.locator('#history-drop-past-drop')).toHaveClass(/selected/);
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=missing');
  await expect(page.getByRole('status')).toContainText('No recorded claim for this reward.');
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(
    page.getByRole('button', { name: 'Open Autumn expedition', exact: true }),
  ).toBeVisible();
  await page.route('**/api/history', (route) => route.fulfill({ json: saved }));
  expect((await request.post('/api/cache/clear', { headers, data: {} })).ok()).toBeTruthy();
  await expect(
    page.getByRole('button', { name: 'Open Autumn expedition', exact: true }),
  ).toHaveCount(0);
});

test('a missed publication resyncs stale mining state without losing an unsaved edit', async ({
  page,
  request,
}) => {
  let upgraded = false;
  let dropNextPatch = false;
  let dropped = false;
  let resyncs = 0;
  await page.routeWebSocket('**/socket.io/**', (socket) => {
    const server = socket.connectToServer();
    socket.onMessage((message) => {
      if (message === '5') upgraded = true;
      if (message.toString().startsWith('42["state_resync"')) resyncs++;
      server.send(message);
    });
    server.onMessage((message) => {
      if (dropNextPatch && message.toString().startsWith('42["state_patch"')) {
        dropNextPatch = false;
        dropped = true;
      } else socket.send(message);
    });
  });
  await page.goto('/settings#connection');
  await expect.poll(() => upgraded).toBe(true);
  await page.route('**/api/settings', (route) =>
    route.request().method() === 'POST'
      ? route.fulfill({ status: 503, json: { detail: 'save_failed' } })
      : route.continue(),
  );
  const interval = page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true });
  await interval.fill('45');
  await expect(page.getByRole('alert')).toContainText('Changes could not be saved');
  dropNextPatch = true;
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, channels: [], current_drop: null } },
  });
  await expect.poll(() => dropped).toBe(true);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'channel_add',
      data: { ...fixture.channels[1], id: 3, name: 'replacement' },
    },
  });
  await expect.poll(() => resyncs).toBe(1);
  await expect(interval).toBeEnabled();
  await expect(interval).toHaveValue('45');
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'Now mining' }).getByRole('progressbar'),
  ).toHaveCount(0);
  await expect(
    page.locator('#channels-list').getByText('replacement', { exact: true }),
  ).toBeVisible();
  await expect(page.locator('#channels-list')).not.toContainText('northwind');
  await expect(page.locator('#channels-list')).not.toContainText('harbor');
  await expect(page.getByRole('alert')).toContainText('Changes could not be saved');
  await page.unroute('**/api/settings');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes,
    )
    .toBe(45);
});

test('Reload dashboard preserves real nested edits for review against newer settings', async ({
  page,
  request,
}) => {
  let upgraded = false;
  let connection!: WebSocketRoute;
  await page.routeWebSocket('**/socket.io/**', (socket) => {
    connection = socket;
    const server = socket.connectToServer();
    socket.onMessage((message) => {
      if (message === '5') upgraded = true;
      server.send(message);
    });
  });
  await page.goto('/?edit=priorities');
  await expect(page.getByRole('combobox', { name: 'Mining priority' })).toBeEnabled();
  await expect.poll(() => upgraded).toBe(true);
  await page.route('**/api/settings', (route) =>
    route.request().method() === 'POST'
      ? route.fulfill({ status: 503, json: { detail: 'save_failed' } })
      : route.continue(),
  );
  const allowed = page.getByRole('group', { name: 'Allowed reward types', exact: true });
  await allowed.getByRole('checkbox', { name: 'Badges', exact: true }).uncheck();
  await expect(page.getByRole('alert')).toContainText('Changes could not be saved');
  await page.getByRole('link', { name: 'Campaigns', exact: true }).click();
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Active', exact: true }).uncheck();
  await expect(page.getByRole('alert')).toContainText('Changes could not be saved');
  connection.send('42["protocol_mismatch",{"protocol":3}]');
  const reload = page.getByRole('button', { name: 'Reload dashboard', exact: true });
  await expect(reload).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try again', exact: true })).toBeDisabled();
  await page.unroute('**/api/settings');
  page.once('dialog', (dialog) => dialog.accept());
  await Promise.all([page.waitForEvent('load'), reload.click()]);
  await expect(page.getByRole('alert')).toContainText('Your unsaved edits were restored');
  const before = await (await request.get('/api/settings')).json();
  expect(before.mining_benefits.BADGE).toBe(true);
  expect(before.inventory_filters.show_active).toBe(true);
  expect(
    (
      await request.post('/api/settings', {
        headers,
        data: {
          revision: before.revision,
          mining_benefits: { EMOTE: false },
          inventory_filters: { show_upcoming: false },
          connection_quality: 5,
        },
      })
    ).ok(),
  ).toBe(true);
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await page.getByRole('link', { name: 'Edit', exact: true }).click();
  await expect(allowed.getByRole('checkbox', { name: 'Badges', exact: true })).not.toBeChecked();
  await expect(allowed.getByRole('checkbox', { name: 'Emotes', exact: true })).not.toBeChecked();
  await expect(page.getByRole('alert')).toContainText('Your unsaved edits were restored');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).mining_benefits.BADGE)
    .toBe(false);
  const saved = await (await request.get('/api/settings')).json();
  expect(saved.inventory_filters).toEqual({
    ...before.inventory_filters,
    show_active: false,
    show_upcoming: false,
  });
  expect(saved.mining_benefits).toEqual({ ...before.mining_benefits, BADGE: false, EMOTE: false });
  expect(saved.connection_quality).toBe(5);
  expect(saved.games_to_watch).toEqual(before.games_to_watch);
  await page.reload();
  await expect(page.getByRole('combobox', { name: 'Mining priority' })).toBeEnabled();
  await expect(page.getByText(/Your unsaved edits were restored/)).toHaveCount(0);
});
