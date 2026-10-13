import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mdiDockRight } from '@mdi/js';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const activity = Array.from({ length: 80 }, (_, id) => ({
  id,
  first_at: '2026-10-03T12:00:00Z',
  last_at: '2026-10-03T12:00:00Z',
  category: 'claims',
  severity: 'info',
  code: 'claimed',
  args: {},
  message: `Recorded reward ${id}`,
  campaign_id: 'campaign-1',
  drop_id: 'reward-1',
  channel_id: null,
  count: 1,
  recovered: false,
}));

test.beforeEach(async ({ request }) => {
  expect((await request.get('/__test/health')).ok()).toBeTruthy();
  expect(await (await request.post('/__test/reset', { headers, data: {} })).json()).toEqual({
    ok: true,
  });
});

test('mining preferences keeps detailed rules in accessible help without changing settings', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', { headers, data: { mining_priority_mode: 'short_events' } });
  let writes = 0;
  page.on('request', (request) => {
    if (request.method() === 'POST' && new URL(request.url()).pathname === '/api/settings')
      writes++;
  });
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 1024, height: 360 },
    { width: 390, height: 844 },
    { width: 320, height: 740 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/?edit=priorities');
    const priority = page.getByRole('combobox', { name: 'Mining priority', exact: true });
    await expect(priority).toBeEnabled();
    await expect(priority).toHaveAccessibleDescription(/24 hours or less/);
    await expect(
      page.getByText('Selected games first. Drag to reorder.', { exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByText('One phrase per line. Matches any part of a name.', { exact: true }),
    ).toHaveCount(0);
    const games = page.getByRole('region', { name: 'Game priorities', exact: true });
    const gamesTop = (await games.boundingBox())!.y;
    const priorityHelp = page.getByRole('button', { name: 'Mining priority help', exact: true });
    const rules = page.locator('#mining-priority-details');
    await expect(rules).toBeHidden();
    await priorityHelp.focus();
    await priorityHelp.press('Enter');
    await expect(rules).toBeVisible();
    await expect(rules).toContainText('Your saved drag order breaks ties.');
    expect((await games.boundingBox())!.y).toBe(gamesTop);
    const bounds = (await rules.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height);
    await page.keyboard.press('Escape');
    await expect(rules).toBeHidden();
    await expect(priorityHelp).toBeFocused();
    for (const [topic, detail] of [
      ['Also mine from other games', 'including required prerequisite drops'],
      ['Ignore rewards by name', 'case-insensitive literal substring'],
    ] as const) {
      const help = page.getByRole('button', { name: `${topic} help`, exact: true });
      await help.click();
      const note = page.getByRole('note', { name: `${topic} help`, exact: true });
      await expect(note).toBeVisible();
      await expect(note).toContainText(detail);
      await expect(page.locator(':popover-open')).toHaveCount(1);
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      await page.mouse.click(8, 8);
      await expect(note).toBeHidden();
    }
    await priorityHelp.scrollIntoViewIfNeeded();
    await priorityHelp.click();
    await page.getByRole('heading', { name: 'Mining preferences', exact: true }).click();
    await expect(rules).toBeHidden();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(viewport.width);
    await page.screenshot({
      path: `../artifacts/preferences-help-${viewport.width}.png`,
      fullPage: true,
    });
  }
  await page.context().setOffline(true);
  try {
    await request.post('/__test/reconnect', { headers, data: {} });
    await expect(
      page.getByRole('combobox', { name: 'Mining priority', exact: true }),
    ).toBeDisabled();
    for (const topic of ['Mining priority', 'Also mine from other games', 'Ignore rewards by name'])
      await expect(page.getByRole('button', { name: `${topic} help`, exact: true })).toBeEnabled();
    await expect(page.getByLabel('Ignore rewards by name', { exact: true })).toBeDisabled();
    await expect(
      page
        .getByRole('group', { name: 'Allowed reward types', exact: true })
        .getByRole('checkbox')
        .first(),
    ).toBeDisabled();
  } finally {
    await page.context().setOffline(false);
  }
  expect(writes).toBe(0);
});

