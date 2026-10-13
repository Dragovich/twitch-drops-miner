import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import {
  mdiAlertCircleOutline,
  mdiCheck,
  mdiContentCopy,
  mdiPlayCircleOutline,
  mdiPriorityHigh,
  mdiRefresh,
  mdiUpdate,
} from '@mdi/js';
import fixture from './fixture.json' with { type: 'json' };
import type { Snapshot } from '../src/shared/lib/types';
const snapshot: Snapshot = {
  ...(fixture as Snapshot),
  settings: { ...fixture.settings, mining_priority_mode: 'manual' },
};
const headers = { 'X-TDM-Request': '1' };

for (const width of [1280, 320]) {
  test(`automatic claiming persists globally with accessible help at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?edit=priorities');
    const control = page.getByRole('checkbox', { name: 'Automatically claim drops', exact: true });
    const before = await (await request.get('/api/settings')).json();
    await expect(control).toBeChecked();
    await control.focus();
    await control.press('Space');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).auto_claim_drops)
      .toBe(false);
    await page.reload();
    await expect(control).not.toBeChecked();
    const after = await (await request.get('/api/settings')).json();
    expect(after.games_to_watch).toEqual(before.games_to_watch);
    expect(after.mining_benefits).toEqual(before.mining_benefits);
    expect(after.mining_paused).toBe(before.mining_paused);
    const help = page.getByRole('button', { name: 'Automatically claim drops help', exact: true });
    await help.focus();
    await help.press('Enter');
    const note = page.getByRole('note', { name: 'Automatically claim drops help', exact: true });
    await expect(note).toBeVisible();
    await expect(note).toContainText('claim earned drops yourself in your Twitch inventory');
    await expect(note).toContainText('Some rewards require earlier drops to be claimed.');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.keyboard.press('Escape');
    await expect(note).toBeHidden();
    if (width < 768) {
      expect((await control.locator('..').boundingBox())!.height).toBeGreaterThanOrEqual(44);
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.screenshot({ path: `../artifacts/auto-claim-${width}.png` });
    await control.check();
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).auto_claim_drops)
      .toBe(true);
  });
}

for (const width of [1280, 320]) {
  test(`mining priority uses a persistent icon selector without rewriting the manual order at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const initial = await (await request.get('/api/settings')).json();
    expect(
      (
        await request.post('/api/settings', {
          headers,
          data: { revision: initial.revision, games_to_watch: ['Rust', 'Other'] },
        })
      ).ok(),
    ).toBe(true);
    const before = await (await request.get('/api/settings')).json();
    await page.goto('/settings#mining');
    const priority = page.getByRole('combobox', { name: 'Mining priority', exact: true });
    const control = priority.locator('..');
    await expect(priority).toHaveValue('manual');
    await expect(priority.locator('option')).toHaveText([
      'Default (manual order)',
      'Short events first',
      'Ending soonest',
    ]);
    await expect(control.locator('path')).toHaveAttribute('d', mdiPriorityHigh);
    const box = (await control.boundingBox())!;
    expect([box.width, box.height]).toEqual(width < 768 ? [44, 44] : [36, 36]);
    const rows = page
      .getByRole('list', { name: 'Game priorities', exact: true })
      .getByRole('listitem');
    const order = await rows.allTextContents();
    await priority.focus();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(priority).toHaveValue('short_events');
    await expect(control).toHaveAttribute('title', 'Mining priority: Short events first');
    await expect(priority).toHaveAccessibleDescription(/24 hours or less/);
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).mining_priority_mode)
      .toBe('short_events');
    expect(await rows.allTextContents()).toEqual(order);
    await page.reload();
    await expect(priority).toHaveValue('short_events');
    await priority.selectOption('ending_soonest');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).mining_priority_mode)
      .toBe('ending_soonest');
    await expect(priority).toHaveAccessibleDescription(/nearest reward deadlines/);
    const after = await (await request.get('/api/settings')).json();
    expect(after.games_to_watch).toEqual(before.games_to_watch);
    expect(after.inventory_filters).toEqual(before.inventory_filters);
    expect([after.auto_mine_badges, after.auto_mine_emotes]).toEqual([
      before.auto_mine_badges,
      before.auto_mine_emotes,
    ]);
    await page.getByRole('button', { name: 'Reorder Rust', exact: true }).focus();
    await page.keyboard.press('ArrowDown');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
      .not.toEqual(before.games_to_watch);
    const reordered = await rows.allTextContents();
    await priority.selectOption('manual');
    await expect
      .poll(async () => (await (await request.get('/api/settings')).json()).mining_priority_mode)
      .toBe('manual');
    expect(await rows.allTextContents()).toEqual(reordered);
    const restoredBox = (await control.boundingBox())!;
    expect([restoredBox.width, restoredBox.height]).toEqual([box.width, box.height]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page
      .locator('#priorities')
      .screenshot({ path: `../artifacts/mining-priority-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test('mining priority retains rapid edits through conflicts, retry and reconnect', async ({
  page,
  request,
}) => {
  await page.goto('/settings#mining');
  const priority = page.getByRole('combobox', { name: 'Mining priority', exact: true });
  const autoClaim = page.getByRole('checkbox', { name: 'Automatically claim drops', exact: true });
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    '**/api/settings',
    async (route) => {
      await gate;
      await route.continue();
    },
    { times: 1 },
  );
  const sent = page.waitForRequest('**/api/settings');
  await priority.selectOption('short_events');
  await sent;
  await priority.selectOption('ending_soonest');
  await autoClaim.uncheck();
  const current = await (await request.get('/api/settings')).json();
  expect(
    (
      await request.post('/api/settings', {
        headers,
        data: { revision: current.revision, games_to_watch: ['Other', 'Rust'] },
      })
    ).ok(),
  ).toBe(true);
  release();
  await expect(page.getByRole('alert')).toContainText('Settings changed on another device');
  await expect(priority).toHaveValue('ending_soonest');
  await expect(autoClaim).not.toBeChecked();
  expect((await (await request.get('/api/settings')).json()).mining_priority_mode).toBe('manual');
  expect((await (await request.get('/api/settings')).json()).auto_claim_drops).toBe(true);
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).mining_priority_mode)
    .toBe('ending_soonest');
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).auto_claim_drops)
    .toBe(false);
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Other',
    'Rust',
  ]);
  await priority.selectOption('short_events');
  await request.post('/__test/reconnect', { headers, data: {} });
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).mining_priority_mode)
    .toBe('short_events');
  await page.goto('/settings#mining');
  await expect(priority).toHaveValue('short_events');
  await page.context().setOffline(true);
  try {
    await request.post('/__test/reconnect', { headers, data: {} });
    await expect(priority).toBeDisabled();
    await expect(autoClaim).toBeDisabled();
  } finally {
    await page.context().setOffline(false);
  }
  await expect(priority).toBeEnabled();
  await expect(priority).toHaveValue('short_events');
  await expect(autoClaim).toBeEnabled();
  await expect(autoClaim).not.toBeChecked();
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Other',
    'Rust',
  ]);
});

test('automatic reward types persist without changing games or display filters', async ({
  page,
  request,
}) => {
  await page.goto('/settings#mining');
  const before = await (await request.get('/api/settings')).json();
  const badges = page
    .getByRole('group', { name: 'Also mine from other games', exact: true })
    .getByRole('checkbox', { name: 'Badges', exact: true });
  const emotes = page
    .getByRole('group', { name: 'Also mine from other games', exact: true })
    .getByRole('checkbox', { name: 'Emotes', exact: true });
  await expect(badges).not.toBeChecked();
  await expect(emotes).not.toBeChecked();
  await badges.check();
  await emotes.check();
  await expect
    .poll(async () => {
      const settings = await (await request.get('/api/settings')).json();
      return [settings.auto_mine_badges, settings.auto_mine_emotes];
    })
    .toEqual([true, true]);
  await page.reload();
  await expect(badges).toBeChecked();
  await expect(emotes).toBeChecked();
  const after = await (await request.get('/api/settings')).json();
  expect(after.games_to_watch).toEqual(before.games_to_watch);
  expect(after.inventory_filters).toEqual(before.inventory_filters);
  await badges.uncheck();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).auto_mine_badges)
    .toBe(false);
  await expect(emotes).toBeChecked();
});

for (const width of [1280, 320]) {
  test(`refresh button tracks completion, failures, stale events and reconnects without notices at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const refresh = page.getByRole('button', { name: 'Refresh inventory', exact: true });
    const control = page.getByRole('button', { name: /^Refresh/ });
    const size = async () => {
      const box = (await control.boundingBox())!;
      return [box.width, box.height];
    };
    const expectedSize = width < 768 ? [44, 44] : [36, 36];
    await expect(refresh).toHaveText('');
    await expect(refresh.locator('path')).toHaveAttribute('d', mdiRefresh);
    await expect(refresh).toHaveAttribute('title', /^Refresh inventory/);
    expect(await size()).toEqual(expectedSize);
    await page.evaluate(() => document.fonts.ready);
    const iconCenter = () =>
      page
        .getByRole('button', { name: /^Refresh/ })
        .locator('svg')
        .evaluate((icon) => {
          const box = icon.getBoundingClientRect();
          return box.x + box.width / 2;
        });
    const idleIconCenter = await iconCenter();
    const refreshIcon = await refresh.locator('svg').boundingBox();
    expect(refreshIcon!.width).toBe(18);
    expect(refreshIcon!.height).toBe(18);
    await refresh.click();
    await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
    await expect(control).toHaveAttribute('aria-busy', 'true');
    await expect(control).toHaveAttribute('title', /^Refreshing\.\.\./);
    expect(await size()).toEqual(expectedSize);
    expect(await control.locator('svg').evaluate((el) => getComputedStyle(el).animationName)).toBe(
      'spin',
    );
    await page.emulateMedia({ reducedMotion: 'reduce' });
    expect(await control.locator('svg').evaluate((el) => getComputedStyle(el).animationName)).toBe(
      'none',
    );
    expect(await iconCenter()).toBeCloseTo(idleIconCenter, 1);
    await page.reload();
    await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
    await page.goto('/campaigns');
    await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
    expect(await size()).toEqual(expectedSize);
    await page.goto('/');
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_refresh', data: { sequence: 2, state: 'refreshed', error: null } },
    });
    await expect(page.getByRole('button', { name: 'Refreshed', exact: true })).toBeEnabled();
    await expect(control.locator('path')).toHaveAttribute('d', mdiCheck);
    await expect(control).toHaveAttribute('aria-busy', 'false');
    await expect(page.locator('main [aria-live="polite"]')).toContainText('Refreshed');
    expect(await size()).toEqual(expectedSize);
    expect(await iconCenter()).toBeCloseTo(idleIconCenter, 1);
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_refresh', data: { sequence: 1, state: 'refreshing', error: null } },
    });
    await expect(page.getByRole('button', { name: 'Refreshed', exact: true })).toBeEnabled();
    await expect(page.getByText('Inventory refresh requested.', { exact: true })).toHaveCount(0);
    await expect(refresh).toBeVisible({ timeout: 6000 });
    await refresh.click();
    await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
    await request.post('/__test/event', {
      headers,
      data: {
        event: 'inventory_refresh',
        data: {
          sequence: 4,
          state: 'failed',
          error: 'The public catalog is unavailable or incomplete.',
        },
      },
    });
    const retry = page.getByRole('button', { name: 'Refresh failed - Retry', exact: true });
    await expect(retry).toBeEnabled();
    expect(await iconCenter()).toBeCloseTo(idleIconCenter, 1);
    await expect(retry.locator('path')).toHaveAttribute('d', mdiAlertCircleOutline);
    expect(await size()).toEqual(expectedSize);
    await expect(retry).toHaveAttribute(
      'title',
      'Refresh failed - Retry\nThe public catalog is unavailable or incomplete.',
    );
    await expect(retry).toHaveAccessibleDescription(
      'The public catalog is unavailable or incomplete.',
    );
    await expect(page.getByRole('alert')).toHaveCount(0);
    await page.screenshot({ path: `../artifacts/refresh-button-failure-${width}.png` });
    expect((await new AxeBuilder({ page }).include('main').analyze()).violations).toEqual([]);
    await retry.click();
    await expect(page.getByRole('button', { name: 'Refreshing...', exact: true })).toBeDisabled();
  });
}

