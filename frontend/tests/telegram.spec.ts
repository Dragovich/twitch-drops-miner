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
  await expect(page.getByText('Telegram settings saved.', { exact: true })).toBeVisible();
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
  await expect(page.getByText('Telegram settings saved.', { exact: true })).toBeVisible();
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
