import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
const stardew = {
  id: '490744',
  name: 'Stardew Valley',
  box_art_url: 'https://static-cdn.jtvnw.net/ttv-boxart/490744-{width}x{height}.jpg',
};

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 390, height: 844 },
  { width: 1024, height: 360 },
]) {
  test(`game suggestions overlay the unchanged list at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const games = Array.from({ length: 15 }, (_, index) => ({
      ...stardew,
      id: `${index}`,
      name: `Search result ${index + 1}`,
    }));
    await page.route('**/api/games', (route) =>
      route.fulfill({
        json: route.request().postDataJSON().search ? games : [],
      }),
    );
    await page.goto('/?edit=priorities');
    const search = page.getByRole('combobox', { name: 'Search games' });
    const selected = page.getByRole('region', { name: 'Game priorities', exact: true });
    const before = await selected.boundingBox();
    await search.fill('search');
    const results = page.getByRole('listbox');
    await expect(results.getByRole('option')).toHaveCount(15);
    const popup = page.locator('.search-popover');
    const box = (await popup.boundingBox())!;
    const field = (await search.boundingBox())!;
    expect(box.x).toBe(field.x);
    expect(box.width).toBe(field.width);
    expect(box.height).toBeLessThanOrEqual(320);
    if (viewport.height > 500) expect(box.height).toBe(320);
    expect(box.y).toBeGreaterThanOrEqual(8);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height - 8);
    expect(await selected.boundingBox()).toEqual(before);
    await results.getByRole('option').last().scrollIntoViewIfNeeded();
    await expect(results.getByRole('option').last()).toBeInViewport();
    expect(await selected.boundingBox()).toEqual(before);
    await search.press('Escape');
    await expect(results).toBeHidden();
    await expect(search).toHaveValue('search');
    await search.press('ArrowDown');
    await expect(results).toBeVisible();
    for (let index = 0; index < 10; index++) await search.press('ArrowDown');
    await expect(results.getByRole('option').nth(10)).toBeInViewport({ ratio: 0.99 });
    await expect(search).toBeFocused();
    expect(await selected.boundingBox()).toEqual(before);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/game-dropdown-${viewport.width}.png` });
    await page.getByRole('heading', { name: 'Mining preferences', exact: true }).click();
    await expect(results).toBeHidden();
    await search.click();
    await expect(results).toBeVisible();
    await page.setViewportSize({ ...viewport, height: viewport.height - 80 });
    await expect
      .poll(async () => {
        const box = (await popup.boundingBox())!;
        return box.y >= 8 && box.y + box.height <= viewport.height - 88;
      })
      .toBe(true);
    await search.press('Tab');
    await page.keyboard.press('Tab');
    await page.keyboard.press('Tab');
    await expect(results).toBeHidden();
  });
}

