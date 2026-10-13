import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const channels = Array.from({ length: 57 }, (_, index) => ({
  ...fixture.channels[0]!,
  id: index + 1,
  login: `streamer${index + 1}`,
  name: `Streamer ${index + 1}`,
  viewers: 1000 - index,
  watching: index === 56,
}));

test.beforeEach(async ({ request }) => {
  expect((await request.get('/__test/health')).ok()).toBeTruthy();
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
  await request.post('/__test/event', {
    headers,
    data: { event: 'channels_batch_update', data: { channels } },
  });
});

for (const width of [1280, 320]) {
  test(`Channels paginates the full searchable list at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    const list = page.getByRole('region', { name: 'Channels', exact: true });
    const pages = page.getByRole('navigation', { name: 'Channel pages' });
    await expect(list.locator('.row')).toHaveCount(25);
    await expect(list.locator('.row').first()).toContainText('Streamer 57');
    await expect(pages).toContainText('1 / 3');
    await expect(pages.getByRole('button', { name: 'Previous page' })).toBeDisabled();
    await list.evaluate((el) => {
      el.scrollTop = 200;
    });
    const next = pages.getByRole('button', { name: 'Next page' });
    await next.focus();
    await next.press('Enter');
    await expect(pages).toContainText('2 / 3');
    await expect(next).toBeFocused();
    await expect(list.locator('.row').first()).toContainText('Streamer 25');
    expect(await list.evaluate((el) => el.scrollTop)).toBe(0);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.screenshot({ path: `../artifacts/channels-${width}.png`, fullPage: true });
    await next.click();
    await expect(pages).toContainText('3 / 3');
    await expect(next).toBeDisabled();
    await expect(list.locator('.row')).toHaveCount(7);
    await page.reload();
    await expect(pages).toContainText('3 / 3');
    await page.getByRole('searchbox', { name: 'Search channels' }).fill('STREAMER 56');
    await expect(list.locator('.row')).toHaveCount(1);
    await expect(list.getByRole('link', { name: 'Streamer 56', exact: true })).toBeVisible();
    await expect(pages).toHaveCount(0);
    expect(new URL(page.url()).searchParams.has('channels_page')).toBe(false);
    await page.getByRole('button', { name: 'Clear search' }).click();
    await expect(pages).toContainText('1 / 3');
    await expect(list.locator('.row').first()).toContainText('Streamer 57');
  });
}

test('Channel pages clamp invalid links and shrinking live lists, and Watch preserves settings', async ({
  page,
  request,
}) => {
  const list = page.getByRole('region', { name: 'Channels', exact: true });
  const pages = page.getByRole('navigation', { name: 'Channel pages' });
  for (const value of ['-1', 'NaN', 'Infinity', '1.5']) {
    await page.goto(`/?channels_page=${value}`);
    await expect(pages).toContainText('1 / 3');
    await expect(list.locator('.row')).toHaveCount(25);
  }
  await page.goto('/?channels_page=999');
  await expect(pages).toContainText('3 / 3');
  await expect(list.locator('.row')).toHaveCount(7);
  await pages.getByRole('button', { name: 'Previous page' }).click();
  const before = await (await request.get('/api/settings')).json();
  const selected = page.waitForRequest('**/api/channels/select');
  await list.getByRole('button', { name: 'Watch Streamer 25', exact: true }).click();
  expect((await selected).postDataJSON()).toEqual({ channel_id: 25 });
  await expect(page.getByRole('button', { name: 'Return to Auto Mode' })).toBeVisible();
  expect(await (await request.get('/api/settings')).json()).toEqual(before);
  await request.post('/__test/event', {
    headers,
    data: { event: 'channels_batch_update', data: { channels: channels.slice(0, 2) } },
  });
  await expect(list.locator('.row')).toHaveCount(2);
  await expect(list.getByRole('link', { name: 'Streamer 1', exact: true })).toBeVisible();
  await expect(pages).toHaveCount(0);
});

test('Campaign details have no streamer list or status lookup', async ({ page, request }) => {
  const campaign = {
    ...fixture.campaigns[0]!,
    allowed_channels: [{ login: 'exclusive_streamer', name: 'Exclusive streamer' }],
  };
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns: [campaign] } },
  });
  const queries: string[] = [];
  page.on('request', (request) => {
    if (request.url().endsWith('/api/campaigns/channels')) queries.push(request.url());
  });
  await page.goto(`/campaigns?campaign=${campaign.id}`);
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  await expect(detail.getByRole('heading', { name: 'Explorer jacket' })).toBeVisible();
  await expect(detail.getByText('Exclusive streamer', { exact: true })).toHaveCount(0);
  await expect(detail.getByRole('heading', { name: 'Participating channels' })).toHaveCount(0);
  expect(queries).toEqual([]);
  expect(
    (
      await request.post('/api/campaigns/channels', { headers, data: { campaign_id: campaign.id } })
    ).status(),
  ).toBe(404);
});

const avatar = 'https://static-cdn.jtvnw.net/offline-avatar.png';
const offline = {
  ...fixture.channels[0]!,
  id: 801,
  login: 'sleepy_streamer',
  name: 'Sleepy Streamer',
  online: false,
  watching: false,
  game: null,
  viewers: null,
  avatar_url: avatar,
};
const found = [
  {
    id: 801,
    login: offline.login,
    name: offline.name,
    online: false,
    game: null,
    avatar_url: avatar,
  },
  {
    id: 802,
    login: 'awake_streamer',
    name: 'Awake Streamer',
    online: true,
    game: 'Rust',
    avatar_url: null,
  },
];

for (const viewport of [
  { width: 1280, height: 900 },
  { width: 320, height: 740 },
  { width: 1024, height: 360 },
]) {
  test(`manual Twitch search shows avatars and accepts offline channels at ${viewport.width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize(viewport);
    const queries: unknown[] = [];
    await page.route('**/api/channels/search', (route) => {
      queries.push(route.request().postDataJSON());
      return route.fulfill({ json: found });
    });
    await page.route(avatar, (route) =>
      route.fulfill({
        contentType: 'image/png',
        body: Buffer.from(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==',
          'base64',
        ),
      }),
    );
    await request.post('/__test/event', {
      headers,
      data: { event: 'channels_batch_update', data: { channels: [offline] } },
    });
    await page.goto('/');
    expect(queries).toEqual([]);
    const before = await (await request.get('/api/settings')).json();
    await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
    const input = page.getByRole('combobox', { name: 'Twitch channel name or URL' });
    await page.getByRole('spinbutton', { name: 'Auto mode after (minutes, optional)' }).fill('15');
    await input.fill('sleep');
    const results = page.getByRole('listbox', { name: 'Twitch channel name or URL' });
    await expect(results.getByRole('option')).toHaveCount(2);
    expect(queries).toEqual([{ search: 'sleep' }]);
    const sleeping = results.getByRole('option').first();
    await expect(sleeping).toContainText('@sleepy_streamer');
    await expect(sleeping).toContainText('Offline');
    await expect(sleeping.locator('img')).toHaveAttribute('src', avatar);
    await expect(results.getByRole('option').last()).toContainText('Live, Rust');
    const popup = (await page.locator('.search-popover').boundingBox())!;
    expect(popup.y).toBeGreaterThanOrEqual(8);
    expect(popup.y + popup.height).toBeLessThanOrEqual(viewport.height - 8);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      viewport.width,
    );
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({
      path: `../artifacts/channel-search-${viewport.width}.png`,
      fullPage: true,
    });
    const selection = page.waitForRequest('**/api/channels/select');
    await input.press('ArrowDown');
    await input.press('Enter');
    expect((await selection).postDataJSON()).toEqual({
      channel: 'sleepy_streamer',
      duration_minutes: 15,
    });
    await expect(results).toBeHidden();
    const mining = page.getByRole('region', { name: 'Now mining', exact: true });
    await expect(mining).toContainText('Waiting for Sleepy Streamer to go live');
    await expect(mining.locator('img')).toHaveAttribute('src', avatar);
    await expect(mining.locator('img')).toHaveCSS('width', '32px');
    await expect(mining.locator('img')).toHaveCSS('height', '32px');
    await expect(mining.getByRole('progressbar')).toHaveCount(0);
    await expect(page.getByText(/^Auto mode at /)).toBeVisible();
    expect(await (await request.get('/api/settings')).json()).toEqual(before);
    await page.screenshot({
      path: `../artifacts/channel-waiting-${viewport.width}.png`,
      fullPage: true,
    });
    await request.post('/__test/event', {
      headers,
      data: {
        event: 'channels_batch_update',
        data: { channels: [{ ...offline, online: true, watching: true, game: 'Rust' }] },
      },
    });
    await expect(mining).toContainText('Watching Sleepy Streamer');
    await expect(mining.locator('img')).toHaveAttribute('src', avatar);
    const streamerLink = mining.getByRole('link', { name: offline.name, exact: true });
    await expect(streamerLink).toHaveText(offline.name);
    await expect(streamerLink).toHaveAttribute('href', 'https://www.twitch.tv/sleepy_streamer');
    await expect(streamerLink).toHaveAttribute('target', '_blank');
    await expect(streamerLink).toHaveAttribute('rel', 'noreferrer');
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: fixture.current_drop },
    });
    await expect(mining.getByRole('progressbar')).toBeVisible();
    await expect(mining.locator(`img[src="${avatar}"]`)).toBeVisible();
    await expect(streamerLink).toHaveAttribute('href', 'https://www.twitch.tv/sleepy_streamer');
    await page.getByRole('button', { name: 'Return to Auto Mode' }).click();
    await expect(page.getByRole('img', { name: 'Automatic selection', exact: true })).toBeVisible();
  });
}

