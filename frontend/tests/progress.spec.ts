import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };

for (const width of [1440, 390]) {
  test(`progress percentage is saved from Mining preferences at ${width}px`, async ({
    page,
    request,
  }) => {
    await request.post('/__test/reset', { headers, data: {} });
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    const mining = page.getByRole('region', { name: 'Now mining', exact: true });
    await expect(mining.getByText('42 / 60 min', { exact: true })).toBeVisible();
    await page.goto('/?edit=priorities');
    const setting = page.getByRole('checkbox', { name: 'Show progress percentage' });
    await expect(setting).not.toBeChecked();
    await setting.check();
    await expect
      .poll(
        async () => (await (await request.get('/api/settings')).json()).show_progress_percentage,
      )
      .toBe(true);
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await page.reload();
    await expect(mining.getByText('42 / 60 min (70%)', { exact: true })).toBeVisible();
    await page.screenshot({ path: `../artifacts/progress-percentage-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    await request.post('/__test/event', {
      headers,
      data: {
        event: 'drop_progress',
        data: { ...fixture.current_drop, confirmed_minutes: 59, current_minutes: 60 },
      },
    });
    await expect(mining.getByText('59 / 60 min (98%)', { exact: true })).toBeVisible();
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: { ...fixture.current_drop, confirmed_at: null } },
    });
    await expect(mining.getByText('0 / 60 min', { exact: true })).toBeVisible();
    await expect(mining.getByText(/\d+%/)).toHaveCount(0);
    await page.goto('/?edit=priorities');
    await expect(setting).toBeChecked();
    await setting.uncheck();
    await expect
      .poll(
        async () => (await (await request.get('/api/settings')).json()).show_progress_percentage,
      )
      .toBe(false);
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: fixture.current_drop },
    });
    await page.getByRole('link', { name: 'Back to Mining', exact: true }).click();
    await expect(mining.getByText('42 / 60 min', { exact: true })).toBeVisible();
  });
}