test('keyboard selection stays on the input and cannot select an old query', async ({
  page,
  request,
}) => {
  await page.route('**/api/games', (route) =>
    route.fulfill({
      json: route.request().postDataJSON().search
        ? [stardew, { ...stardew, id: '2', name: 'Starbound' }]
        : [],
    }),
  );
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  await search.fill('star');
  const results = page.getByRole('listbox');
  await expect(results.getByRole('option')).toHaveCount(2);
  await search.press('ArrowDown');
  await expect(results.getByRole('option', { name: stardew.name })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await search.press('ArrowDown');
  await expect(results.getByRole('option', { name: 'Starbound' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(search).toBeFocused();
  await search.fill('stardew');
  await expect(search).not.toHaveAttribute('aria-activedescendant');
  await expect(results.getByRole('option')).toHaveCount(2);
  await search.press('ArrowUp');
  await search.press('ArrowUp');
  await search.press('Enter');
  await expect(results).toBeHidden();
  await expect(search).toBeFocused();
  await expect(search).toHaveValue('');
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([stardew]);
});

test('dismissed pending searches stay closed when the result arrives', async ({ page }) => {
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => (finish = resolve));
  await page.route('**/api/games', async (route) => {
    if (route.request().postDataJSON().search) await gate;
    await route.fulfill({ json: [stardew] });
  });
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  try {
    await search.fill('stardew');
    await search.press('Escape');
  } finally {
    finish();
  }
  await expect(page.getByRole('button', { name: 'Add Game', exact: true })).toBeEnabled();
  await expect(page.getByRole('listbox')).toBeHidden();
  await expect(search).toHaveValue('stardew');
  await search.press('ArrowDown');
  await expect(page.getByRole('option', { name: stardew.name })).toBeVisible();
});

test('suggestions follow the field when an autosave error shifts the page', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  let finish!: () => void;
  let saving = false;
  const gate = new Promise<void>((resolve) => (finish = resolve));
  await page.route('**/api/settings', async (route) => {
    if (route.request().method() !== 'POST') return route.continue();
    saving = true;
    await gate;
    await route.fulfill({ status: 409, json: { detail: 'settings_conflict' } });
  });
  await page.route('**/api/games', (route) =>
    route.fulfill({ json: route.request().postDataJSON().search ? [stardew] : [] }),
  );
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  let before = 0;
  try {
    await page
      .getByRole('group', { name: 'Also mine from other games' })
      .getByLabel('Badges')
      .check();
    await expect.poll(() => saving).toBe(true);
    await search.fill('stardew');
    await expect(page.getByRole('option', { name: stardew.name })).toBeVisible();
    before = (await search.boundingBox())!.y;
  } finally {
    finish();
  }
  await expect(page.getByRole('alert')).toBeVisible();
  await expect
    .poll(async () => {
      const field = (await search.boundingBox())!;
      const popup = (await page.locator('.search-popover').boundingBox())!;
      return field.y > before && popup.y === field.y + field.height + 4;
    })
    .toBe(true);
  await search.press('ArrowDown');
  await expect(page.getByRole('option', { name: stardew.name })).toHaveAttribute(
    'aria-selected',
    'true',
  );
});

test.describe('touch game selection', () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 844 } });
  test('a tap selects a game and closes suggestions', async ({ page }) => {
    await page.route('**/api/games', (route) => route.fulfill({ json: [stardew] }));
    await page.goto('/?edit=priorities');
    await page.getByRole('combobox', { name: 'Search games' }).fill('stardew');
    await page.getByRole('option', { name: stardew.name }).tap();
    await expect(page.locator('[data-game="Stardew Valley"]')).toBeVisible();
    await expect(page.getByRole('listbox')).toBeHidden();
  });
});

test('older webviews can search, dismiss and select without the Popover API', async ({ page }) => {
  await page.addInitScript(() => {
    delete (HTMLElement.prototype as Partial<HTMLElement>).showPopover;
    delete (HTMLElement.prototype as Partial<HTMLElement>).hidePopover;
  });
  await page.route('**/api/games', (route) =>
    route.request().postDataJSON().search === 'fail'
      ? route.fulfill({ status: 503, json: { detail: 'request_failed' } })
      : route.fulfill({ json: [stardew] }),
  );
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  const result = page.getByRole('option', { name: stardew.name });
  await search.fill('stardew');
  await expect(result).toBeInViewport();
  await page.getByRole('heading', { name: 'Mining preferences', exact: true }).click();
  await expect(result).toBeHidden();
  await search.click();
  await expect(result).toBeVisible();
  await search.press('Escape');
  await expect(result).toBeHidden();
  await expect(search).toHaveValue('stardew');
  await search.press('ArrowDown');
  await search.press('Enter');
  await expect(page.locator('[data-game="Stardew Valley"]')).toBeVisible();
  await expect(result).toBeHidden();
  await search.fill('fail');
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await retry.focus();
  await retry.press('Escape');
  await expect(retry).toBeHidden();
  await expect(search).toBeFocused();
  await expect(search).toHaveValue('fail');
});
test.beforeEach(async ({ request, page }) => {
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
  await page.route('https://static-cdn.jtvnw.net/**', (route) =>
    route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lVEAAAAASUVORK5CYII=',
        'base64',
      ),
    }),
  );
});