test('refresh request errors stay in the button and require a connected Twitch account', async ({
  page,
  request,
}) => {
  await page.route('**/api/reload', (route) =>
    route.fulfill({ status: 503, json: { detail: 'request_failed' } }),
  );
  await page.getByRole('button', { name: 'Refresh inventory', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Refresh failed - Retry', exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await request.post('/__test/event', {
    headers,
    data: { event: 'login_status', data: { status: 'Logged out', user_id: null } },
  });
  await expect(
    page.getByRole('button', { name: 'Refresh failed - Retry', exact: true }),
  ).toBeDisabled();
});

test('Maintenance shows a release notice and notes link without installing anything', async ({
  page,
}) => {
  const writes: string[] = [];
  page.on('request', (request) => {
    if (request.method() === 'POST' && request.url().includes('/api/')) writes.push(request.url());
  });
  await page.route('**/api/version', (route) =>
    route.fulfill({
      json: {
        current_version: '0.1.0',
        latest_version: '0.2.0',
        update_available: true,
        check_succeeded: true,
        download_url: 'https://github.com/ohne-b/twitch-drops-miner/releases/tag/v0.2.0',
      },
    }),
  );
  await page.goto('/settings#maintenance');
  const maintenance = page.locator('#maintenance');
  await expect(maintenance.getByText('Version 0.2.0 is available.')).toBeVisible();
  await expect(maintenance.getByTitle('Installed version: 0.1.0')).toHaveText('v0.1.0');
  await expect(maintenance.locator('summary')).toHaveText('Advanced actions');
  await expect(maintenance).not.toContainText('This application automatically mines');
  await expect(maintenance.getByRole('link', { name: 'Release notes' })).toHaveAttribute(
    'href',
    'https://github.com/ohne-b/twitch-drops-miner/releases/tag/v0.2.0',
  );
  await expect(maintenance.getByRole('button', { name: /^(Install|Update now)/ })).toHaveCount(0);
  await maintenance.getByRole('button', { name: 'Check for updates' }).click();
  await expect(maintenance.getByText('Version 0.2.0 is available.')).toBeVisible();
  expect(writes).toEqual([]);
  await page.screenshot({ path: '../artifacts/server-updates-available.png' });
  await page.setViewportSize({ width: 375, height: 720 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  expect((await new AxeBuilder({ page }).include('#maintenance').analyze()).violations).toEqual([]);
});

test('failed release checks stay distinct from up-to-date and can be retried', async ({ page }) => {
  let successful = false;
  await page.route('**/api/version', (route) =>
    route.fulfill({
      json: {
        current_version: '0.1.0',
        latest_version: successful ? '0.1.0' : null,
        update_available: false,
        check_succeeded: successful,
        download_url: 'https://github.com/ohne-b/twitch-drops-miner/releases',
      },
    }),
  );
  await page.goto('/settings#maintenance');
  const maintenance = page.locator('#maintenance');
  await expect(maintenance.getByText('Could not check for updates. Try again.')).toBeVisible();
  await expect(maintenance.getByText("You're up to date.")).toHaveCount(0);
  successful = true;
  await maintenance.getByRole('button', { name: 'Check for updates' }).click();
  await expect(maintenance.getByText("You're up to date.")).toBeVisible();
  await expect(maintenance.getByRole('link', { name: 'Release notes' })).toHaveCount(0);
  await page.screenshot({ path: '../artifacts/server-updates-current.png' });
});

for (const width of [1280, 320]) {
  test(`compact update, account and manual controls at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
    const mine = page.getByRole('button', { name: 'Mine', exact: true });
    await expect(mine).toHaveText('');
    await expect(mine).toHaveAttribute('title', 'Mine');
    await expect(mine).toHaveAttribute('type', 'submit');
    await expect(mine.locator('path')).toHaveAttribute('d', mdiPlayCircleOutline);
    await expect(mine).toBeDisabled();
    await page.getByRole('combobox', { name: 'Twitch channel name or URL' }).fill('extra_streamer');
    await expect(mine).toBeEnabled();
    await mine.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByText('Watching extra_streamer', { exact: true })).toBeVisible();

    let checks = 0;
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route('**/api/version', async (route) => {
      if (++checks > 1) await gate;
      await route.fulfill({
        json: {
          current_version: '1.2.0',
          latest_version: '1.2.0',
          update_available: false,
          check_succeeded: true,
          download_url: null,
        },
      });
    });
    await page.goto('/settings');
    const account = page.locator('#account');
    const status = account.locator('.account-identity');
    const logout = account.getByRole('button', { name: 'Log out of Twitch', exact: true });
    await expect(logout).toBeEnabled();
    const h = (await status.boundingBox())!;
    const l = (await logout.boundingBox())!;
    expect(l.x).toBeGreaterThanOrEqual(h.x + h.width);
    expect(l.x - (h.x + h.width)).toBeLessThanOrEqual(12);
    expect(l.y + l.height / 2).toBeCloseTo(h.y + h.height / 2, 0);
    await expect(account.getByText('Twitch ID: 123456', { exact: true })).toHaveCount(0);
    await account.screenshot({ path: `../artifacts/account-controls-${width}.png` });

    await page.getByRole('link', { name: 'Maintenance', exact: true }).click();
    const update = page.getByRole('button', { name: 'Check for updates', exact: true });
    await expect(update).toBeEnabled();
    await expect(update).toHaveText('');
    await expect(update.locator('path')).toHaveAttribute('d', mdiUpdate);
    await update.scrollIntoViewIfNeeded();
    const before = (await update.boundingBox())!;
    expect(before.width).toBe(width < 768 ? 44 : 36);
    await update.click();
    const checking = page.getByRole('button', { name: 'Checking for updates…', exact: true });
    await expect(checking).toBeDisabled();
    await expect(checking).toHaveAttribute('aria-busy', 'true');
    await expect(checking).toHaveAttribute('title', 'Checking for updates…');
    expect((await checking.boundingBox())!.width).toBe(before.width);
    expect((await checking.boundingBox())!.height).toBe(before.height);
    expect(await checking.locator('svg').evaluate((el) => getComputedStyle(el).animationName)).toBe(
      'spin',
    );
    await page.emulateMedia({ reducedMotion: 'reduce' });
    expect(await checking.locator('svg').evaluate((el) => getComputedStyle(el).animationName)).toBe(
      'none',
    );
    await expect(page.locator('#maintenance').getByRole('status')).toHaveText(
      'Checking for updates…',
    );
    release();
    await expect(update).toBeEnabled();
    await expect(update).toHaveAttribute('aria-busy', 'false');
    await expect(page.getByText("You're up to date.", { exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    expect(
      (await new AxeBuilder({ page }).include('#account').include('#maintenance').analyze())
        .violations,
    ).toEqual([]);
  });
}

test('retired notifications are absent from the dashboard and API', async ({ page, request }) => {
  await page.goto('/settings');
  await expect(page.getByRole('heading', { name: 'Telegram Notifications' })).toHaveCount(0);
  expect((await request.post('/api/settings/test-telegram', { headers, data: {} })).status()).toBe(
    404,
  );
  expect(JSON.stringify(await (await request.get('/api/settings')).json())).not.toContain(
    'telegram',
  );
});
test.beforeEach(async ({ request, page }) => {
  const reset = await request.post('/__test/reset', { headers, data: {} });
  expect(reset.ok()).toBe(true);
  expect(await reset.json()).toEqual({ ok: true });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
});
test('dashboard policy allows its WebSocket connection', async ({ page }) => {
  const opened = await page.evaluate(
    () =>
      new Promise<boolean>((resolve) => {
        const url = new URL('/socket.io/?EIO=4&transport=websocket&protocol=2', location.href);
        url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const socket = new WebSocket(url);
        const finish = (ok: boolean) => {
          clearTimeout(timer);
          socket.close();
          resolve(ok);
        };
        const timer = setTimeout(() => finish(false), 5000);
        socket.onopen = () => finish(true);
        socket.onerror = () => finish(false);
      }),
  );
  expect(opened).toBe(true);
});

test('shared logo loads in the dashboard, login and favicon at responsive sizes', async ({
  page,
}) => {
  const brand = page.getByRole('link', { name: 'Drops Miner', exact: true });
  await expect(page).toHaveTitle('70% Rust - Drops Miner');
  await expect(page.getByRole('link', { name: 'GitHub repository' })).toHaveAttribute(
    'href',
    'https://github.com/ohne-b/twitch-drops-miner',
  );
  const logo = brand.locator('img');
  expect(await brand.evaluate((element) => getComputedStyle(element).fontSize)).toBe('16px');
  const source = await logo.getAttribute('src');
  expect(source).toMatch(/^\/assets\/twitch-drops-miner-logo-[\w-]+\.svg$/);
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', source!);
  await expect
    .poll(() => logo.evaluate((img: HTMLImageElement) => img.naturalWidth))
    .toBeGreaterThan(0);
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 1440, height: 300 },
    { width: 320, height: 640 },
  ]) {
    await page.setViewportSize(viewport);
    await brand.scrollIntoViewIfNeeded();
    const header = (await brand.locator('..').boundingBox())!;
    const mark = (await logo.boundingBox())!;
    expect(header.height).toBe(viewport.width < 1024 ? 60 : 80);
    expect(mark.y).toBeGreaterThanOrEqual(header.y);
    expect(mark.y + mark.height).toBeLessThanOrEqual(header.y + header.height);
    const title = (await brand.boundingBox())!;
    expect(title.x + title.width).toBeLessThanOrEqual(header.x + header.width);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    if (viewport.height === 300) {
      const settings = page.getByRole('link', { name: 'Settings', exact: true });
      await settings.scrollIntoViewIfNeeded();
      await expect(settings).toBeInViewport();
      const github = page.getByRole('link', { name: 'GitHub repository' });
      await github.scrollIntoViewIfNeeded();
      await expect(github).toBeInViewport();
    }
    await page.screenshot({ path: `../artifacts/logo-${viewport.width}x${viewport.height}.png` });
  }
  await page.route('**/api/auth/status', (route) =>
    route.fulfill({ json: { enabled: true, authenticated: false } }),
  );
  await page.goto('/login');
  await expect(page.getByRole('heading', { name: 'Unlock dashboard' })).toBeVisible();
  const loginLogo = page.locator('main img');
  await expect(loginLogo).toHaveAttribute('src', source!);
  await expect
    .poll(() => loginLogo.evaluate((img: HTMLImageElement) => img.naturalWidth))
    .toBeGreaterThan(0);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: '../artifacts/logo-login.png', fullPage: true });
});

test('confirmed progress and compact desktop design', async ({ page }) => {
  await expect(page.getByText('42 / 60 min', { exact: true })).toBeVisible();
  await expect(page.getByText('Watching: northwind', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Watching northwind', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Recent activity', exact: true })).toHaveCount(0);
  await expect(page.getByRole('progressbar', { name: 'Explorer jacket' })).toHaveAttribute(
    'aria-valuenow',
    '42',
  );
  await expect(page.getByText('48 / 60 min')).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(
    page.locator('aside').getByRole('link', { name: 'Twitch account', exact: true }),
  ).toBeVisible();
  const github = page.getByRole('link', { name: 'GitHub repository' }).locator('svg path');
  const githubBox = (await github.boundingBox())!;
  expect(githubBox.width).toBeCloseTo(32, 1);
  const accountBox = (await page
    .locator('aside')
    .getByRole('link', { name: 'Twitch account', exact: true })
    .boundingBox())!;
  expect(githubBox.y + githubBox.height).toBeLessThan(accountBox.y);
  const channels = page
    .locator('section')
    .filter({ has: page.getByRole('heading', { name: 'Channels', exact: true }) });
  const queue = page
    .locator('section')
    .filter({ has: page.getByRole('heading', { name: 'Up next', exact: true }) });
  expect((await channels.boundingBox())?.width).toBe((await queue.boundingBox())?.width);
  expect((await channels.boundingBox())?.height).toBe((await queue.boundingBox())?.height);
  await expect(page.getByText('Confirmed by Twitch', { exact: true })).toHaveCount(0);
  await page.screenshot({ path: '../artifacts/redesign-desktop.png', fullPage: true });
  expect(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme)).toBe(
    'dark',
  );
  expect(
    await page
      .locator('body')
      .evaluate((element) => getComputedStyle(element, '::-webkit-scrollbar').width),
  ).toBe('3px');
});
test('mining confirmation stays beside selection mode inside Now mining', async ({
  page,
  request,
}) => {
  const header = page.getByRole('heading', { name: 'Mining', exact: true }).locator('..');
  const mining = page.getByRole('region', { name: 'Now mining', exact: true });
  const confirmed = mining.locator('time');
  const mode = mining.getByRole('img', { name: 'Automatic selection', exact: true });
  const refresh = header.getByRole('button', { name: 'Refresh inventory', exact: true });
  for (const width of [1440, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(confirmed).toContainText('Confirmed ');
    await expect(confirmed).toHaveAttribute('datetime', snapshot.current_drop!.confirmed_at!);
    await expect(confirmed).toHaveCSS('color', 'rgb(136, 136, 136)');
    await expect(confirmed).toHaveCSS('font-size', '12px');
    const timeBox = (await confirmed.boundingBox())!;
    const modeBox = (await mode.boundingBox())!;
    expect(timeBox.x + timeBox.width).toBeLessThanOrEqual(modeBox.x);
    expect(timeBox.y + timeBox.height / 2).toBeCloseTo(modeBox.y + modeBox.height / 2, 0);
    await expect(header.locator('time')).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/mining-confirmation-${width}.png` });
  }
  await request.post('/__test/event', {
    headers,
    data: { event: 'drop_progress', data: { ...snapshot.current_drop, confirmed_at: null } },
  });
  await expect(confirmed).toHaveCount(0);
  await expect(refresh).toBeVisible();
});
for (const width of [1440, 320]) {
  test(`Now mining keeps artwork, wrapped details and manual controls aligned at ${width}px`, async ({
    page,
    request,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const card = page.getByRole('region', { name: 'Now mining', exact: true });
    const reward = card.locator('#mining-drop-details');
    const identity = reward.locator('../..');
    const artwork = identity.locator(':scope > span');
    const text = reward.locator('..');
    const watching = snapshot.channels.find((channel) => channel.watching)!;
    const streamerLink = card.getByRole('link', { name: watching.name, exact: true });
    await expect(streamerLink).toHaveText(watching.name);
    await expect(streamerLink).toHaveAttribute('href', 'https://www.twitch.tv/northwind');
    for (const title of ['Explorer jacket', 'Explorer jacket and companion reward bundle']) {
      expect(
        (
          await request.post('/__test/event', {
            headers,
            data: { event: 'drop_progress', data: { ...snapshot.current_drop, drop_name: title } },
          })
        ).ok(),
      ).toBe(true);
      await expect(reward).toHaveText(title);
      const artBox = (await artwork.boundingBox())!;
      const textBox = (await text.boundingBox())!;
      expect([artBox.width, artBox.height]).toEqual([88, 88]);
      await expect(text.locator('img, .rounded-full')).toHaveCount(0);
      expect(artBox.y + artBox.height / 2).toBeCloseTo(textBox.y + textBox.height / 2, 0);
      expect(artBox.x + artBox.width).toBeLessThan(textBox.x);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
    }
    await card.screenshot({ path: `../artifacts/now-mining-reward-${width}.png` });
    expect(
      (
        await request.post('/__test/event', {
          headers,
          data: {
            event: 'manual_mode_update',
            data: { ...snapshot.manual_mode, active: true, expires_at: '2026-10-05T01:00:00Z' },
          },
        })
      ).ok(),
    ).toBe(true);
    const back = card.getByRole('button', { name: 'Return to Auto Mode', exact: true });
    const timer = back.locator('..').getByText(/^Auto mode at /);
    await expect(timer).toBeVisible();
    const buttonBox = (await back.boundingBox())!;
    const timerBox = (await timer.boundingBox())!;
    expect(timerBox.x + timerBox.width).toBeLessThan(buttonBox.x);
    const header = card.locator(':scope > div').first();
    await expect(header.getByText('Manual selection', { exact: true })).toBeVisible();
    await expect(header.getByRole('button', { name: 'Return to Auto Mode' })).toBeVisible();
    expect(timerBox.y + timerBox.height).toBeLessThan((await identity.boundingBox())!.y);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    if (width < 768) expect(buttonBox.height).toBeGreaterThanOrEqual(44);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await card.screenshot({ path: `../artifacts/now-mining-manual-${width}.png` });
    await back.click();
    await expect(back).toHaveCount(0);
    await expect(card.getByRole('img', { name: 'Automatic selection', exact: true })).toBeVisible();
    expect(
      (
        await request.post('/__test/event', {
          headers,
          data: {
            event: 'initial_state',
            data: {
              ...snapshot,
              current_drop: null,
              channels: [],
              settings: { ...snapshot.settings, games_to_watch: [] },
              mining: { ...snapshot.mining, state: 'no_selection' },
            },
          },
        })
      ).ok(),
    ).toBe(true);
    await expect(card.getByRole('link', { name: 'Choose games', exact: true })).toBeVisible();
    await expect(card.getByRole('link', { name: 'Choose games', exact: true })).toBeInViewport();
    await expect(card.getByRole('progressbar')).toHaveCount(0);
    await card.screenshot({ path: `../artifacts/now-mining-empty-${width}.png` });
  });
}

test('unconfirmed rewards show minute counts without inventing confirmation', async ({
  page,
  request,
}) => {
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: {
          event: 'initial_state',
          data: {
            ...snapshot,
            current_drop: { ...snapshot.current_drop, confirmed_at: null },
            mining: { ...snapshot.mining, state: 'awaiting_progress' },
            campaigns: snapshot.campaigns.map((campaign) => ({
              ...campaign,
              drops: campaign.drops.map((drop) => ({ ...drop, confirmed_at: null })),
            })),
          },
        },
      })
    ).ok(),
  ).toBe(true);
  const mining = page.getByRole('region', { name: 'Now mining', exact: true });
  const count = mining.getByText('0 / 60 min', { exact: true });
  await expect(count).toBeVisible();
  await expect(count).toHaveAttribute('title', 'No confirmed progress yet');
  await expect(count).toHaveAccessibleDescription('No confirmed progress yet');
  await expect(mining.getByText('No confirmed progress yet', { exact: true })).toHaveCount(0);
  await expect(mining.getByRole('progressbar')).toBeVisible();
  await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
  await expect(mining.getByRole('progressbar')).toHaveAccessibleDescription(
    'No confirmed progress yet',
  );
  await expect(mining.getByRole('progressbar').locator('div')).toHaveCSS('width', '0px');
  await expect(page.getByText(/Confirmed /)).toHaveCount(0);
  await expect(page.getByText(/Waiting for Twitch progress/)).toHaveCount(0);

  await page.goto('/campaigns?campaign=campaign-1');
  const details = page.getByRole('complementary', { name: 'Campaign details' });
  await expect(details.getByText('0 / 60 min', { exact: true })).toBeVisible();
  await expect(details.getByText('0 / 120 min', { exact: true })).toBeVisible();
  await expect(details.getByText('0 / 60 min', { exact: true })).toHaveAccessibleDescription(
    'No confirmed progress yet',
  );
  await expect(details.getByText('Waiting for prerequisite claims', { exact: true })).toBeVisible();
  await expect(details.getByRole('progressbar')).toHaveCount(2);
  for (const bar of await details.getByRole('progressbar').all()) {
    await expect(bar).toHaveAttribute('aria-valuenow', '0');
    await expect(bar).toHaveAccessibleDescription('No confirmed progress yet');
    await expect(bar.locator('div')).toHaveCSS('width', '0px');
  }
  await expect(details.getByText(/Confirmed |Waiting for Twitch progress/)).toHaveCount(0);

  await request.post('/__test/event', {
    headers,
    data: {
      event: 'drop_progress',
      data: { ...snapshot.current_drop, confirmed_minutes: 0 },
    },
  });
  await page.goto('/');
  await expect(count).toBeVisible();
  await expect(count).not.toHaveAttribute('title');
  await expect(mining.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '0');
  await expect(page.getByText(/Confirmed /)).toBeVisible();
  await request.post('/__test/event', {
    headers,
    data: { event: 'drop_progress_stop', data: {} },
  });
  await expect(mining.getByText('No confirmed progress yet', { exact: true })).toBeVisible();
  await expect(mining.getByText(/\d+ \/ \d+ min/)).toHaveCount(0);
  await expect(mining.getByRole('progressbar')).toHaveCount(0);
  await expect(page.getByText(/Confirmed /)).toHaveCount(0);
});
test('Up next scrolls within its panel with reward artwork and safe fallbacks', async ({
  page,
  request,
}) => {
  const png =
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
  await page.route('https://art.example/**', (route) =>
    route.request().url().endsWith('broken.png')
      ? route.abort()
      : route.fulfill({ contentType: 'image/png', body: Buffer.from(png, 'base64') }),
  );
  const game = snapshot.wanted_items[0]!;
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'channels_batch_update',
      data: {
        channels: Array.from({ length: 30 }, (_, i) => ({
          ...snapshot.channels[0]!,
          id: i + 1,
          name: `channel-${i}`,
          watching: i === 0,
        })),
      },
    },
  });
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'wanted_items_update',
      data: [
        {
          ...game,
          campaigns: [
            {
              ...game.campaigns[0],
              drops: Array.from({ length: 30 }, (_, i) => ({
                name: `Reward ${i}`,
                benefits: [`Reward ${i}`],
                image_url:
                  i === 0
                    ? 'https://art.example/reward.png'
                    : i === 1
                      ? 'https://art.example/broken.png'
                      : '',
              })),
            },
          ],
        },
      ],
    },
  });
  const panel = page.getByRole('region', { name: 'Up next', exact: true });
  const channelPanel = page.getByRole('region', { name: 'Channels', exact: true });
  const channels = page.locator('section').filter({ has: channelPanel });
  const queue = page.locator('section').filter({ has: panel });
  await expect(panel.locator('li')).toHaveCount(30);
  await expect(panel.locator('li').first().locator('img')).toHaveAttribute(
    'src',
    'https://art.example/reward.png',
  );
  await expect(panel.locator('li').nth(1).locator('svg')).toBeVisible();
  await expect(panel.locator('li').nth(2).locator('svg')).toBeVisible();
  for (const [width, height] of [
    [1440, 900],
    [1598, 520],
    [390, 900],
  ] as const) {
    await page.setViewportSize({ width, height });
    if (width >= 1280) {
      expect((await channels.boundingBox())!.height).toBe((await queue.boundingBox())!.height);
      expect((await channels.boundingBox())!.y).toBe((await queue.boundingBox())!.y);
    } else {
      expect((await channels.boundingBox())!.y).toBeGreaterThan((await queue.boundingBox())!.y);
    }
    for (const region of [panel, channelPanel]) {
      expect((await region.boundingBox())!.height).toBeLessThanOrEqual(
        width >= 1280 ? height : 440,
      );
      expect(
        await region.evaluate(
          (el) => el.scrollHeight > el.clientHeight && getComputedStyle(el).overflowY === 'auto',
        ),
      ).toBe(true);
      await region.focus();
      await page.keyboard.press('End');
      await expect.poll(() => region.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  for (const region of [panel, channelPanel])
    await region.evaluate((el) => {
      el.scrollTop = 0;
    });
  await page.screenshot({ path: '../artifacts/overview-queue.png', fullPage: true });
  await page.getByRole('searchbox', { name: 'Search channels' }).fill('no matching channel');
  await expect(channelPanel.getByText('No matching results')).toBeVisible();
  await request.post('/__test/event', {
    headers,
    data: { event: 'wanted_items_update', data: [] },
  });
  await expect(panel.getByText('No wanted drops queued...')).toBeVisible();
  expect((await channels.boundingBox())!.height).toBe((await queue.boundingBox())!.height);
});

test('Overview fits the desktop viewport and only scrolls the page when space is limited', async ({
  page,
}) => {
  for (const [width, height] of [
    [1920, 945],
    [1440, 900],
    [1280, 720],
  ]) {
    await page.setViewportSize({ width: width!, height: height! });
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeLessThanOrEqual(
      height!,
    );
    const channels = page.getByRole('region', { name: 'Channels', exact: true });
    const queue = page.getByRole('region', { name: 'Up next', exact: true });
    expect((await channels.boundingBox())!.height).toBeGreaterThan(100);
    expect((await queue.boundingBox())!.height).toBeGreaterThan(100);
  }
  await page.setViewportSize({ width: 1440, height: 480 });
  expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBeGreaterThan(480);
  await page.getByRole('region', { name: 'Channels', exact: true }).scrollIntoViewIfNeeded();
  await expect(page.getByRole('button', { name: 'Watch harbor', exact: true })).toBeVisible();
  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
  expect((await page.locator('aside').boundingBox())!.y).toBeCloseTo(0, 0);
  await expect(page.getByRole('navigation').getByRole('link', { name: 'Mining' })).toBeInViewport();
  await expect(
    page.locator('aside').getByRole('link', { name: 'Twitch account', exact: true }),
  ).toBeInViewport();
  await page.screenshot({ path: '../artifacts/overview-short-window.png', fullPage: true });
});
test('every route loads directly and stays usable on a phone', async ({ page }) => {
  await page.goto('/settings#connection');
  const proxy = await page.getByLabel('Proxy URL', { exact: true }).boundingBox();
  const quality = await page.getByLabel('Connection Quality:', { exact: true }).boundingBox();
  expect(proxy!.y).toBeCloseTo(quality!.y, 0);
  await page.screenshot({ path: '../artifacts/settings-desktop.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  for (const [route, title] of [
    ['/campaigns', 'Campaigns'],
    ['/history', 'Campaigns'],
    ['/activity', 'Activity'],
    ['/settings', 'Settings'],
  ]) {
    await page.goto(route!);
    await expect(page.getByRole('heading', { name: title!, exact: true })).toBeVisible();
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
    ).toBe(true);
  }
  await page.screenshot({ path: '../artifacts/redesign-settings-mobile.png', fullPage: true });
});

