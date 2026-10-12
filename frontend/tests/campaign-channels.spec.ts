import { expect, test, type APIRequestContext, type Page, type Route } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const allowed = Array.from({ length: 13 }, (_, index) => ({
  id: index + 1,
  login: `streamer${index + 1}`,
  name: `Streamer ${index + 1}`,
}));
const campaign = {
  ...fixture.campaigns[0]!,
  game_id: 1,
  allowed_channels: allowed,
  ends_at: '2099-10-02T00:00:00Z',
};

test.beforeEach(async ({ request }) => {
  expect((await request.get('/__test/health')).ok()).toBeTruthy();
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
  await request.post('/__test/event', {
    headers,
    data: { event: 'inventory_batch_update', data: { campaigns: [campaign] } },
  });
});

function response(offset = 0, id = campaign.id) {
  return {
    campaign_id: id,
    offset,
    total: allowed.length,
    channels: allowed.slice(offset, offset + 10).map((channel) => ({
      ...channel,
      online: channel.id === 3 ? false : channel.id === 4 ? null : true,
      eligible:
        channel.id === 2 || channel.id === 3
          ? false
          : channel.id === 4 || channel.id === 5
            ? null
            : true,
      game: channel.id === 2 ? 'Other category' : channel.id <= 5 ? null : 'Rust',
      checked_at: new Date().toISOString(),
    })),
  };
}

async function event(request: APIRequestContext, event: string, data: unknown) {
  expect(
    (await request.post('/__test/event', { headers, data: { event, data } })).ok(),
  ).toBeTruthy();
}

async function mockStatuses(page: Page) {
  const queries: { campaign_id: string; offset: number }[] = [];
  await page.route('**/api/campaigns/channels', async (route) => {
    const query = route.request().postDataJSON();
    queries.push(query);
    await route.fulfill({ json: response(query.offset, query.campaign_id) });
  });
  return queries;
}