test('Activity fits short desktop and phone viewports and distinguishes empty filters', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity } },
  });
  for (const viewport of [
    { width: 1440, height: 500 },
    { width: 390, height: 400 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await page.goto('/activity');
    const log = page.locator('#activity-list');
    await expect(log.locator('article')).toHaveCount(80);
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(
      viewport.height,
    );
    expect(await log.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    await expect(page.getByRole('heading', { name: 'Activity' })).toBeInViewport();
    await expect(page.getByRole('searchbox')).toBeInViewport();
    await page.getByRole('searchbox').fill('no such event');
    await expect(page.getByText('No matching activity', { exact: true })).toBeVisible();
    await page.screenshot({
      path: `../artifacts/consistency-activity-${viewport.width}-${viewport.height}.png`,
    });
  }
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, console: [], activity: [] } },
  });
  await expect(page.getByText('No activity yet', { exact: true })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('Activity aligns rows with optional details and lets longer messages grow', async ({
  page,
  request,
}) => {
  const rows = [
    { ...activity[0], message: 'Claimed reward' },
    { ...activity[1], message: 'Mining paused', campaign_id: null, drop_id: null },
    {
      ...activity[2],
      message: 'Connection restored after retrying the request. '.repeat(14),
      recovered: true,
      count: 3,
    },
    activity[3],
  ];
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity: rows } },
  });
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/activity');
    const articles = page.locator('#activity-list article');
    await expect(articles).toHaveCount(4);
    const bounds = await articles.evaluateAll((elements) =>
      elements.map((el) => {
        const row = el.getBoundingClientRect();
        return {
          height: row.height,
          centers: Array.from(el.children).map((child) => {
            const box = child.getBoundingClientRect();
            return box.y + box.height / 2 - row.y;
          }),
        };
      }),
    );
    expect(bounds[0]!.height).toBe(bounds[1]!.height);
    for (const row of bounds.slice(0, 2))
      expect(Math.max(...row.centers) - Math.min(...row.centers)).toBeLessThan(1);
    expect(bounds[2]!.height).toBeGreaterThan(bounds[0]!.height);
    const action = articles.first().getByRole('link', { name: 'Campaign details' });
    const box = (await action.boundingBox())!;
    expect(box.width).toBeGreaterThanOrEqual(width < 768 ? 44 : 36);
    expect(box.height).toBeGreaterThanOrEqual(width < 768 ? 44 : 36);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await articles.first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: `../artifacts/activity-row-alignment-${width}.png` });
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('Settings tabs keep their position, drafts and browser history', async ({ page }) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 700 });
    await page.goto('/settings#account');
    const tabs = page.getByRole('navigation', { name: 'Settings sections' });
    const top = (await tabs.boundingBox())!.y;
    await tabs.getByRole('link', { name: 'Dashboard access' }).click();
    await page
      .getByLabel('New password (at least 8 characters)', { exact: true })
      .fill('unsaved-draft');
    for (const name of ['Connection', 'Maintenance', 'Twitch account', 'Dashboard access']) {
      await tabs.getByRole('link', { name, exact: true }).click();
      expect((await tabs.boundingBox())!.y).toBeCloseTo(top, 0);
      await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeInViewport();
      expect(await page.evaluate(() => scrollY)).toBe(0);
    }
    await expect(
      page.getByLabel('New password (at least 8 characters)', { exact: true }),
    ).toHaveValue('unsaved-draft');
    await page.goBack();
    await expect(tabs.getByRole('link', { name: 'Twitch account' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect((await tabs.boundingBox())!.y).toBeCloseTo(top, 0);
  }
});

test('Mining and Activity details return to the originating link and scroll position', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity } },
  });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 700 });
    for (const id of [
      'mining-drop-details',
      'up-next-campaign-campaign-1',
      'up-next-drop-campaign-1-reward-2',
      'activity-campaign-30',
    ]) {
      const path = id.startsWith('activity') ? '/activity' : '/';
      await page.goto(path);
      const link = page.locator(`#${id}`);
      await link.evaluate((element) => element.scrollIntoView({ block: 'center' }));
      const top = await page.evaluate(() => scrollY);
      const lists = await page
        .locator('[data-restore-scroll]')
        .evaluateAll((elements) => elements.map((el) => el.scrollTop));
      if (path === '/activity')
        await expect(link.locator('path')).toHaveAttribute('d', mdiDockRight);
      await link.click();
      await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
      await page.getByRole('button', { name: 'Close details' }).click();
      await expect(page).toHaveURL(new RegExp(path === '/' ? '/$' : '/activity$'));
      await expect(link).toBeFocused();
      await expect.poll(() => page.evaluate(() => scrollY)).toBeCloseTo(top, 0);
      expect(
        await page
          .locator('[data-restore-scroll]')
          .evaluateAll((elements) => elements.map((el) => el.scrollTop)),
      ).toEqual(lists);
      await link.press('Enter');
      await page.goBack();
      await expect(link).toBeFocused();
    }
  }
  await page.goto('/campaigns?campaign=campaign-1&drop=reward-1');
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(page).toHaveURL(/\/campaigns$/);
});