test('keyboard focus remains visible without outlines across controls', async ({ page }) => {
  await page.goto('/settings#connection');
  await page.keyboard.press('Tab');
  const field = page.getByLabel('Proxy URL', { exact: true });
  const before = await field.evaluate((element) => getComputedStyle(element).borderColor);
  await field.focus();
  expect(await field.evaluate((element) => getComputedStyle(element).borderColor)).not.toBe(before);
  expect(await field.evaluate((element) => getComputedStyle(element).outlineStyle)).toBe('none');
  for (const name of ['Log out of Twitch', 'Enable password protection']) {
    await page
      .getByRole('navigation', { name: 'Settings sections' })
      .getByRole('link', {
        name: name === 'Log out of Twitch' ? 'Twitch account' : 'Dashboard access',
        exact: true,
      })
      .click();
    await page.keyboard.press('Tab');
    const button = page.getByRole('button', { name, exact: true });
    await button.focus();
    await expect
      .poll(() => button.evaluate((element) => getComputedStyle(element).backgroundColor))
      .toBe('rgb(51, 51, 51)');
  }
  await page.goto('/?edit=priorities');
  const checkbox = page
    .getByRole('group', { name: 'Allowed reward types', exact: true })
    .getByRole('checkbox', { name: 'Badges', exact: true });
  await checkbox.focus();
  expect(
    await checkbox.evaluate(
      (element) => getComputedStyle(element.closest('label')!).backgroundColor,
    ),
  ).toBe('rgb(51, 51, 51)');
  await page.emulateMedia({ forcedColors: 'active' });
  expect(await checkbox.evaluate((element) => getComputedStyle(element).outlineStyle)).toBe(
    'solid',
  );
});
test('channel search, clear, selection and automatic mode', async ({ page }) => {
  const channels = page.getByRole('region', { name: 'Channels', exact: true });
  await page.getByRole('searchbox', { name: 'Search channels' }).fill('HARBOR');
  await expect(channels.getByRole('link', { name: 'northwind', exact: true })).toHaveCount(0);
  await expect(
    page
      .getByRole('region', { name: 'Now mining', exact: true })
      .getByRole('link', { name: 'northwind', exact: true }),
  ).toBeVisible();
  const watch = page.getByRole('button', { name: 'Watch harbor' });
  await expect(watch).toHaveText('');
  await expect(watch).toHaveAttribute('title', 'Watch harbor');
  await watch.click();
  await expect(page.getByText('Manual selection')).toBeVisible();
  const automatic = page.getByRole('button', { name: 'Return to Auto Mode' });
  await expect(automatic).toHaveText('');
  await expect(automatic).toHaveAttribute('title', 'Return to Auto Mode');
  await page.screenshot({ path: '../artifacts/overview-action-icons.png', fullPage: true });
  await automatic.click();
  await expect(page.getByRole('img', { name: 'Automatic selection' })).toBeVisible();
  await page.getByRole('button', { name: 'Clear search' }).click();
  await expect(channels.getByRole('link', { name: 'northwind', exact: true })).toBeVisible();
});

