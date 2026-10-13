import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
test.beforeEach(async ({ request }) => {
  await request.post('/__test/reset', { headers, data: {} });
});

for (const width of [1440, 390]) {
  test(`game priorities retain a stationary border at ${width}px`, async ({ page, request }) => {
    await request.post('/api/settings', {
      headers,
      data: {
        games_to_watch: Array.from({ length: 30 }, (_, index) => `Game ${index + 1}`),
      },
    });
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?edit=priorities');
    const list = page.getByRole('region', { name: 'Game priorities', exact: true });
    await expect(list.getByRole('listitem')).toHaveCount(30);
    await expect(list).toHaveCSS('border-top-width', '1px');
    await expect(list).toHaveCSS('border-bottom-width', '1px');
    await expect(list).toHaveCSS('overflow-y', 'auto');
    const before = await list.boundingBox();
    const search = page.getByRole('combobox', { name: 'Search games...' });
    const searchBefore = await search.boundingBox();
    await list.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await expect(
      list.getByRole('button', { name: 'Reorder Game 30', exact: true }),
    ).toBeInViewport();
    expect(await list.boundingBox()).toEqual(before);
    expect(await search.boundingBox()).toEqual(searchBefore);
    await page.screenshot({ path: `../artifacts/bordered-games-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });

  test(`Available and History scroll inside matching borders at ${width}px`, async ({
    page,
    request,
  }) => {
    const campaigns = Array.from({ length: 25 }, (_, index) => ({
      ...fixture.campaigns[0]!,
      id: `frame-${index}`,
      name: `Campaign ${String(index + 1).padStart(2, '0')}`,
    }));
    await request.post('/__test/event', {
      headers,
      data: {
        event: 'inventory_batch_update',
        data: { campaigns },
      },
    });
    const saved = await (await request.get('/api/history')).json();
    await page.route('**/api/history', async (route) =>
      route.fulfill({
        json: {
          ...saved,
          entries: campaigns.map((campaign) => ({
            ...saved.entries[0],
            id: `claim-${campaign.id}`,
            campaign_id: campaign.id,
            campaign: campaign.name,
          })),
        },
      }),
    );
    await page.setViewportSize({ width, height: 900 });
    for (const history of [false, true]) {
      await page.goto(`/campaigns?view=list&sort=name${history ? '&tab=history' : ''}`);
      const list = page.getByRole('region', {
        name: history ? 'History' : 'Campaigns',
        exact: true,
      });
      await expect(list.locator('.campaign-summary')).toHaveCount(25);
      await expect(list).toHaveCSS('border-top-width', '1px');
      await expect(list).toHaveCSS('border-bottom-width', '1px');
      await expect(list.locator('.campaign-list')).toHaveCSS('border-top-width', '0px');
      await expect(list).toHaveCSS('overflow-y', 'auto');
      const before = await list.boundingBox();
      const top = await page.evaluate(() => window.scrollY);
      await list.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
      await expect(
        list.getByRole('button', { name: 'Open Campaign 25', exact: true }),
      ).toBeInViewport();
      expect(await list.boundingBox()).toEqual(before);
      expect(await page.evaluate(() => window.scrollY)).toBe(top);
      await page.screenshot({
        path: `../artifacts/bordered-${history ? 'history' : 'campaigns'}-${width}.png`,
      });
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      await page
        .getByRole('searchbox', { name: 'Search campaigns and rewards' })
        .fill('no matching campaign');
      await expect(list.getByText('No matching results', { exact: true })).toBeVisible();
      await expect(list).toHaveCSS('border-top-width', '1px');
      await page.getByRole('button', { name: 'Change campaign layout', exact: true }).click();
      await expect(list).toHaveCSS('border-top-width', '0px');
      await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).fill('');
      await expect(list.locator('.campaign-summary').first()).toHaveCSS('border-top-width', '1px');
    }
  });
}