test('Detail return retains filters through query edits and reveals its trigger after resize', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, activity } },
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/activity');
  await page.getByRole('searchbox').fill('reward 3');
  await page.getByRole('combobox', { name: 'Filter category' }).selectOption('claims');
  await page.getByRole('combobox', { name: 'Filter level' }).selectOption('info');
  await page.locator('#activity-campaign-33').click();
  await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
  await page.getByRole('searchbox').fill('Autumn');
  await page.getByRole('combobox', { name: 'Sort campaigns' }).selectOption('newest');
  await page.getByRole('button', { name: 'Change campaign layout' }).click();
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Active', exact: true }).uncheck();
  await page.setViewportSize({ width: 390, height: 500 });
  await page.getByRole('button', { name: 'Close details' }).press('Escape');
  await expect(page.getByRole('searchbox')).toHaveValue('reward 3');
  await expect(page.getByRole('combobox', { name: 'Filter category' })).toHaveValue('claims');
  await expect(page.getByRole('combobox', { name: 'Filter level' })).toHaveValue('info');
  await expect(page.locator('#activity-campaign-33')).toBeFocused();
  await expect(page.locator('#activity-campaign-33')).toBeInViewport();
  await expect(page.locator('#activity-list article')).toHaveCount(11);
  await page.goto('/');
  await page.getByRole('searchbox', { name: 'Search channels' }).fill('north');
  await page.locator('#mining-drop-details').click();
  await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('searchbox', { name: 'Search channels' })).toHaveValue('north');
  await expect(page.locator('#channels-list .row')).toHaveCount(1);
});

test('Available rewards show claims once with truthful times and uncropped artwork', async ({
  page,
  request,
}) => {
  const image = 'https://example.test/reward.png';
  await page.route('https://example.test/**', (route) =>
    route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aVRsAAAAASUVORK5CYII=',
        'base64',
      ),
    }),
  );
  const campaign = structuredClone(fixture.campaigns[0]!);
  campaign.game_box_art_url = image;
  campaign.claimed_drops = 1;
  campaign.drops[0]!.is_claimed = true;
  campaign.drops[0]!.benefits.push({
    name: 'Additional benefit',
    type: 'DIRECT_ENTITLEMENT',
    image_url: image,
  });
  campaign.drops.forEach((drop) => {
    drop.benefits[0]!.image_url = image;
  });
  campaign.drops[0]!.benefits[0]!.image_url = '';
  const entries = campaign.drops.map((drop) => ({
    id: drop.id,
    campaign_id: campaign.id,
    campaign: campaign.name,
    game: campaign.game_name,
    drop_name: drop.name,
    benefits: [drop.name],
    required_minutes: drop.required_minutes,
    claimed_at: '2026-10-03T06:24:00Z',
    claimed_at_is_observed: false,
    image_url: image,
  }));
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, campaigns: [campaign] } },
  });
  await page.route('**/api/history', async (route) =>
    route.fulfill({ json: { ...(await (await route.fetch()).json()), entries } }),
  );
  await page.goto('/');
  await expect(page.locator('[aria-labelledby="mining-heading"] img')).toHaveCSS(
    'object-fit',
    'cover',
  );
  campaign.drops[0]!.benefits[0]!.image_url = image;
  await request.post('/__test/event', {
    headers,
    data: { event: 'initial_state', data: { ...fixture, campaigns: [campaign] } },
  });
  await expect(page.locator('[aria-labelledby="mining-heading"] img')).toHaveCSS(
    'object-fit',
    'contain',
  );
  await page.locator('#mining-drop-details').click();
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  const claimed = page.locator('#drop-reward-1');
  await expect(claimed.getByText(/^Claimed .+2026/)).toBeVisible();
  await expect(claimed.getByText('Explorer jacket', { exact: true })).toHaveCount(1);
  await expect(claimed.getByText('Additional benefit', { exact: true })).toBeVisible();
  await expect(claimed).toHaveCSS('border-left-width', '0px');
  await expect(claimed).toHaveCSS('padding-left', '0px');
  await expect(claimed).toHaveAttribute('aria-current', 'true');
  await expect(detail.getByRole('heading', { name: 'History', exact: true })).toHaveCount(0);
  await expect(page.locator('#drop-reward-2').getByText(/^Claimed/)).toHaveCount(0);
  for (const art of await detail.locator('.reward-detail img').all())
    await expect(art).toHaveCSS('object-fit', 'contain');
  entries[0]!.claimed_at_is_observed = true;
  await page.reload();
  await expect(claimed.getByText(/^Claimed: First observed/)).toBeVisible();
  await page.goto('/campaigns?tab=history&campaign=campaign-1&drop=reward-1');
  await expect(page.locator('#history-drop-reward-1 img')).toHaveCSS('object-fit', 'contain');
  await expect(page.locator('#history-drop-reward-1')).toHaveCSS('border-left-width', '0px');
  await expect(detail.getByRole('progressbar')).toHaveCount(0);
});