test('search adds a game without campaigns and persists its official name and cover', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route('**/api/games', (route) =>
    route.fulfill({ json: route.request().postDataJSON().search ? [stardew] : [] }),
  );
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  await search.fill('stardew');
  const result = page.getByRole('option', { name: 'Stardew Valley', exact: true });
  await expect(result).toBeVisible();
  await expect(result.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await result.click();
  const row = page.locator('[data-game="Stardew Valley"]');
  await expect(row).toBeVisible();
  await expect(row.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([stardew]);
  await page.reload();
  await expect(row.locator('img')).toHaveAttribute('src', /490744-80x112.jpg/);
  await page.screenshot({ path: '../artifacts/game-search-mobile.png', fullPage: true });
  await row.getByRole('button', { name: 'Remove Stardew Valley' }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([]);
});

test('old Unicode saved names gain covers without a campaign and retain their priority', async ({
  page,
  request,
}) => {
  const state = structuredClone(fixture);
  state.settings.games_to_watch = ['Rust', 'STRASSE'];
  const metadata = { ...stardew, name: 'Straße' };
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: { event: 'initial_state', data: state },
      })
    ).ok(),
  ).toBeTruthy();
  await page.route('**/api/games', (route) => route.fulfill({ json: [metadata] }));
  await page.goto('/?edit=priorities');
  await expect(page.locator('[data-game="STRASSE"] img')).toHaveAttribute(
    'src',
    /490744-80x112.jpg/,
  );
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([metadata]);
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Rust',
    'STRASSE',
  ]);
});

test('changed queries ignore stale results and failed searches can retry or add a name', async ({
  page,
}) => {
  let finishSlow: (() => void) | undefined;
  await page.route('**/api/games', async (route) => {
    const query = route.request().postDataJSON();
    if (query.names) return route.fulfill({ json: [] });
    if (query.search === 'slow') {
      await new Promise<void>((resolve) => {
        finishSlow = resolve;
      });
      return route.fulfill({ json: [{ ...stardew, name: 'Old result' }] });
    }
    if (query.search === 'fail')
      return route.fulfill({ status: 503, json: { detail: 'request_failed' } });
    return route.fulfill({ json: [stardew] });
  });
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  await search.fill('slow');
  await expect.poll(() => !!finishSlow).toBe(true);
  await expect(page.getByRole('listbox', { name: 'Search games', exact: true })).toHaveCount(0);
  await expect(page.getByText('Searching Twitch…', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Add Game', exact: true })).toBeDisabled();
  await search.fill('stardew');
  await expect(page.getByRole('option', { name: 'Stardew Valley', exact: true })).toBeVisible();
  finishSlow!();
  await expect(page.getByRole('option', { name: 'Old result', exact: true })).toHaveCount(0);
  await search.fill('fail');
  await expect(
    page.getByText('Twitch search is unavailable. You can still add a name.'),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add Game', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.locator('[data-game="fail"]')).toBeVisible();
  await search.fill('fail');
  await expect(
    page.getByText('Twitch search is unavailable. You can still add a name.'),
  ).toBeVisible();
  await page.unroute('**/api/games');
  await page.route('**/api/games', (route) => route.fulfill({ json: [stardew] }));
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await retry.focus();
  await retry.press('Enter');
  await expect(page.getByRole('option', { name: 'Stardew Valley', exact: true })).toBeVisible();
  await expect(search).toBeFocused();
});

test('early campaign selections get covers and Enter accepts Twitch word matches', async ({
  page,
  request,
}) => {
  let releaseSearch: (() => void) | undefined;
  const elder = { ...stardew, id: '65654', name: 'The Elder Scrolls Online' };
  const warcraft = { ...stardew, id: '18122', name: 'World of Warcraft' };
  await page.route('**/api/games', async (route) => {
    const query = route.request().postDataJSON();
    if (query.names)
      return route.fulfill({ json: query.names.includes(elder.name) ? [elder] : [] });
    if (query.search === 'elder') {
      await new Promise<void>((resolve) => {
        releaseSearch = resolve;
      });
      return route.fulfill({ json: [elder] });
    }
    return route.fulfill({ json: [warcraft] });
  });
  await page.goto('/?edit=priorities');
  const search = page.getByRole('combobox', { name: 'Search games' });
  await search.fill('elder');
  await expect.poll(() => !!releaseSearch).toBe(true);
  await expect(page.getByText('Searching Twitch…', { exact: true })).toHaveCount(0);
  await page.getByRole('option', { name: elder.name, exact: true }).click();
  releaseSearch!();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).game_metadata)
    .toEqual([elder]);
  await search.fill('world warcraft');
  await expect(page.getByRole('option', { name: warcraft.name, exact: true })).toBeVisible();
  await search.press('Enter');
  await expect(page.locator('[data-game="World of Warcraft"]')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