test('manual channel search fences old replies and preserves exact entry after search failure', async ({
  page,
}) => {
  let release!: () => void;
  const pending = new Promise<void>((done) => {
    release = done;
  });
  await page.route('**/api/channels/search', async (route) => {
    if (route.request().postDataJSON().search === 'old') {
      await pending;
      await route.fulfill({ json: found }).catch(() => {});
    } else {
      await route.fulfill({ status: 503, json: { error: 'request_failed' } });
    }
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  const input = page.getByRole('combobox', { name: 'Twitch channel name or URL' });
  const old = page.waitForRequest('**/api/channels/search');
  await input.fill('old');
  await old;
  await input.fill('https://www.twitch.tv/extra_streamer');
  await expect(
    page.getByText('Could not search Twitch. You can still enter an exact channel name or URL.'),
  ).toBeVisible();
  release();
  await expect(page.getByRole('option')).toHaveCount(0);
  const selection = page.waitForRequest('**/api/channels/select');
  await input.press('Enter');
  expect((await selection).postDataJSON().channel).toBe('https://www.twitch.tv/extra_streamer');
  await expect(page.getByText('Watching extra_streamer', { exact: true })).toBeVisible();
});

test('choosing a channel suggestion validates the timer before issuing a selection', async ({
  page,
}) => {
  await page.route('**/api/channels/search', (route) => route.fulfill({ json: found }));
  const selections: unknown[] = [];
  page.on('request', (request) => {
    if (request.url().endsWith('/api/channels/select')) selections.push(request.postDataJSON());
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  const input = page.getByRole('combobox', { name: 'Twitch channel name or URL' });
  const timer = page.getByRole('spinbutton', { name: 'Auto mode after (minutes, optional)' });
  await timer.fill('0');
  await input.fill('sleep');
  await expect(page.getByRole('option')).toHaveCount(2);
  await page.getByRole('option').first().click();
  expect(await timer.evaluate((element: HTMLInputElement) => element.validity.rangeUnderflow)).toBe(
    true,
  );
  expect(selections).toEqual([]);
  await expect(input).toHaveValue('sleepy_streamer');
  await expect(timer).toBeFocused();
  await timer.fill('15');
  await timer.press('Enter');
  await expect(page.getByText('Watching sleepy_streamer', { exact: true })).toBeVisible();
  expect(selections).toEqual([{ channel: 'sleepy_streamer', duration_minutes: 15 }]);
});