test('offline channels with unknown viewers do not crash Overview', async ({ page, request }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/?edit=priorities');
  await expect(page.getByRole('button', { name: 'Reorder Rust', exact: true })).toBeEnabled();
  const offline = {
    ...snapshot.channels[0]!,
    name: 'offline-channel',
    viewers: null,
    online: false,
    watching: false,
    game: null,
  };
  await request.post('/__test/event', {
    headers,
    data: { event: 'channel_update', data: offline },
  });
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  const row = page
    .locator('.row')
    .filter({ has: page.getByRole('link', { name: 'offline-channel', exact: true }) });
  await expect(row).toContainText('Offline');
  await expect(row.getByRole('button', { name: 'Watch offline-channel' })).toBeEnabled();
  await request.post('/__test/event', {
    headers,
    data: { event: 'channels_batch_update', data: { channels: [offline] } },
  });
  await expect(
    page.getByRole('region', { name: 'Channels', exact: true }).locator('.row'),
  ).toHaveCount(1);
  await expect(page.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});
test('campaign filtering and truthful expanded progress', async ({ page }) => {
  await page.goto('/campaigns');
  await page.getByText('Autumn expedition', { exact: true }).click();
  await expect(page.getByText('42 / 60 min')).toBeVisible();
  await page.getByRole('button', { name: 'Close details' }).click();
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByLabel('Game items', { exact: true }).uncheck();
  await expect(page.getByText('No matching results')).toBeVisible();
  await page.getByLabel('Game items', { exact: true }).check();
  await expect(page.getByText('Autumn expedition', { exact: true })).toBeVisible();
});

test('campaign sort controls preserve filters and keep counts and resets in compact positions', async ({
  page,
  request,
}) => {
  const original = snapshot.campaigns[0]!;
  const campaigns = [
    {
      ...original,
      id: 'zeta',
      name: 'Zeta campaign',
      starts_at: '2026-09-25T00:00:00Z',
      ends_at: '2026-10-10T00:00:00Z',
      total_drops: 2,
    },
    {
      ...original,
      id: 'alpha',
      name: 'Alpha campaign',
      starts_at: '2026-09-26T00:00:00Z',
      ends_at: '2026-10-09T00:00:00Z',
      total_drops: 5,
    },
    {
      ...original,
      id: 'beta',
      name: 'beta campaign',
      starts_at: '2026-09-27T00:00:00Z',
      ends_at: '2026-10-11T00:00:00Z',
      total_drops: 3,
    },
  ];
  await page.goto('/campaigns');
  const sort = page.getByRole('combobox', { name: 'Sort campaigns' });
  const publishCampaigns = async () => {
    await expect(sort).toBeVisible();
    await request.post('/__test/event', {
      headers,
      data: { event: 'inventory_batch_update', data: { campaigns } },
    });
  };
  await publishCampaigns();
  const titles = page.locator('main .campaign-open > span > span.font-medium');
  const writes: string[] = [];
  page.on('request', (request) => {
    if (request.url().endsWith('/api/settings') && request.method() !== 'GET')
      writes.push(request.method());
  });
  for (const [value, names] of [
    ['newest', ['beta campaign', 'Alpha campaign', 'Zeta campaign']],
    ['ending', ['Alpha campaign', 'Zeta campaign', 'beta campaign']],
    ['drops', ['Alpha campaign', 'beta campaign', 'Zeta campaign']],
    ['name', ['Alpha campaign', 'beta campaign', 'Zeta campaign']],
    ['default', ['Alpha campaign', 'Zeta campaign', 'beta campaign']],
  ] as const) {
    await sort.selectOption(value);
    await expect(titles).toHaveText([...names]);
  }
  expect(writes).toEqual([]);
  await sort.selectOption('newest');
  await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).fill('Alpha');
  await expect(page.getByText('1 of 3 campaigns', { exact: true })).toBeVisible();
  await page.reload();
  await expect(sort).toHaveValue('newest');
  await publishCampaigns();
  await expect(titles).toHaveText(['Alpha campaign']);
  await expect(page.getByRole('button', { name: 'Clear filters', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  const clear = page.getByRole('button', { name: 'Clear filters', exact: true });
  await expect(page.getByRole('button', { name: 'All games', exact: true })).toHaveCount(0);
  await clear.click();
  await expect(titles).toHaveText(['beta campaign', 'Alpha campaign', 'Zeta campaign']);
  await expect(sort).toHaveValue('newest');
  await expect(page.getByRole('searchbox', { name: 'Search campaigns and rewards' })).toHaveValue(
    '',
  );
  const count = page.getByText('3 campaigns', { exact: true });
  const tabs = (await page
    .getByRole('navigation', { name: 'Campaign views', exact: true })
    .boundingBox())!;
  const countBox = (await count.boundingBox())!;
  await expect(count).toHaveCSS('font-size', '12px');
  await expect(count).toHaveCSS('color', 'rgb(136, 136, 136)');
  const refreshBox = (await page
    .getByRole('button', { name: 'Refresh inventory', exact: true })
    .boundingBox())!;
  expect(countBox.x + countBox.width).toBeLessThan(refreshBox.x);
  expect(Math.abs(countBox.y + countBox.height / 2 - tabs.y - tabs.height / 2)).toBeLessThan(2);
  expect(countBox.x).toBeGreaterThan(tabs.x + tabs.width);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: '../artifacts/campaign-sort-desktop.png', fullPage: true });
  await page.getByRole('link', { name: 'History', exact: true }).click();
  await expect(page.getByRole('link', { name: 'History', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expect(page.getByRole('combobox', { name: 'Sort history' })).toHaveValue('newest');
  await page.getByRole('link', { name: 'Available', exact: true }).click();
  await expect(titles).toHaveText(['beta campaign', 'Alpha campaign', 'Zeta campaign']);
  await sort.selectOption('ending');
  await sort.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(sort).toBeFocused();
  await expect
    .poll(() =>
      sort.evaluate((element) => getComputedStyle(element.parentElement!).backgroundColor),
    )
    .toBe('rgb(51, 51, 51)');
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 750 });
    await expect(sort).toBeVisible();
    await page.getByRole('button', { name: 'Filters', exact: true }).click();
    await expect(clear).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
    await page.keyboard.press('Escape');
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: '../artifacts/campaign-sort-phone.png', fullPage: true });
  await page.goto('/campaigns?sort=not-a-sort');
  await expect(sort).toHaveValue('default');
  await publishCampaigns();
  await expect(titles).toHaveText(['Alpha campaign', 'Zeta campaign', 'beta campaign']);
});
test('History contains recorded claims independently of campaign completion and coverage', async ({
  page,
  request,
}) => {
  await page.goto('/campaigns');
  const original = snapshot.campaigns[0]!;
  await expect(page.getByRole('button', { name: 'Stop mining Rust', exact: true })).toBeEnabled();
  const completed = {
    ...original,
    id: 'completed',
    name: 'Completed campaign',
    finished: true,
    active: false,
    expired: true,
    claimed_drops: 2,
    drops: original.drops.map((drop) => ({ ...drop, is_claimed: true, is_mineable: false })),
  };
  const expired = {
    ...original,
    id: 'expired',
    name: 'Expired campaign',
    active: false,
    expired: true,
  };
  const ignored = { ...original, id: 'ignored', name: 'Ignored campaign', mining_finished: true };
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: { campaigns: [expired, ignored, completed, original] },
    },
  });
  await expect(page.getByText('Completed campaign', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Ignored campaign', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
  await expect(page.getByText('Expired campaign', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('checkbox', { name: 'Expired', exact: true })).toHaveCount(0);
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: [
          {
            id: 'legacy',
            campaign_id: 'old',
            game: 'Rust',
            campaign: 'Historical campaign',
            drop_name: 'Old reward',
            required_minutes: 30,
            benefits: ['Old reward'],
            claimed_at: '2025-01-01T00:00:00Z',
          },
        ],
      },
    }),
  );
  await page.getByRole('link', { name: 'History', exact: true }).click();
  await expect(page.getByText('Historical campaign', { exact: true })).toBeVisible();
  await expect(page.getByText('Completed campaign', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Expired campaign', { exact: true })).toHaveCount(0);
  await expect(page.getByText('Ignored campaign', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('checkbox', { name: 'Game items', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Games', exact: true }).click();
  await page.getByRole('checkbox', { name: 'Rust', exact: true }).check();
  await expect(page.getByText('Historical campaign', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'All games', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).fill('Old reward');
  await expect(page.getByText('Historical campaign', { exact: true })).toBeVisible();
  await page.getByText('Historical campaign', { exact: true }).click();
  await expect(page.getByText('Old reward', { exact: true }).first()).toBeVisible();
  await expect(page.getByRole('button', { name: /Mine Rust|Stop mining Rust/ })).toHaveCount(0);
  await expect(
    page
      .getByRole('navigation', { name: 'Main navigation' })
      .getByRole('link', { name: 'History' }),
  ).toHaveCount(0);
  await page.screenshot({ path: '../artifacts/campaigns-history.png', fullPage: true });
});

test('History claims honor game filters and retry failed loading', async ({ page }) => {
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
    {
      times: 1,
    },
  );
  const started = page.waitForRequest((request) => request.url().endsWith('/api/history'));
  await page.goto('/campaigns?tab=finished');
  await started;
  try {
    await expect(page.getByText(/^Loading history/)).toHaveCount(0);
  } finally {
    finishLoad();
  }
  await expect(page.getByRole('alert')).toContainText('Could not load history. Try again.');
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: [
          {
            id: 'legacy',
            campaign_id: 'old',
            game: 'Old game',
            campaign: 'Historical campaign',
            drop_name: 'Old reward',
            required_minutes: 30,
            benefits: ['Old reward'],
            claimed_at: '2025-01-01T00:00:00Z',
          },
        ],
      },
    }),
  );
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await expect(retry).toHaveText('');
  await expect(retry).toHaveAttribute('title', 'Try again');
  await retry.click();
  await expect(page.getByText('Historical campaign', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Games/ }).click();
  await expect(page.getByRole('checkbox', { name: 'Old game', exact: true })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Rust', exact: true }).check();
  await expect(page.getByText('Historical campaign', { exact: true })).toHaveCount(0);
  await page.getByRole('checkbox', { name: 'Old game', exact: true }).check();
  await expect(page.getByText('Historical campaign', { exact: true })).toBeVisible();
});
test('discovery stays visible without mining until Mine is explicitly selected', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', { headers, data: { games_to_watch: [] } });
  await page.goto('/campaigns');
  await expect(page.getByText('Autumn expedition', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Mine Rust', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Rust']);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Stop mining Rust', exact: true })).toBeVisible();
  await page.goto('/?edit=priorities');
  await expect(page.locator('#priorities [data-game]')).toHaveCount(1);
  await page.getByRole('button', { name: /Remove Rust/ }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual([]);
  await page.goto('/campaigns');
  await expect(page.getByRole('button', { name: 'Mine Rust', exact: true })).toBeVisible();
});
test('game priorities show icons instead of editable numbers', async ({ page, request }) => {
  await page.goto('/?edit=priorities');
  await page.getByRole('combobox', { name: 'Search games...' }).fill('The Elder Scrolls Online');
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toContain('The Elder Scrolls Online');
  await expect(page.getByRole('spinbutton', { name: /Priority for/ })).toHaveCount(0);
  await page
    .getByRole('button', { name: 'Reorder The Elder Scrolls Online', exact: true })
    .press('ArrowUp');
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Rust', 'The Elder Scrolls Online', 'Sea of Thieves']);
  await page.reload();
  await expect(page.locator('#priorities [data-game]').nth(1)).toHaveAttribute(
    'data-game',
    'The Elder Scrolls Online',
  );
});

test('Twitch logout leaves the dashboard available and shows the next login', async ({ page }) => {
  await page.goto('/settings');
  const tabs = page.getByRole('navigation', { name: 'Settings sections' });
  await tabs.getByRole('link', { name: 'Connection', exact: true }).click();
  await expect(page.getByText('Dashboard connected', { exact: true })).toBeVisible();
  await tabs.getByRole('link', { name: 'Twitch account', exact: true }).click();
  await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
  await expect(page.getByText('NEWCODE', { exact: true })).toBeVisible();
  await tabs.getByRole('link', { name: 'Connection', exact: true }).click();
  await expect(page.getByText('Dashboard connected', { exact: true })).toBeVisible();
  await tabs.getByRole('link', { name: 'Twitch account', exact: true }).click();
  await expect(page.getByText('Connected', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Log out of Twitch', exact: true })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByText('NEWCODE', { exact: true })).toBeVisible();
});

test('manual game confirmation supports Escape and safe literal names', async ({ page }) => {
  await page.goto('/?edit=priorities');
  await page.getByRole('combobox', { name: 'Search games...' }).fill('<script>new game</script>');
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).not.toBeVisible();
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByText('<script>new game</script>', { exact: true })).toBeVisible();
});
test('autosave retains conflicting edits and retries only edited fields', async ({
  page,
  request,
}) => {
  await page.goto('/settings#connection');
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    '**/api/settings',
    async (route) => {
      await gate;
      await route.continue();
    },
    { times: 1 },
  );
  const sent = page.waitForRequest('**/api/settings');
  const interval = page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true });
  await interval.fill('45');
  await sent;
  const current = await (await request.get('/api/settings')).json();
  await request.post('/api/settings', {
    headers,
    data: {
      revision: current.revision,
      minimum_refresh_interval_minutes: 90,
      connection_quality: 3,
    },
  });
  release();
  await expect(page.getByRole('alert')).toContainText('Settings changed on another device');
  await expect(interval).toHaveValue('45');
  expect((await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes).toBe(
    90,
  );
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes,
    )
    .toBe(45);
  expect((await (await request.get('/api/settings')).json()).connection_quality).toBe(3);
});