test('Up next shares identical windows but retains distinct dates and upcoming labels', async ({
  page,
  request,
}) => {
  const state = structuredClone(fixture);
  const drops = state.wanted_items[0]!.campaigns[0]!.drops;
  drops[1]!.starts_at = '2026-09-20T02:00:00+02:00';
  drops[1]!.ends_at = '2026-10-02T02:00:00+02:00';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await page.goto('/');
  const queue = page.locator('#up-next-list');
  await expect(queue.getByText(/^Ends:/)).toHaveCount(1);
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(0);
  drops[1]!.ends_at = '2026-10-03T00:00:00Z';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(2);
  drops[1]!.eligibility = 'upcoming';
  await request.post('/__test/event', { headers, data: { event: 'initial_state', data: state } });
  await expect(queue.locator('li').getByText(/^Starts:/)).toHaveCount(1);
  await expect(queue.locator('li').getByText(/^Ends:/)).toHaveCount(1);
});

test('Mining lists have modest gutters, inset separators and full phone touch targets', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  for (const id of ['channels-list', 'up-next-list']) {
    const list = page.locator(`#${id}`);
    await expect(list).toHaveCSS('padding-right', '4px');
    const first = list.locator(':scope > div').first();
    await expect(first).toHaveCSS('margin-left', '16px');
    await expect(first).toHaveCSS('margin-right', '16px');
  }
  await page.goto('/?edit=priorities');
  await expect(page.getByRole('region', { name: 'Game priorities' })).toHaveCSS(
    'padding-right',
    '4px',
  );
  await expect(page.locator('[data-game]').first()).toHaveCSS('margin-left', '16px');
  await page.setViewportSize({ width: 390, height: 844 });
  const grip = page.locator('.drag-handle').first();
  expect((await grip.boundingBox())!.width).toBe(44);
  expect((await grip.boundingBox())!.height).toBe(44);
  await grip.hover();
  await expect(grip).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  const check = page
    .getByRole('group', { name: 'Allowed reward types', exact: true })
    .getByRole('checkbox', { name: 'Badges', exact: true })
    .locator('..');
  expect((await check.boundingBox())!.height).toBeGreaterThanOrEqual(44);
  await page.getByRole('combobox', { name: 'Search games...' }).fill('Elder');
  const result = page.getByRole('listbox', { name: 'Search games...' }).getByRole('option').first();
  expect((await result.boundingBox())!.height).toBeGreaterThanOrEqual(44);
});

test('Campaigns keeps the total beside refresh and gives search the full phone width', async ({
  page,
}) => {
  for (const width of [320, 390, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    for (const tab of ['', '?tab=history']) {
      await page.goto(`/campaigns${tab}`);
      const tabs = (await page.getByRole('navigation', { name: 'Campaign views' }).boundingBox())!;
      const refresh = (await page
        .getByRole('button', { name: 'Refresh inventory', exact: true })
        .boundingBox())!;
      if (width >= 768)
        expect(Math.abs(tabs.y + tabs.height / 2 - refresh.y - refresh.height / 2)).toBeLessThan(1);
      const count = (await page.locator('.campaign-refresh p').boundingBox())!;
      expect(count.x + count.width).toBeLessThan(refresh.x);
      expect(Math.abs(count.y + count.height / 2 - refresh.y - refresh.height / 2)).toBeLessThan(1);
      const search = (await page.getByRole('searchbox').boundingBox())!;
      if (width < 768) expect(search.width).toBe(width - 32);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await page.screenshot({
        path: `../artifacts/consistency-campaigns-${width}-${tab ? 'history' : 'available'}.png`,
      });
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
