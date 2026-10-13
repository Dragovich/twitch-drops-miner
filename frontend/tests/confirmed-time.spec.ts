import { expect, test } from '@playwright/test';
import fixture from './fixture.json' with { type: 'json' };

const headers = { 'X-TDM-Request': '1' };
test.use({ locale: 'en-US', timezoneId: 'Europe/Berlin' });
test.beforeEach(async ({ request }) => {
  expect((await request.post('/__test/reset', { headers, data: {} })).ok()).toBeTruthy();
});

test('confirmation timestamps keep the full tooltip and add dates only when needed', async ({
  page,
  request,
}) => {
  await page.clock.install({ time: new Date('2026-10-12T12:40:00Z') });
  await page.goto('/');
  for (const [confirmed, expected] of [
    ['2026-10-12T12:36:00Z', /^Confirmed 14:36$/],
    ['2026-10-11T12:36:00Z', /^Confirmed Oct 11.*14:36$/],
    ['2025-10-12T12:36:00Z', /^Confirmed Oct 12, 2025.*14:36$/],
    ['2026-10-11T22:00:00Z', /^Confirmed 00:00$/],
  ] as const) {
    await request.post('/__test/event', {
      headers,
      data: { event: 'drop_progress', data: { ...fixture.current_drop, confirmed_at: confirmed } },
    });
    const time = page.locator(`time[datetime="${confirmed}"]`);
    await expect(time).toHaveText(expected);
    await expect(time).toHaveAttribute('title', new RegExp(confirmed.slice(0, 4)));
    await expect(time).toHaveAttribute('aria-label', (await time.getAttribute('title'))!);
  }
  const confirmed = '2026-10-12T12:36:00Z';
  const campaign = fixture.campaigns[0]!;
  await request.post('/__test/event', {
    headers,
    data: {
      event: 'inventory_batch_update',
      data: {
        campaigns: [
          {
            ...campaign,
            drops: campaign.drops.map((drop) => ({ ...drop, confirmed_at: confirmed })),
          },
        ],
      },
    },
  });
  await page.goto(`/campaigns?campaign=${campaign.id}`);
  const detail = page.getByRole('complementary', { name: 'Campaign details' });
  await expect(detail.getByText(/Confirmed /)).toHaveCount(0);
  await expect(detail.locator('time')).toHaveCount(0);
  await expect(detail.getByRole('progressbar', { name: 'Explorer jacket' })).toHaveAttribute(
    'aria-valuenow',
    '42',
  );
});

test('a confirmation gains its date at local midnight without another progress report', async ({
  page,
  request,
}) => {
  await page.clock.install({ time: new Date('2026-10-12T21:59:00Z') });
  const confirmed = '2026-10-12T12:36:00Z';
  await request.post('/__test/event', {
    headers,
    data: { event: 'drop_progress', data: { ...fixture.current_drop, confirmed_at: confirmed } },
  });
  await page.goto('/');
  const time = page.locator(`time[datetime="${confirmed}"]`);
  await expect(time).toHaveText('Confirmed 14:36');
  await page.clock.runFor(61_000);
  await expect(time).toHaveText(/^Confirmed Oct 12.*14:36$/);
  await expect(time).toHaveAttribute('datetime', confirmed);
});