test('history displays saved artwork, old entries and clearly labels unknown claim times', async ({
  page,
  request,
}) => {
  const history = await (await request.get('/api/history')).json();
  await page.route('https://static-cdn.jtvnw.net/reward.png', (route) =>
    route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" />',
    }),
  );
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        total: 3,
        entries: [
          { ...history.entries[0], image_url: 'https://static-cdn.jtvnw.net/reward.png' },
          { ...history.entries[0], id: 'legacy', drop_name: 'Older reward' },
          {
            ...history.entries[0],
            id: 'imported',
            drop_name: 'Imported badge',
            claimed_at_is_observed: true,
          },
        ],
      },
    }),
  );
  await page.goto('/history');
  await page.getByText('Autumn expedition', { exact: true }).click();
  const image = page.locator('img[src="https://static-cdn.jtvnw.net/reward.png"]');
  await expect(image).toBeVisible();
  await expect.poll(() => image.evaluate((node: HTMLImageElement) => node.naturalWidth)).toBe(40);
  await expect(page.getByText('Older reward', { exact: true })).toBeVisible();
  await expect(page.getByText('Imported badge', { exact: true })).toBeVisible();
  await expect(page.getByText(/^First observed /)).toBeVisible();
});

test('cache clearing removes history across open dashboards without exporting or reimporting it', async ({
  page,
  context,
  request,
}) => {
  await page.goto('/history');
  await expect(page).toHaveURL(/\/campaigns\?tab=history$/);
  await expect(page.getByText('Autumn expedition', { exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'CSV', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'JSON', exact: true })).toHaveCount(0);
  await expect(page.getByLabel('Since (UTC)', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Clear local history' })).toHaveCount(0);
  const other = await context.newPage();
  await other.goto('/settings#maintenance');
  await other.getByRole('button', { name: 'Clear All Cache', exact: true }).click();
  await expect(other.getByRole('dialog')).toContainText('local claim history');
  await other.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click();
  expect((await (await request.get('/api/history')).json()).entries).toHaveLength(1);
  await other.getByRole('button', { name: 'Clear All Cache', exact: true }).click();
  await other.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByText('No recorded claims yet')).toBeVisible();
  await page.reload();
  await expect(page.getByText('No recorded claims yet')).toBeVisible();
  await other.close();
});

test('catalog restrictions and hostile strings remain explicit and inert', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_status',
      data: { available: false, checked_at: new Date().toISOString() },
    },
  });
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('link', { name: 'Campaigns', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText(
    'Campaign data is incomplete or the public catalog is unavailable or stale',
  );
  await request.post('/__test/event', {
    headers,
    data: { event: 'console_output', data: { message: '<img src=x onerror="alert(1)">' } },
  });
  await page.getByRole('link', { name: 'Activity', exact: true }).click();
  await expect(page.getByText('<img src=x onerror="alert(1)">', { exact: true })).toBeVisible();
  expect(await page.locator('img[src="x"]').count()).toBe(0);
});
test('public catalog campaigns are visible without mining and expose source freshness in the refresh button', async ({
  page,
  request,
}) => {
  const catalogTime = new Date().toISOString();
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: {
        ...snapshot,
        current_drop: null,
        channels: [],
        wanted_items: [],
        settings: { ...snapshot.settings, games_to_watch: [] },
        inventory_status: {
          available: true,
          checked_at: catalogTime,
          catalog_updated_at: catalogTime,
        },
        campaigns: Array.from({ length: 145 }, (_, i) => ({
          ...snapshot.campaigns[0],
          id: `public-${i}`,
          name: `Public campaign ${i}`,
          linked: null,
        })),
      },
    },
  });
  await expect(
    page.getByRole('button', { name: 'Refresh inventory', exact: true }),
  ).toHaveAttribute('title', /Campaign catalog: SunkwiBOT. Updated/);
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Campaigns', exact: true })
    .click();
  await expect(page.getByText('145 campaigns', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Mine Rust', exact: true })).toHaveCount(25);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page
    .getByRole('searchbox', { name: 'Search campaigns and rewards' })
    .fill('Public campaign 144');
  await expect(page.getByText('Public campaign 144', { exact: true })).toBeVisible();
});

test('incomplete catalog preserves account data and unknown linkage without a discovery banner', async ({
  page,
  request,
}) => {
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: {
        ...snapshot,
        current_drop: { ...snapshot.current_drop!, drop_name: 'Account reward fixture' },
        inventory_status: { available: false, checked_at: new Date().toISOString() },
        campaigns: snapshot.campaigns.map((campaign) => ({ ...campaign, linked: null })),
      },
    },
  });
  await expect(page.getByText('Account reward fixture', { exact: true })).toBeVisible();
  await expect(page.getByText(/Found \d+ campaigns through live Twitch channels/)).toHaveCount(0);
  await page.getByRole('link', { name: 'Campaigns', exact: true }).click();
  await expect(page.getByText(/Found \d+ campaigns through live Twitch channels/)).toHaveCount(0);
  await expect(page.getByText('Account link unknown', { exact: true })).toHaveCount(0);
  await page.locator('.campaign-open').first().click();
  await expect(page.getByRole('link', { name: 'Check account link' }).first()).toBeVisible();
});
test('dashboard password, login, logout and API guard', async ({ page, browser, request }) => {
  await page.goto('/settings#access');
  await page
    .getByLabel('New password (at least 8 characters)', { exact: true })
    .fill('example-test-password');
  await page.getByLabel('Confirm new password', { exact: true }).fill('example-test-password');
  await page.getByRole('button', { name: 'Enable password protection', exact: true }).click();
  await expect(page.getByText('Password protection is enabled.', { exact: true })).toBeVisible();
  expect((await request.get('/api/settings')).status()).toBe(401);
  const context = await browser.newContext();
  const other = await context.newPage();
  await other.goto('http://127.0.0.1:8765/campaigns');
  await expect(other.getByRole('heading', { name: 'Unlock dashboard' })).toBeVisible();
  await other.getByLabel('Password', { exact: true }).fill('wrong');
  await other.getByRole('button', { name: 'Log in', exact: true }).click();
  await expect(other.getByRole('alert')).toContainText('Incorrect password');
  await other.getByLabel('Password', { exact: true }).fill('example-test-password');
  await other.getByRole('button', { name: 'Log in', exact: true }).click();
  await expect(other.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
  await other.getByRole('button', { name: 'Log out', exact: true }).click();
  await expect(other.getByRole('heading', { name: 'Unlock dashboard' })).toBeVisible();
  await context.close();
});
test('pages and confirmation dialogs meet automated accessibility checks', async ({ page }) => {
  for (const route of ['/', '/campaigns', '/history', '/activity', '/settings']) {
    await page.goto(route);
    await expect(page.locator('h1')).toBeVisible();
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
      .analyze();
    expect(results.violations, `${route}: ${JSON.stringify(results.violations)}`).toEqual([]);
  }
  await page.goto('/?edit=priorities');
  await page.getByRole('combobox', { name: 'Search games...' }).fill('Custom game');
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('saved game filters can be cleared after a campaign disappears', async ({ page, request }) => {
  const settings = await (await request.get('/api/settings')).json();
  await request.post('/api/settings', {
    headers,
    data: {
      revision: settings.revision,
      inventory_filters: { ...settings.inventory_filters, game_name_search: ['Old game'] },
    },
  });
  await page.goto('/campaigns');
  await expect(page.getByText('No matching results')).toBeVisible();
  await page.getByRole('button', { name: /^Games/ }).click();
  await expect(page.getByLabel('Old game', { exact: true })).toBeChecked();
  await page.getByRole('button', { name: 'All games', exact: true }).click();
  await expect(page.getByText('Autumn expedition', { exact: true })).toBeVisible();
});

test('a failed initial auth status remains recoverable without a page reload', async ({ page }) => {
  await page.route(
    '**/api/auth/status',
    (route) => route.fulfill({ status: 503, json: { detail: 'unavailable' } }),
    { times: 1 },
  );
  await page.reload();
  await expect(page.getByLabel('Password', { exact: true })).toBeVisible();
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  await expect(retry).toHaveText('');
  await expect(retry).toHaveAttribute('title', 'Try again');
  await retry.click();
  await expect(page.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
});

test('failed settings save retains input', async ({ page }) => {
  await page.goto('/settings#connection');
  await page.getByLabel('Proxy URL', { exact: true }).fill('http://127.0.0.1:9999');
  await page.route(
    '**/api/settings',
    (route) => route.fulfill({ status: 500, json: { detail: 'save_failed' } }),
    { times: 1 },
  );
  await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('Proxy URL', { exact: true })).toHaveValue('http://127.0.0.1:9999');
  await expect(page.getByText('gui.auth.save_failed')).toHaveCount(0);
});

test('autosave queues newer input, stays quiet and persists the latest edit across reload', async ({
  page,
  request,
}) => {
  await page.goto('/settings#connection');
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    '**/api/settings',
    async (route) => {
      await gate;
      await route.continue();
    },
    { times: 1 },
  );
  const sent = page.waitForRequest('**/api/settings');
  const interval = page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true });
  await interval.fill('45');
  await sent;
  await expect(interval).toBeEnabled();
  await expect(page.getByText(/^(Saving.*|Changes saved\.)$/)).toHaveCount(0);
  await interval.fill('60');
  release();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes,
    )
    .toBe(60);
  await expect(interval).toHaveValue('60');
  await expect(page.getByText('Changes saved.', { exact: true })).toHaveCount(0);
  await page.reload();
  await expect(interval).toHaveValue('60');
});

