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