for (const width of [1280, 375]) {
  test(`campaign participants distinguish availability and retain reward details at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const queries = await mockStatuses(page);
    await page.goto('/campaigns');
    const trigger = page
      .locator('.campaign-summary')
      .getByRole('button', { name: 'Campaign details', exact: true });
    await trigger.click();
    const detail = page.getByRole('complementary', { name: 'Campaign details' });
    const participants = detail.getByRole('region', { name: 'Participating channels' });
    await expect(
      participants.getByRole('button', { name: 'Watch Streamer 1', exact: true }),
    ).toBeEnabled();
    await expect(participants.getByText('Live · Not eligible here · Other category')).toBeVisible();
    await expect(participants.getByText('Offline', { exact: true })).toBeVisible();
    await expect(participants.getByText('Status unknown', { exact: true })).toBeVisible();
    await expect(
      participants.getByText('Live · Eligibility unknown', { exact: true }),
    ).toBeVisible();
    for (const id of [2, 3, 4, 5])
      await expect(
        participants.getByRole('button', { name: `Watch Streamer ${id}`, exact: true }),
      ).toBeDisabled();
    await expect(detail.getByRole('heading', { name: 'Explorer jacket' })).toHaveCount(1);
    await expect(detail.getByRole('heading', { name: 'Trail companion' })).toHaveCount(1);
    expect(queries).toEqual([{ campaign_id: campaign.id, offset: 0 }]);
    await participants.getByRole('button', { name: 'Next channels' }).click();
    await expect(
      participants.getByRole('button', { name: 'Watch Streamer 11', exact: true }),
    ).toBeEnabled();
    await expect(participants.getByText('11–13 of 13')).toBeVisible();
    await expect(participants.getByRole('link')).toHaveCount(3);
    expect(queries[1]).toEqual({ campaign_id: campaign.id, offset: 10 });
    await participants.getByRole('button', { name: 'Previous channels' }).click();
    await expect(
      participants.getByRole('button', { name: 'Watch Streamer 1', exact: true }),
    ).toBeEnabled();
    await detail.getByRole('heading', { name: campaign.name }).scrollIntoViewIfNeeded();
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.screenshot({ path: `../artifacts/campaign-channels-${width}.png` });
    await participants.getByRole('button', { name: 'Participating channels help' }).click();
    const help = participants.getByRole('note', { name: 'Participating channels help' });
    await expect(help).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(help).toBeHidden();
    await expect(detail).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(detail).toHaveCount(0);
    await expect(trigger).toBeFocused();
  });
}

test('Watch selects a manual login without changing games, pause, or shared reward progress', async ({
  page,
  request,
}) => {
  await mockStatuses(page);
  const before = await (await request.get('/api/settings')).json();
  await page.goto('/campaigns?campaign=campaign-1');
  const participants = page.getByRole('region', { name: 'Participating channels' });
  const selected = page.waitForRequest('**/api/channels/select');
  await participants.getByRole('button', { name: 'Watch Streamer 1', exact: true }).click();
  expect((await selected).postDataJSON()).toEqual({ channel: 'streamer1' });
  await expect(participants.getByText('Manual channel: streamer1')).toBeVisible();
  const after = await (await request.get('/api/settings')).json();
  expect(after.games_to_watch).toEqual(before.games_to_watch);
  expect(after.mining_paused).toBe(before.mining_paused);
  await expect(page.getByRole('heading', { name: 'Explorer jacket' })).toBeVisible();
  await participants.getByRole('button', { name: 'Return to Auto Mode' }).click();
  await expect(participants.getByText('Manual channel: streamer1')).toHaveCount(0);
  await event(request, 'manual_mode_update', {
    active: false,
    pending_channel: 'streamer1',
    error: 'Could not select this channel',
  });
  await expect(participants.getByRole('button', { name: 'Selecting Streamer 1' })).toBeDisabled();
  await expect(participants.getByText('Could not select this channel')).toBeVisible();
});

test('failed lookups remain unknown and stale page replies cannot replace the current page', async ({
  page,
}) => {
  const held: Route[] = [];
  let fail = true;
  await page.route('**/api/campaigns/channels', async (route) => {
    const { offset } = route.request().postDataJSON();
    if (fail) {
      await route.fulfill({ status: 503, json: { detail: 'request_failed' } });
      return;
    }
    if (offset === 0) {
      held.push(route);
      return;
    }
    await route.fulfill({ json: response(offset) });
  });
  await page.goto('/campaigns?campaign=campaign-1');
  const participants = page.getByRole('region', { name: 'Participating channels' });
  await expect(
    participants.getByText(
      'Channel status could not be checked. Retry or open the channel on Twitch.',
    ),
  ).toBeVisible();
  await expect(
    participants.getByRole('button', { name: 'Watch Streamer 1', exact: true }),
  ).toBeDisabled();
  fail = false;
  await participants.getByRole('button', { name: 'Refresh channel status' }).click();
  await expect.poll(() => held.length).toBe(1);
  await participants.getByRole('button', { name: 'Next channels' }).click();
  await expect(
    participants.getByRole('button', { name: 'Watch Streamer 11', exact: true }),
  ).toBeEnabled();
  await held[0]!.fulfill({ json: response(0) }).catch(() => {});
  await expect(participants.getByRole('link', { name: 'Streamer 1', exact: true })).toHaveCount(0);
});

test('history and unrestricted campaigns never invent participant lists or trigger lookups', async ({
  page,
  request,
}) => {
  const queries = await mockStatuses(page);
  await page.goto('/campaigns?tab=history&campaign=campaign-1');
  await expect(page.getByRole('complementary', { name: 'Campaign details' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Participating channels' })).toHaveCount(0);
  expect(queries).toEqual([]);
  await event(request, 'inventory_batch_update', {
    campaigns: [{ ...campaign, allowed_channels: [] }],
  });
  await page.goto('/campaigns?campaign=campaign-1');
  await expect(page.getByRole('heading', { name: 'Explorer jacket' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Participating channels' })).toHaveCount(0);
  expect(queries).toEqual([]);
});

test('only an open, visible detail refreshes its channel page', async ({ page }) => {
  await page.clock.install();
  let reads = 0;
  await page.route('**/api/campaigns/channels', async (route) => {
    reads += 1;
    const body = response();
    const now = await page.evaluate(() => new Date().toISOString());
    body.channels.forEach((channel) => {
      channel.checked_at = now;
    });
    await route.fulfill({ json: body });
  });
  await page.goto('/campaigns?campaign=campaign-1');
  const watch = page.getByRole('button', { name: 'Watch Streamer 1', exact: true });
  await expect(watch).toBeEnabled();
  expect(reads).toBe(1);
  await page.clock.runFor(31000);
  await expect.poll(() => reads).toBe(2);
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect(watch).toBeDisabled();
  await page.clock.runFor(61000);
  expect(reads).toBe(2);
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect(watch).toBeEnabled();
  expect(reads).toBe(3);
  await page.getByRole('button', { name: 'Close details' }).click();
  await expect(page.getByRole('region', { name: 'Participating channels' })).toHaveCount(0);
  await page.clock.runFor(61000);
  expect(reads).toBe(3);
});

test('background refresh preserves keyboard focus on a still-fresh Watch action', async ({
  page,
}) => {
  await page.clock.install();
  let pending: Route | undefined;
  let reads = 0;
  await page.route('**/api/campaigns/channels', async (route) => {
    if (++reads > 1) {
      pending = route;
      return;
    }
    const body = response();
    const now = await page.evaluate(() => new Date().toISOString());
    body.channels.forEach((channel) => {
      channel.checked_at = now;
    });
    await route.fulfill({ json: body });
  });
  await page.goto('/campaigns?campaign=campaign-1');
  const watch = page.getByRole('button', { name: 'Watch Streamer 1', exact: true });
  await expect(watch).toBeEnabled();
  await watch.focus();
  await page.clock.runFor(31000);
  await expect.poll(() => !!pending).toBe(true);
  await expect(watch).toBeEnabled();
  await expect(watch).toBeFocused();
  const body = response();
  const now = await page.evaluate(() => new Date().toISOString());
  body.channels.forEach((channel) => {
    channel.checked_at = now;
  });
  await pending!.fulfill({ json: body });
  await expect(watch).toBeFocused();
});