test('manual game confirmation appends to the latest settings from another device', async ({
  page,
  request,
}) => {
  await page.goto('/?edit=priorities');
  await page.getByRole('combobox', { name: 'Search games...' }).fill('Manual name');
  await page.getByRole('button', { name: 'Add Game', exact: true }).click();
  const settings = await (await request.get('/api/settings')).json();
  await request.post('/api/settings', {
    headers,
    data: { revision: settings.revision, games_to_watch: ['Another device'] },
  });
  await expect(page.locator('[data-game="Another device"]')).toHaveCount(1);
  await page.getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Another device', 'Manual name']);
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Another device',
    'Manual name',
  ]);
});

test('autosave survives reconnect and navigation', async ({ page, request }) => {
  await page.goto('/settings#connection');
  await page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true }).fill('45');
  await request.post('/__test/reconnect', { headers, data: {} });
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes,
    )
    .toBe(45);
  await page.goto('/settings#connection');
  await expect(page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true })).toHaveValue(
    '45',
  );
  await expect(page.getByRole('button', { name: 'Save changes', exact: true })).toHaveCount(0);
});

test('activity follows through bounded-buffer rollover and pauses for reading', async ({
  page,
  request,
}) => {
  await page.goto('/activity');
  await expect(page.getByLabel('Activity', { exact: true }).locator('article')).toHaveCount(3);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'initial_state',
      data: { ...snapshot, console: Array.from({ length: 1000 }, (_, i) => `Message ${i}`) },
    },
  });
  const log = page.getByLabel('Activity', { exact: true });
  await expect(log.locator('article')).toHaveCount(1000);
  await request.post('/__test/event', {
    headers,
    data: { event: 'console_output', data: { message: 'Newest message' } },
  });
  await expect
    .poll(() => log.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight))
    .toBeLessThan(2);
  await log.evaluate((el) => {
    el.scrollTop = 0;
  });
  const follow = page.getByRole('button', { name: 'Follow latest', exact: true });
  await expect(follow).toBeVisible();
  await expect(follow).toHaveText('');
  await expect(follow).toHaveAttribute('title', 'Follow latest');
  await request.post('/__test/event', {
    headers,
    data: { event: 'console_output', data: { message: 'Another message' } },
  });
  await expect.poll(() => log.evaluate((el) => el.scrollTop)).toBe(0);
  await follow.focus();
  await page.keyboard.press('Enter');
  await expect
    .poll(() => log.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight))
    .toBeLessThan(2);
  await expect(follow).toHaveAttribute('aria-pressed', 'true');
});

test('artwork expands Twitch dimensions before making a request', async ({ page, request }) => {
  const art = 'https://example.test/art/game-{width}x{height}.png';
  await page.route('https://example.test/**', (route) =>
    route.fulfill({
      contentType: 'image/png',
      body: Buffer.from(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aVRsAAAAASUVORK5CYII=',
        'base64',
      ),
    }),
  );
  await request.post('/__test/event', {
    headers,
    data: { event: 'channel_update', data: { ...snapshot.channels[0], game_icon: art } },
  });
  await expect(page.locator('img[src="https://example.test/art/game-80x112.png"]')).toBeVisible();
});

test('login uses translated strings supplied by the public auth endpoint', async ({ page }) => {
  await page.route('**/api/auth/status', (route) =>
    route.fulfill({
      json: {
        enabled: true,
        authenticated: false,
        translations: {
          login_title: 'Dashboard entsperren',
          password: 'Passwort',
          login: 'Anmelden',
        },
      },
    }),
  );
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Dashboard entsperren' })).toBeVisible();
  await expect(page.getByLabel('Passwort', { exact: true })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('password errors stay in Settings and protection changes reach a second browser', async ({
  page,
  browser,
}) => {
  await page.goto('/settings#access');
  await page
    .getByLabel('New password (at least 8 characters)', { exact: true })
    .fill('example-test-password');
  await page.getByLabel('Confirm new password', { exact: true }).fill('example-test-password');
  await page.getByRole('button', { name: 'Enable password protection', exact: true }).click();
  await expect(page.getByText('Password protection is enabled.', { exact: true })).toBeVisible();
  const context = await browser.newContext();
  const other = await context.newPage();
  await other.goto('http://127.0.0.1:8765/settings#access');
  await other.getByLabel('Password', { exact: true }).fill('example-test-password');
  await other.getByRole('button', { name: 'Log in', exact: true }).click();
  await expect(other.getByRole('heading', { name: 'Mining', exact: true })).toBeVisible();
  await other.goto('http://127.0.0.1:8765/settings#access');
  await expect(other.getByText('Password protection is enabled.', { exact: true })).toBeVisible();
  await page.getByLabel('Current password', { exact: true }).fill('wrong-password');
  await page.getByRole('button', { name: 'Disable protection', exact: true }).click();
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Disable protection', exact: true })
    .click();
  await expect(page.getByRole('dialog').getByRole('alert')).toContainText('Incorrect password');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByLabel('Current password', { exact: true }).fill('example-test-password');
  await page.getByRole('button', { name: 'Disable protection', exact: true }).click();
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Disable protection', exact: true })
    .click();
  await expect(other.getByText('Password protection is disabled.', { exact: true })).toBeVisible();
  await expect(
    other.getByRole('button', { name: 'Enable password protection', exact: true }),
  ).toBeVisible();
  await context.close();
});

test('explicit game priorities preserve manual spelling and case-insensitive uniqueness', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', {
    headers,
    data: { games_to_watch: ['Custom game', 'rust'] },
  });
  await page.goto('/?edit=priorities');
  const rows = page.locator('#priorities [data-game]');
  await expect(rows).toHaveCount(2);
  expect(
    await rows.evaluateAll((items) => items.map((item) => item.getAttribute('data-game'))),
  ).toEqual(['Custom game', 'rust']);
  await expect(page.getByRole('button', { name: 'Select All', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Reorder rust', exact: true }).press('ArrowUp');
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['rust', 'Custom game']);
  const games = (await (await request.get('/api/settings')).json()).games_to_watch;
  expect(games).toEqual(['rust', 'Custom game']);
  expect(games.filter((name: string) => name.toLowerCase() === 'rust')).toHaveLength(1);
});

test('history refreshes after claims and reports clear failure inside its dialog', async ({
  page,
  request,
}) => {
  await page.goto('/history');
  await page.getByText('Autumn expedition', { exact: true }).click();
  await expect(page.getByText('Canvas pack', { exact: true }).first()).toBeVisible();
  const history = await (await request.get('/api/history')).json();
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        total: 2,
        entries: [
          ...history.entries,
          { ...history.entries[0], id: 'new-claim', drop_name: 'New reward' },
        ],
      },
    }),
  );
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'drop_update',
      data: {
        campaign_id: 'campaign-1',
        campaign: { claimed_drops: 1 },
        drop: { ...snapshot.campaigns[0]!.drops[0], is_claimed: true },
      },
    },
  });
  await expect(page.getByText('New reward', { exact: true })).toBeVisible();
  await page.route('**/api/cache/clear', (route) =>
    route.fulfill({ status: 500, json: { detail: 'failure' } }),
  );
  await page.goto('/settings#maintenance');
  await page.getByRole('button', { name: 'Clear All Cache', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Confirm', exact: true }).click();
  await expect(page.getByRole('dialog').getByRole('alert')).toBeVisible();
  expect((await (await request.get('/api/history')).json()).entries).toHaveLength(1);
});

test('channel snapshots replace old rows and allow backend-verified special event streams', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', { headers, data: { games_to_watch: ['Special Events'] } });
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'channels_batch_update',
      data: {
        channels: [
          {
            ...snapshot.channels[0],
            name: 'event-host',
            login: 'event_host',
            game: 'Just Chatting',
            acl_based: true,
          },
        ],
      },
    },
  });
  const channels = page.getByRole('region', { name: 'Channels', exact: true });
  await expect(channels.getByRole('link', { name: 'event-host', exact: true })).toHaveAttribute(
    'href',
    'https://www.twitch.tv/event_host',
  );
  await expect(channels.getByRole('link', { name: 'northwind', exact: true })).toHaveCount(0);
});

