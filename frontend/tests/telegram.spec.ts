import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

test('Telegram saves privately, tests explicitly and removes configuration', async ({
  page,
  request,
}) => {
  const headers = { 'X-TDM-Request': '1' };
  await request.post('/__test/reset', { headers, data: {} });
  const before = await (await request.get('/api/telegram')).json();
  await request.post('/api/telegram', {
    headers,
    data: { revision: before.revision, remove: true },
  });
  let tests = 0;
  await page.route('**/api/telegram/bot', async (route) => {
    const config = await (await request.get('/api/telegram')).json();
    return route.fulfill({
      json: {
        revision: config.revision,
        bot: { name: 'Drops Miner Alerts', username: 'drops_miner_bot' },
      },
    });
  });
  await page.route('**/api/telegram/test', (route) => {
    tests++;
    return route.fulfill({ json: { success: true } });
  });
  await page.goto('/settings#notifications');
  const enable = page.getByRole('checkbox', {
    name: 'Notify me when a reward is claimed',
    exact: true,
  });
  const image = page.getByRole('checkbox', { name: 'Include image', exact: true });
  const token = page.getByLabel('Bot token', { exact: true });
  const chat = page.getByLabel('Chat ID', { exact: true });
  await expect(enable).toBeEnabled();
  await expect(image).not.toBeChecked();
  await enable.check();
  await image.check();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText('Enter a valid bot token and numeric chat ID.');
  await expect(
    page.getByRole('button', { name: 'Reload saved settings', exact: true }),
  ).toHaveCount(0);
  await token.fill('123:private_test_token');
  await chat.fill('42');
  await expect(page.getByRole('button', { name: 'Send test', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await expect(page.getByText('Telegram settings saved.', { exact: true })).toHaveCount(0);
  const bot = page.getByRole('link', { name: 'Open Drops Miner Alerts in Telegram' });
  await expect(bot).toHaveAttribute('href', 'https://t.me/drops_miner_bot');
  await expect(page.getByRole('button', { name: 'Save', exact: true }).locator('svg')).toHaveCount(
    1,
  );
  await expect(page.getByRole('button', { name: 'Remove configuration', exact: true })).toHaveText(
    '',
  );
  await expect(token).toHaveValue('');
  await expect(token).toHaveAttribute('placeholder', 'Token saved');
  expect(tests).toBe(0);
  const saved = await (await request.get('/api/telegram')).json();
  expect(saved.token_configured).toBe(true);
  expect(saved.include_image).toBe(true);
  expect(JSON.stringify(saved)).not.toContain('private_test_token');
  expect(
    await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage })),
  ).not.toContain('private_test_token');
  await page.getByRole('button', { name: 'Send test', exact: true }).click();
  await expect(page.getByText('Test message sent.', { exact: true })).toBeVisible();
  expect(tests).toBe(1);
  await page.reload();
  await expect(token).toHaveValue('');
  await expect(enable).toBeChecked();
  await expect(image).toBeChecked();
  await chat.fill('-84');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await expect(page.getByText('Telegram settings saved.', { exact: true })).toHaveCount(0);
  await expect(bot).toBeVisible();
  expect((await (await request.get('/api/telegram')).json()).token_configured).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: '../artifacts/telegram-settings-mobile.png', fullPage: true });
  await page.setViewportSize({ width: 1365, height: 900 });
  await page.screenshot({ path: '../artifacts/telegram-settings-desktop.png', fullPage: true });
  await page.getByRole('button', { name: 'Remove configuration', exact: true }).click();
  await expect(enable).not.toBeChecked();
  await expect(image).not.toBeChecked();
  await expect(chat).toHaveValue('');
  await expect(bot).toHaveCount(0);
  expect((await (await request.get('/api/telegram')).json()).token_configured).toBe(false);
});

test('Telegram reports a configuration conflict without overwriting another client', async ({
  page,
  request,
}) => {
  const headers = { 'X-TDM-Request': '1' };
  await request.post('/__test/reset', { headers, data: {} });
  await page.goto('/settings#notifications');
  await expect(page.getByLabel('Chat ID', { exact: true })).toBeVisible();
  const saved = await (await request.get('/api/telegram')).json();
  await request.post('/api/telegram', {
    headers,
    data: { revision: saved.revision, chat_id: '73' },
  });
  await page.getByLabel('Chat ID', { exact: true }).fill('42');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText(
    'Settings changed on another device. Reopen Notifications to load the saved settings.',
  );
  expect((await (await request.get('/api/telegram')).json()).chat_id).toBe('73');
  await expect(page.getByLabel('Chat ID', { exact: true })).toHaveValue('42');
  await expect(
    page.getByRole('button', { name: 'Reload saved settings', exact: true }),
  ).toHaveCount(0);
  await page.getByRole('link', { name: 'Connection', exact: true }).click();
  await page.getByRole('link', { name: 'Notifications', exact: true }).click();
  await expect(page.getByLabel('Chat ID', { exact: true })).toHaveValue('73');
});

test('Bot metadata cannot block saves or show a different saved configuration', async ({
  page,
  request,
}) => {
  const headers = { 'X-TDM-Request': '1' };
  await request.post('/__test/reset', { headers, data: {} });
  const before = await (await request.get('/api/telegram')).json();
  await request.post('/api/telegram', {
    headers,
    data: { revision: before.revision, enabled: true, token: '123:test_token', chat_id: '42' },
  });
  let mode: 'stale' | 'failed' | 'current' = 'stale';
  let lookups = 0;
  const name = 'A very long bot name with <markup> & spaces that stays readable';
  await page.route('**/api/telegram/bot', async (route) => {
    lookups++;
    if (mode === 'failed') return route.fulfill({ status: 503 });
    const config = await (await request.get('/api/telegram')).json();
    return route.fulfill({
      json: {
        revision: mode === 'stale' ? before.revision : config.revision,
        bot: { name, username: 'drops_miner_bot' },
      },
    });
  });
  await page.goto('/settings#notifications');
  const link = page.getByRole('link', { name: `Open ${name} in Telegram` });
  await expect.poll(() => lookups).toBe(1);
  await expect(link).toHaveCount(0);
  mode = 'failed';
  await page.getByLabel('Chat ID', { exact: true }).fill('84');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => lookups).toBe(2);
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Send test', exact: true })).toBeEnabled();
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect((await (await request.get('/api/telegram')).json()).chat_id).toBe('84');
  mode = 'current';
  await page.reload();
  await expect(link).toBeVisible();
  await page.setViewportSize({ width: 320, height: 740 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(link).toHaveText(name);
  await page.getByLabel('Bot token', { exact: true }).fill('456:new_token');
  await expect(link).toHaveCount(0);
});