test('manual lookup has no preparation message and clears pending state without a reload', async ({
  page,
  request,
}) => {
  const entry = page.getByRole('button', { name: 'Mine channel', exact: true });
  await expect(entry).toHaveText('');
  await expect(entry).toHaveAttribute('title', 'Mine channel');
  await expect(entry).toHaveAttribute('aria-expanded', 'false');
  await entry.click();
  await expect(entry).toHaveAttribute('aria-expanded', 'true');
  await page.getByRole('combobox', { name: 'Twitch channel name or URL' }).fill('ronnyberger');
  const mine = page.getByRole('button', { name: 'Mine', exact: true });
  await expect(mine).toBeEnabled();
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'manual_mode_update',
      data: { ...snapshot.manual_mode, pending_channel: 'metashi12' },
    },
  });
  await expect(mine).toBeDisabled();
  await expect(mine).toHaveAttribute('aria-busy', 'true');
  await expect(page.getByText('Preparing metashi12...')).toHaveCount(0);
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'Channels', exact: true }).locator('form'),
  ).toHaveCount(0);
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'manual_mode_update',
      data: {
        ...snapshot.manual_mode,
        pending_channel: null,
        error: 'Could not check that channel with Twitch. Try again.',
      },
    },
  });
  await expect(page.getByRole('alert')).toHaveText(
    'Could not check that channel with Twitch. Try again.',
  );
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  await expect(mine).toBeEnabled();
  await expect(mine).toHaveAttribute('aria-busy', 'false');
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'manual_mode_update',
      data: { ...snapshot.manual_mode, pending_channel: null, error: null },
    },
  });
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('manual channel entry accepts a URL and optional timer without selecting games', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', { headers, data: { games_to_watch: ['Other game'] } });
  await page.setViewportSize({ width: 1440, height: 420 });
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  const input = page.getByRole('combobox', { name: 'Twitch channel name or URL' });
  const mine = page.getByRole('button', { name: 'Mine', exact: true });
  await expect(input).toHaveAttribute('placeholder', 'Twitch channel name or URL');
  await expect(page.getByText(/Mine an eligible reward from your campaign catalog/)).toHaveCount(0);
  await input.fill('https://example.com/streamer');
  await mine.click();
  await expect(page.getByRole('alert')).toHaveText(
    'Enter a Twitch channel name or a direct twitch.tv channel URL.',
  );
  await input.fill('missing');
  await mine.click();
  await expect(page.getByRole('alert')).toHaveText('That Twitch channel was not found.');
  await page.getByRole('button', { name: 'Mine channel', exact: true }).click();
  await expect(input).toHaveValue('missing');
  const panel = page.getByRole('region', { name: 'Channels', exact: true });
  expect((await panel.boundingBox())!.height).toBeGreaterThan(90);
  await input.scrollIntoViewIfNeeded();
  await expect(input).toBeInViewport();
  await page.getByRole('alert').scrollIntoViewIfNeeded();
  await expect(page.getByRole('alert')).toBeInViewport();
  await page.screenshot({ path: '../artifacts/manual-channel-short.png' });
  await input.fill('https://www.twitch.tv/extra_streamer');
  await page.getByRole('spinbutton', { name: 'Auto mode after (minutes, optional)' }).fill('15');
  const selection = page.waitForRequest(
    (r) => r.url().endsWith('/api/channels/select') && r.method() === 'POST',
  );
  await mine.click();
  expect((await selection).postDataJSON().duration_minutes).toBe(15);
  await expect(panel.getByRole('link', { name: 'extra_streamer', exact: true })).toBeVisible();
  await expect(page.getByText('Manual selection', { exact: true })).toBeVisible();
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Other game',
  ]);
  await expect(page.getByText('Watching extra_streamer', { exact: true })).toBeVisible();
  await expect(page.getByText(/^Auto mode at /)).toBeVisible();
  await expect(page.getByText(/No eligible rewards from your campaign catalog/)).toHaveCount(0);
  await page.screenshot({ path: '../artifacts/manual-channel-timer-short.png' });
  expect((await new AxeBuilder({ page }).include('main').analyze()).violations).toEqual([]);
  await page.getByRole('button', { name: 'Return to Auto Mode' }).click();
  await expect(page.getByRole('img', { name: 'Automatic selection', exact: true })).toBeVisible();
  await expect(page.getByText(/^Auto mode at /)).toHaveCount(0);
});

test('empty selection asks for an explicit mining choice', async ({ page, request }) => {
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'settings_updated',
      data: { ...snapshot.settings, games_to_watch: [] },
    },
  });
  await request.post('/__test/event', { headers, data: { event: 'drop_progress_stop', data: {} } });
  await expect(page.getByText('Choose Mine on a campaign to select its game.')).toBeVisible();
  await expect(page.getByText('Choose the games you want to mine.')).toHaveCount(0);
});

test('phone campaign rows retain claimed counts and independent mining actions', async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await request.post('/api/settings', { headers, data: { games_to_watch: [] } });
  await page.goto('/campaigns');
  await expect(page.locator('.campaign-count')).toContainText('0/2');
  await expect(page.locator('.campaign-summary').getByText('Active', { exact: true })).toHaveCount(
    0,
  );
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: '../artifacts/campaigns-phone.png', fullPage: true });
  await page.getByRole('button', { name: 'Mine Rust', exact: true }).click();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Rust']);
  await expect(page.getByRole('complementary', { name: 'Campaign details' })).toHaveCount(0);
});

test('long international labels remain usable at phone, tablet and zoom-equivalent widths', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', {
    headers,
    data: {
      games_to_watch: ['EinSehrLangerSpielnameOhneTrennzeichen'.repeat(4), '日本語のゲーム'],
      language: 'Deutsch',
    },
  });
  await page.goto('/settings');
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');
  for (const width of [360, 640, 820]) {
    await page.setViewportSize({ width, height: 844 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  }
  await request.post('/api/settings', { headers, data: { language: 'العربية' } });
  await expect(page.locator('html')).not.toHaveAttribute('dir', 'rtl');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('autosave keeps text editing stable and blocks invalid values', async ({ page, request }) => {
  await page.goto('/?edit=priorities');
  const ignored = page.getByLabel('Ignore rewards by name', { exact: true });
  await ignored.fill('Mask\n');
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).drop_name_blacklist)
    .toEqual(['Mask']);
  await expect(ignored).toHaveValue('Mask\n');
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Settings', exact: true })
    .click();
  await page.getByRole('link', { name: 'Connection', exact: true }).click();
  const interval = page.getByLabel('Minimum Refresh Interval (minutes):', { exact: true });
  await interval.fill('');
  await expect(page.getByRole('alert')).toContainText('whole refresh interval');
  expect((await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes).toBe(
    30,
  );
  await interval.fill('20');
  await expect
    .poll(
      async () =>
        (await (await request.get('/api/settings')).json()).minimum_refresh_interval_minutes,
    )
    .toBe(20);
});

test('pointer dragging saves on drop and Escape cancels a second drag', async ({
  page,
  request,
}) => {
  await request.post('/api/settings', {
    headers,
    data: { games_to_watch: ['Rust', 'Sea of Thieves', 'The Elder Scrolls Online'] },
  });
  await page.goto('/?edit=priorities');
  const handle = page.getByRole('button', { name: 'Reorder Rust', exact: true });
  await handle.scrollIntoViewIfNeeded();
  const from = (await handle.boundingBox())!;
  const last = (await page.locator('[data-game="The Elder Scrolls Online"]').boundingBox())!;
  await page.mouse.move(from.x + 10, from.y + 10);
  await page.mouse.down();
  await page.mouse.move(last.x + 70, last.y + last.height - 3, { steps: 8 });
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Rust',
    'Sea of Thieves',
    'The Elder Scrolls Online',
  ]);
  await page.mouse.up();
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Sea of Thieves', 'The Elder Scrolls Online', 'Rust']);
  const moved = (await handle.boundingBox())!;
  const first = (await page.locator('[data-game="Sea of Thieves"]').boundingBox())!;
  await page.mouse.move(moved.x + 10, moved.y + 10);
  await page.mouse.down();
  await page.mouse.move(first.x + 70, first.y + 10, { steps: 8 });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(page.locator('#priorities [data-game]').last()).toHaveAttribute('data-game', 'Rust');
  expect((await (await request.get('/api/settings')).json()).games_to_watch).toEqual([
    'Sea of Thieves',
    'The Elder Scrolls Online',
    'Rust',
  ]);
});

test('touch dragging reorders game priorities', async ({ browser, request }) => {
  const context = await browser.newContext({
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:8765/?edit=priorities');
  const handle = page.getByRole('button', { name: 'Reorder Sea of Thieves', exact: true });
  await handle.scrollIntoViewIfNeeded();
  const from = (await handle.boundingBox())!;
  const first = (await page.locator('[data-game="Rust"]').boundingBox())!;
  const session = await context.newCDPSession(page);
  await session.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ x: from.x + 10, y: from.y + 10 }],
  });
  await session.send('Input.dispatchTouchEvent', {
    type: 'touchMove',
    touchPoints: [{ x: first.x + 70, y: first.y + 10 }],
  });
  await session.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await expect
    .poll(async () => (await (await request.get('/api/settings')).json()).games_to_watch)
    .toEqual(['Sea of Thieves', 'Rust']);
  await context.close();
});

test('copy confirmation expires, restarts and ignores superseded code results', async ({
  page,
  request,
}) => {
  await page.clock.install();
  await page.goto('/settings');
  await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
  const copy = page.getByRole('button', { name: 'Copy code', exact: true });
  const icon = copy.locator('path');
  await expect(copy).toBeVisible();
  await page.clock.pauseAt(await page.evaluate(() => Date.now() + 5000));
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', {
      value: () => Promise.resolve(),
      configurable: true,
    });
  });
  await expect(icon).toHaveAttribute('d', mdiContentCopy);
  await copy.click();
  await expect(icon).toHaveAttribute('d', mdiCheck);
  await page.clock.runFor(2000);
  await copy.click();
  await page.clock.runFor(2000);
  await expect(icon).toHaveAttribute('d', mdiCheck);
  await page.clock.runFor(999);
  await expect(icon).toHaveAttribute('d', mdiCheck);
  await page.clock.runFor(1);
  await expect(icon).toHaveAttribute('d', mdiContentCopy);
  await expect(copy).toHaveAttribute('title', 'Copy code');
  await expect(page.locator('#account [role="status"]')).toHaveCount(0);

  await copy.click();
  await expect(icon).toHaveAttribute('d', mdiCheck);
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', {
      value: () => Promise.reject(new Error('denied')),
      configurable: true,
    });
  });
  await copy.click();
  await expect(icon).toHaveAttribute('d', mdiContentCopy);
  await expect(page.getByRole('alert')).toContainText('Could not copy');
  await page.clock.runFor(4000);
  await expect(page.getByRole('alert')).toContainText('Could not copy');

  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', {
      value: () =>
        new Promise<void>((resolve) => {
          document.addEventListener('finish-copy', () => resolve(), { once: true });
        }),
      configurable: true,
    });
  });
  await copy.click();
  const updated = await request.post('/__test/event', {
    headers,
    data: {
      event: 'oauth_code_required',
      data: { code: 'REPLACED', url: 'https://www.twitch.tv/activate' },
    },
  });
  expect(updated.ok()).toBe(true);
  await expect(page.locator('#account code')).toHaveText('REPLACED');
  await page.evaluate(() => document.dispatchEvent(new Event('finish-copy')));
  await page.clock.runFor(100);
  await expect(icon).toHaveAttribute('d', mdiContentCopy);
  await expect(copy).toHaveAttribute('title', 'Copy code');
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('authorization Done keeps one login status and preserves request errors', async ({
  page,
  request,
}) => {
  const login = {
    status: 'Waiting for authentication...',
    user_id: null,
    oauth_pending: { code: 'TESTCODE', url: 'https://www.twitch.tv/activate' },
  };
  expect(
    (
      await request.post('/__test/event', {
        headers,
        data: { event: 'login_status', data: login },
      })
    ).ok(),
  ).toBe(true);
  await page.goto('/settings#account');
  const account = page.locator('#account');
  const status = account.getByText(login.status, { exact: true });
  const done = account.getByRole('button', { name: 'Done', exact: true });
  const result = account.locator('[role="status"], [role="alert"]');
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => (finish = resolve));
  await page.route('**/api/oauth/confirm', async (route) => {
    await pending;
    await route.fulfill({ status: 503, json: { detail: 'request_failed' } });
  });
  try {
    await done.click();
    await expect(done).toBeDisabled();
    await expect(status).toBeVisible();
  } finally {
    finish();
  }
  await expect(account.getByRole('alert')).toContainText('Request failed');
  await expect(done).toBeEnabled();
  await page.unroute('**/api/oauth/confirm');
  const confirmed = page.waitForResponse('**/api/oauth/confirm');
  await done.click();
  expect((await confirmed).ok()).toBe(true);
  await expect(done).toBeEnabled();
  await expect(status).toBeVisible();
  await expect(result).toHaveCount(0);
});

for (const width of [1280, 390, 320]) {
  test(`copy feedback keeps the authorization layout stable at ${width}px`, async ({
    page,
    context,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto('/settings');
    await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
    const copy = page.getByRole('button', { name: 'Copy code', exact: true });
    await expect(copy).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const layout = () =>
      page.evaluate(() => ({
        accountHeight: document.querySelector('#account')!.getBoundingClientRect().height,
        miningTop: document.querySelector('#account')!.getBoundingClientRect().top + scrollY,
      }));
    const before = await layout();
    await copy.click();
    await expect(copy).toHaveAttribute('title', 'Code copied');
    await expect(page.locator('#account [role="status"]')).toHaveText('Code copied');
    await expect(copy.locator('path')).toHaveAttribute('d', mdiCheck);
    expect(await layout()).toEqual(before);
    await copy.click();
    expect(await layout()).toEqual(before);

    await page.evaluate(() => {
      Object.defineProperty(navigator.clipboard, 'writeText', {
        value: () => Promise.reject(new Error('denied')),
        configurable: true,
      });
    });
    await copy.click();
    const error = page.getByRole('alert');
    await expect(error).toContainText('Could not copy');
    const row = await page.locator('.authorization-row').boundingBox();
    const notice = await error.boundingBox();
    expect(notice!.y - row!.y - row!.height).toBe(12);
    await page.evaluate(() => {
      Object.defineProperty(navigator.clipboard, 'writeText', {
        value: () => Promise.resolve(),
        configurable: true,
      });
    });
    await copy.click();
    await expect(error).toHaveCount(0);
    await expect(copy).toHaveAttribute('title', 'Code copied');
    expect(await layout()).toEqual(before);
  });
}

test('icon actions and authorization row use compact accessible controls', async ({
  page,
  context,
}) => {
  await page.goto('/campaigns');
  await page.getByRole('button', { name: 'Filters', exact: true }).click();
  const actions = page.locator('main .icon-button');
  for (const action of await actions.all()) {
    expect(await action.evaluate((node) => getComputedStyle(node).borderWidth)).toBe('0px');
    if (await action.evaluate((node) => node.tagName === 'BUTTON'))
      expect(await action.innerText()).toBe('');
  }
  const filters = page.getByRole('button', { name: 'Filters', exact: true });
  await filters.hover();
  await expect
    .poll(() => filters.evaluate((node) => getComputedStyle(node).backgroundColor))
    .toBe('rgb(51, 51, 51)');
  await page.goto('/settings');
  await expect(page.locator('#account .account-identity')).toBeVisible();
  await expect(page.locator('#account')).not.toContainText('Dashboard connected');
  await expect(page.locator('#connection')).toContainText('Dashboard connected');
  await page.getByRole('button', { name: 'Log out of Twitch', exact: true }).click();
  await expect(page.getByText('Enter this code at:', { exact: true })).toHaveCount(0);
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.getByRole('button', { name: 'Copy code', exact: true }).click();
  await expect(page.locator('#account .panel')).toHaveCount(0);
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('NEWCODE');
  const code = await page.locator('#account code').locator('..').boundingBox();
  const activate = await page.getByRole('link', { name: 'Twitch Activate' }).boundingBox();
  const done = await page.getByRole('button', { name: 'Done', exact: true }).boundingBox();
  expect(activate!.height).toBe(code!.height);
  expect(done!.height).toBe(code!.height);
  expect(code!.height).toBe(36);
  expect(done!.y).toBe(code!.y);
  await page.screenshot({ path: '../artifacts/authorization-controls.png', fullPage: true });
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', {
      value: () => Promise.reject(new Error('denied')),
      configurable: true,
    });
  });
  await page.getByRole('button', { name: 'Copy code', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Could not copy');
  await page.screenshot({ path: '../artifacts/settings-authorization.png', fullPage: true });
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    const controls = await Promise.all([
      page.locator('#account code').locator('..').boundingBox(),
      page.getByRole('link', { name: 'Twitch Activate' }).boundingBox(),
      page.getByRole('button', { name: 'Done', exact: true }).boundingBox(),
    ]);
    expect(controls.map((box) => box!.height)).toEqual([44, 44, 44]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      width,
    );
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test('native sort options stay legible with a light operating system theme', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  for (const [url, name] of [
    ['/campaigns', 'Sort campaigns'],
    ['/campaigns?tab=history', 'Sort history'],
    ['/?edit=priorities', 'Mining priority'],
  ]) {
    await page.goto(url!);
    const sort = page.getByRole('combobox', { name: name! });
    await expect(sort).toBeVisible();
    expect(await sort.evaluate((node) => getComputedStyle(node).colorScheme)).toBe('dark');
    for (const option of await sort.locator('option').all()) {
      expect(await option.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(
        'rgb(41, 41, 41)',
      );
      expect(await option.evaluate((node) => getComputedStyle(node).color)).toBe(
        'rgb(244, 244, 245)',
      );
    }
    await sort.click();
    await page.screenshot({ path: `../artifacts/${name!.replaceAll(' ', '-')}-open.png` });
    await page.keyboard.press('Escape');
    await sort.focus();
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    await expect(sort).toHaveValue(name === 'Mining priority' ? 'short_events' : 'newest');
    if (name === 'Mining priority') {
      await expect(sort.locator('..')).toHaveCSS('background-color', 'rgb(51, 51, 51)');
      await page.emulateMedia({ forcedColors: 'active' });
      await expect(sort.locator('..')).toHaveCSS('outline-style', 'solid');
    }
  }
});

test('search clear circles stay inside every search field on desktop and phone', async ({
  page,
}) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    for (const url of [
      '/',
      '/?edit=priorities',
      '/campaigns',
      '/campaigns?tab=history',
      '/activity',
    ]) {
      await page.goto(url);
      const search = url.includes('edit=')
        ? page.getByRole('combobox', { name: 'Search games...' })
        : page.getByRole('searchbox');
      await search.fill('example');
      const clear = page.getByRole('button', { name: 'Clear search', exact: true });
      await clear.hover();
      const fieldBox = (await search.boundingBox())!;
      const clearBox = (await clear.boundingBox())!;
      expect(clearBox.x).toBeGreaterThan(fieldBox.x);
      expect(clearBox.y - fieldBox.y).toBeGreaterThanOrEqual(3);
      expect(fieldBox.x + fieldBox.width - clearBox.x - clearBox.width).toBeGreaterThanOrEqual(3);
      expect(fieldBox.y + fieldBox.height - clearBox.y - clearBox.height).toBeGreaterThanOrEqual(3);
      expect(clearBox.width).toBe(clearBox.height);
      expect(
        await clear.evaluate((node) => parseFloat(getComputedStyle(node).borderRadius)),
      ).toBeGreaterThanOrEqual(clearBox.width / 2);
      if (url === '/' || url === '/settings')
        await page.screenshot({
          path: `../artifacts/search-${url === '/' ? 'channels' : 'mining'}-${width}.png`,
        });
      await clear.click();
      await expect(search).toHaveValue('');
      await expect(search).toBeFocused();
    }
  }
});

test('drag grips stay visible and plain while icon actions have circular hover backgrounds', async ({
  page,
}) => {
  await page.goto('/?edit=priorities');
  const grip = page.getByRole('button', { name: 'Reorder Rust', exact: true });
  await page.mouse.move(0, 0);
  await expect(grip).toHaveCSS('opacity', '1');
  await grip.hover();
  await expect(grip).toHaveCSS('opacity', '1');
  expect(await grip.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(
    'rgba(0, 0, 0, 0)',
  );
  await page.screenshot({ path: '../artifacts/plain-drag-grip.png' });
  const remove = page.getByRole('button', { name: 'Remove Rust', exact: true });
  await remove.hover();
  const box = (await remove.boundingBox())!;
  expect(
    await remove.evaluate((node) => parseFloat(getComputedStyle(node).borderRadius)),
  ).toBeGreaterThanOrEqual(box.width / 2);
  await page.emulateMedia({ forcedColors: 'active' });
  await grip.hover();
  await expect(grip).toHaveCSS('opacity', '1');
  expect(
    await grip.locator('span').evaluate((node) => getComputedStyle(node).backgroundImage),
  ).toContain('radial-gradient');
  await page.screenshot({ path: '../artifacts/drag-grip-forced-colors.png' });
});

test('History shares sort, search, game filters and layout while paging recorded campaigns', async ({
  page,
  request,
}) => {
  const original = (await (await request.get('/api/history')).json()).entries[0];
  let campaignCount = 27;
  await page.route('**/api/history', async (route) =>
    route.fulfill({
      json: {
        ...(await (await route.fetch()).json()),
        entries: Array.from({ length: campaignCount }, (_, index) => ({
          ...original,
          id: `reward-${index}`,
          campaign_id: `history-${index}`,
          campaign: `Campaign ${index}`,
          game: index === 26 ? 'Old game' : 'Rust',
          claimed_at: new Date(Date.UTC(2026, 8, index + 1)).toISOString(),
          benefits: [`Benefit ${index}`],
        })),
      },
    }),
  );
  await page.goto('/campaigns?tab=history');
  await expect(page.getByText('27 campaigns', { exact: true })).toBeVisible();
  const titles = page.locator('main .campaign-open > span > span.font-medium');
  await expect(titles).toHaveCount(25);
  await expect(titles.first()).toHaveText('Campaign 26');
  const previous = page.getByRole('button', { name: 'Previous page', exact: true });
  const next = page.getByRole('button', { name: 'Next page', exact: true });
  await expect(previous).toBeDisabled();
  await expect(next).toBeEnabled();
  for (const [button, label] of [
    [previous, 'Previous page'],
    [next, 'Next page'],
  ] as const) {
    await expect(button).toHaveText('');
    await expect(button).toHaveAttribute('title', label);
  }
  for (const width of [390, 375, 360, 320]) {
    await page.setViewportSize({ width, height: 844 });
    const pagination = page.getByRole('navigation', { name: 'Campaign pages' });
    await expect(pagination).toBeInViewport();
    const bounds = (await pagination.boundingBox())!;
    const search = (await page.getByRole('searchbox').boundingBox())!;
    const tabs = (await page.getByRole('navigation', { name: 'Campaign views' }).boundingBox())!;
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(search.y);
    expect(bounds.x).toBeGreaterThanOrEqual(tabs.x + tabs.width);
    expect(bounds.y + bounds.height / 2).toBeCloseTo(tabs.y + tabs.height / 2, 0);
    expect(bounds.x + bounds.width).toBeCloseTo(width - 16, 0);
    expect((await next.boundingBox())!.height).toBe(44);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.screenshot({ path: `../artifacts/history-pagination-${width}-phone.png` });
  }
  await page.screenshot({
    path: '../artifacts/history-pagination-icons-phone.png',
    fullPage: true,
  });
  await next.focus();
  await page.keyboard.press('Enter');
  await expect(titles).toHaveText(['Campaign 1', 'Campaign 0']);
  await expect(next).toBeDisabled();
  await expect(previous).toBeEnabled();
  await previous.click();
  await expect(titles).toHaveCount(25);
  await expect(previous).toBeDisabled();
  await next.click();
  await page.setViewportSize({ width: 1280, height: 720 });
  const sort = page.getByRole('combobox', { name: 'Sort history' });
  await sort.selectOption('name');
  await expect(titles.first()).toHaveText('Campaign 0');
  await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).fill('Benefit 26');
  await expect(titles).toHaveText(['Campaign 26']);
  await page.getByRole('button', { name: /^Games/ }).click();
  await page.getByRole('checkbox', { name: 'Rust', exact: true }).check();
  await expect(page.getByText('No matching results', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'All games', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Games', exact: true })).toBeVisible();
  await page.getByRole('searchbox', { name: 'Search campaigns and rewards' }).fill('');
  await expect(page.getByRole('button', { name: 'Games', exact: true })).toBeVisible();
  await expect(titles).toHaveCount(25);
  await expect(sort).toHaveValue('name');
  await page.getByRole('button', { name: 'Change campaign layout', exact: true }).click();
  await expect(page.locator('main .campaign-summary').first().locator('..')).toHaveClass(/grid/);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: /^Games/ }).click();
  await page.getByRole('checkbox', { name: 'Old game', exact: true }).check();
  await expect(titles).toHaveText(['Campaign 26']);
  await page.keyboard.press('Escape');
  await page.getByText('Campaign 26', { exact: true }).click();
  await page.screenshot({ path: '../artifacts/history-phone.png', fullPage: true });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  campaignCount = 2500;
  await page.setViewportSize({ width: 320, height: 844 });
  await page.goto('/campaigns?tab=history&page=99&game=');
  const pagination = page.getByRole('navigation', { name: 'Campaign pages' });
  await expect(pagination).toHaveText('100 / 100');
  const bounds = (await pagination.boundingBox())!;
  const search = (await page.getByRole('searchbox').boundingBox())!;
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(search.y);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(320);
  await expect(next).toBeDisabled();
  await previous.click();
  await expect(pagination).toHaveText('99 / 100');
  await page.screenshot({ path: '../artifacts/history-pagination-large-count-phone.png' });
});

test('an authoritative history clear removes cached claims even when reloading fails', async ({
  page,
  request,
}) => {
  await page.goto('/campaigns?tab=history');
  await expect(page.getByText('Autumn expedition', { exact: true })).toBeVisible();
  await page.route('**/api/history', (route) =>
    route.fulfill({ status: 503, json: { detail: 'request_failed' } }),
  );
  expect((await request.post('/api/cache/clear', { headers, data: {} })).ok()).toBe(true);
  await expect(page.getByRole('alert')).toContainText('Could not load history');
  await expect(page.getByText('Autumn expedition', { exact: true })).toHaveCount(0);
  await expect(page.locator('main .campaign-summary')).toHaveCount(0);
  expect((await (await request.get('/api/history')).json()).entries).toEqual([]);
  await page.unroute('**/api/history');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(page.getByText('No recorded claims yet', { exact: true })).toBeVisible();
});
