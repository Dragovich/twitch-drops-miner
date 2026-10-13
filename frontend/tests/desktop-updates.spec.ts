import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { mdiDownload, mdiFolderOutline } from '@mdi/js';
import fixture from './fixture.json' with { type: 'json' };
import type { UpdateStatus } from '../src/features/settings/DesktopUpdates';

declare global {
  interface Window {
    updateFixture: {
      status: UpdateStatus;
      calls: string[];
      failNext: boolean;
      publish: (value: Partial<UpdateStatus>) => void;
      emit: (event: string, payload?: unknown) => void;
    };
  }
}

test.beforeEach(async ({ page }) => {
  // Exercise the bundled desktop UI with fake IPC only; never contact Twitch or install anything.
  await page.addInitScript(
    ({ snapshot }) => {
      let id = 0;
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<number, { event: string; handler: number }>();
      const control: Window['updateFixture'] = {
        status: {
          revision: 1,
          phase: 'current',
          current_version: '2.0.1',
          version: null,
          downloaded: 0,
          total: null,
          error: null,
          restart_required: false,
        },
        calls: [],
        failNext: false,
        emit(event, payload) {
          for (const [id, listener] of listeners) {
            if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
          }
        },
        publish(value) {
          control.status = { ...control.status, ...value, revision: control.status.revision + 1 };
          control.emit('desktop-update', control.status);
        },
      };
      window.updateFixture = control;
      Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
        value: {
          unregisterListener: (_event: string, id: number) => listeners.delete(id),
        },
      });
      Object.defineProperty(window, '__TAURI_INTERNALS__', {
        value: {
          transformCallback: (callback: (event: unknown) => void) => {
            callbacks.set(++id, callback);
            return id;
          },
          invoke: async (
            command: string,
            args: {
              event?: string;
              handler?: number;
              action?: string;
              folder?: string;
              request?: { kind: string };
            } = {},
          ) => {
            if (command === 'plugin:event|listen') {
              listeners.set(++id, { event: args.event!, handler: args.handler! });
              return id;
            }
            if (command === 'app_request') {
              if (args.request?.kind === 'auth_status')
                return { enabled: false, authenticated: true };
              if (args.request?.kind === 'settings') return snapshot.settings;
              throw new Error('Unexpected fixture request');
            }
            if (command === 'state_open') return { type: 'snapshot', value: snapshot };
            if (command === 'state_next') return new Promise(() => {});
            if (command === 'desktop_settings')
              return {
                version: '2.0.1',
                tray_available: true,
                autostart: false,
                start_minimized: false,
                close_to_tray: false,
                keep_awake: false,
                keep_awake_failed: false,
                notifications: false,
              };
            if (command === 'open_app_folder') {
              control.calls.push(`folder:${args.folder}`);
              return;
            }
            if (command === 'restart_app') {
              control.calls.push(command);
              return;
            }
            if (command === 'desktop_update') {
              if (args.action) {
                control.calls.push(args.action);
                if (control.failNext) {
                  control.failNext = false;
                  throw new Error('Offline IPC failure');
                }
                if (args.action === 'check')
                  control.publish({ phase: 'checking', version: null, error: null });
                if (args.action === 'download')
                  control.publish({
                    phase: 'downloading',
                    error: null,
                    downloaded: 0,
                    total: null,
                  });
                if (args.action === 'cancel')
                  control.publish({ phase: control.status.version ? 'available' : 'idle' });
                if (args.action === 'install') control.publish({ phase: 'installing' });
              }
              return { ...control.status };
            }
          },
        },
      });
    },
    { snapshot: fixture },
  );
  await page.route('https://**', (route) => route.abort());
  await page.goto('/');
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  await page.getByRole('link', { name: 'Maintenance', exact: true }).click();
});

for (const width of [1280, 390]) {
  test(`desktop updates stay inline through download, navigation and install at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 850 });
    const updates = page.getByRole('region', { name: 'App updates' });
    const check = updates.getByRole('button', { name: 'Check for updates', exact: true });
    await expect(updates.getByTitle('Installed version: 2.0.1')).toHaveText('v2.0.1');
    await expect(updates.getByRole('status')).toHaveText("You're up to date.");
    await expect(page.locator('#maintenance details')).toHaveCount(0);
    await expect(page.locator('#maintenance')).not.toContainText(
      'This application automatically mines',
    );
    await page.screenshot({ path: `../artifacts/desktop-updates-current-${width}.png` });
    await check.focus();
    await check.press('Enter');
    await expect(updates.getByRole('button', { name: 'Checking for updates…' })).toBeDisabled();
    await expect(updates.getByRole('status')).toHaveText('Checking for updates…');
    await expect(updates.getByRole('button', { name: 'Cancel', exact: true })).toHaveCount(0);
    await page.evaluate(() =>
      window.updateFixture.publish({ phase: 'available', version: '2.0.2' }),
    );
    await expect(updates.getByRole('status')).toHaveText('Version 2.0.2 is available.');
    const updateLink = page.getByRole('link', { name: 'Update v2.0.2', exact: true });
    const linkBox = (await updateLink.boundingBox())!;
    if (width >= 1024) {
      const github = (await page.getByRole('link', { name: 'GitHub repository' }).boundingBox())!;
      expect(linkBox.x).toBeGreaterThanOrEqual(github.x + github.width);
      expect(
        Math.abs(linkBox.y + linkBox.height / 2 - (github.y + github.height / 2)),
      ).toBeLessThan(2);
    } else {
      const brand = (await page.locator('.brand-row').boundingBox())!;
      expect(linkBox.y + linkBox.height).toBeLessThanOrEqual(brand.y + brand.height);
    }
    await expect(check).toBeEnabled();
    await expect(updates.getByRole('link', { name: 'Release notes' })).toHaveAttribute(
      'href',
      'https://github.com/ohne-b/twitch-drops-miner/releases/tag/v2.0.2',
    );
    await page.screenshot({ path: `../artifacts/desktop-updates-available-${width}.png` });
    await expect(
      updates.getByRole('button', { name: 'Update', exact: true }).locator('path'),
    ).toHaveAttribute('d', mdiDownload);
    await updates.getByRole('button', { name: 'Update', exact: true }).click();
    await expect(check).toBeDisabled();
    await expect(updates.getByText('0.0 MB downloaded', { exact: true })).toBeVisible();
    await expect(updates.getByRole('progressbar')).toHaveCount(0);
    await page.evaluate(() => window.updateFixture.publish({ downloaded: 2_500_000, total: 0 }));
    await expect(updates.getByText('2.5 MB downloaded', { exact: true })).toBeVisible();
    await expect(updates.getByRole('progressbar')).toHaveCount(0);
    await page.evaluate(() =>
      window.updateFixture.publish({ downloaded: 24_000_000, total: 50_000_000 }),
    );
    await page.getByRole('link', { name: 'Mining', exact: true }).click();
    await page.getByRole('link', { name: 'Update v2.0.2', exact: true }).click();
    await expect(updates.getByRole('status')).toHaveText('Downloading 2.0.2...');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    const progress = updates.getByRole('progressbar', { name: 'Downloading 2.0.2...' });
    await expect(progress).toHaveAttribute('aria-valuenow', '24000000');
    await expect(progress).toHaveAttribute('aria-valuemax', '50000000');
    await expect(progress.locator('div')).toHaveAttribute('style', 'width: 48%;');
    await expect(updates.getByText('24.0 / 50.0 MB (48%)', { exact: true })).toBeVisible();
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: `../artifacts/desktop-updates-downloading-${width}.png` });
    await updates.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(progress).toHaveCount(0);
    await expect(updates.getByRole('button', { name: 'Update', exact: true })).toBeEnabled();
    await updates.getByRole('button', { name: 'Update', exact: true }).click();
    await page.evaluate(() => {
      window.updateFixture.publish({ phase: 'ready' });
      window.updateFixture.emit('desktop-update', {
        ...window.updateFixture.status,
        revision: 0,
        phase: 'downloading',
      });
    });
    await expect(updates.getByRole('button', { name: 'Install and restart' })).toBeEnabled();
    await expect(progress).toHaveCount(0);
    await expect(check).toBeDisabled();
    await page.screenshot({ path: `../artifacts/desktop-updates-ready-${width}.png` });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await page.evaluate(() => window.updateFixture.calls)).toEqual([
      'check',
      'download',
      'cancel',
      'download',
    ]);
    await updates.getByRole('button', { name: 'Install and restart' }).click();
    await expect(updates.getByRole('status')).toHaveText(
      'Stopping mining and installing the update...',
    );
    await expect(updates.getByRole('button', { name: 'Cancel' })).toHaveCount(0);
    expect(await page.evaluate(() => window.updateFixture.calls.at(-1))).toBe('install');
  });
}

test('long update labels stay beside GitHub or the compact brand without overflowing', async ({
  page,
}) => {
  const version = '2.2.0-preview.123456789012345678901234567890';
  await page.evaluate(
    (version) => window.updateFixture.publish({ phase: 'available', version }),
    version,
  );
  const link = page.getByRole('link', { name: `Update v${version}`, exact: true });
  for (const width of [1280, 390, 320]) {
    await page.setViewportSize({ width, height: 850 });
    await expect(link).toBeVisible();
    await expect(link).toHaveAttribute('title', `Version ${version} is available.`);
    expect(await link.locator('span').evaluate((span) => span.scrollWidth > span.clientWidth)).toBe(
      true,
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    const box = (await link.boundingBox())!;
    const beside = (await (
      width >= 1024
        ? page.getByRole('link', { name: 'GitHub repository' })
        : page.locator('.brand-row')
    ).boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(beside.x + beside.width);
    expect(Math.abs(box.y + box.height / 2 - (beside.y + beside.height / 2))).toBeLessThan(2);
  }
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await link.focus();
  await link.press('Enter');
  await expect(page.getByRole('region', { name: 'App updates' })).toBeVisible();
  expect(await page.evaluate(() => window.updateFixture.calls)).toEqual([]);
});

for (const width of [1280, 390, 320]) {
  test(`download progress keeps controls and following content still at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 850 });
    const updates = page.getByRole('region', { name: 'App updates' });
    await page.evaluate(() =>
      window.updateFixture.publish({ phase: 'available', version: '2.2.0' }),
    );
    const positions = async () => ({
      updates: await updates.boundingBox(),
      actionY: (await updates
        .getByRole('button', { name: /^(Update|Cancel|Install and restart)$/ })
        .boundingBox())!.y,
      notesY: (await updates.getByRole('link', { name: 'Release notes' }).boundingBox())!.y,
      clearY: (await page
        .getByRole('button', { name: 'Clear All Cache', exact: true })
        .boundingBox())!.y,
      sidebar: await page.getByRole('complementary').boundingBox(),
    });
    const available = await positions();
    await updates.getByRole('button', { name: 'Update', exact: true }).click();
    await expect(updates.getByText('0.0 MB downloaded', { exact: true })).toBeVisible();
    expect(await positions()).toEqual(available);
    await page.evaluate(() =>
      window.updateFixture.publish({ downloaded: 24_000_000, total: 50_000_000 }),
    );
    await expect(updates.getByRole('progressbar')).toBeVisible();
    expect(await positions()).toEqual(available);
    await updates.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(updates.getByRole('progressbar')).toHaveCount(0);
    expect(await positions()).toEqual(available);
    await updates.getByRole('button', { name: 'Update', exact: true }).click();
    await page.evaluate(() => window.updateFixture.publish({ phase: 'ready' }));
    await expect(updates.getByRole('button', { name: 'Install and restart' })).toBeVisible();
    expect(await positions()).toEqual(available);
  });
}

test('tray checks and update failures retain inline recovery', async ({ page }) => {
  const updates = page.getByRole('region', { name: 'App updates' });
  const check = updates.getByRole('button', { name: 'Check for updates', exact: true });
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  await page.evaluate(() => window.updateFixture.emit('desktop-update-open'));
  await expect(updates).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.updateFixture.calls)).toEqual(['check']);
  await page.evaluate(() =>
    window.updateFixture.publish({ phase: 'failed', error: 'check_failed' }),
  );
  await expect(updates.getByRole('alert')).toHaveText('Could not check for updates. Try again.');
  await page.evaluate(() => {
    window.updateFixture.failNext = true;
  });
  await check.click();
  await expect(updates.getByRole('alert')).toHaveText('Could not complete that action. Try again.');
  await check.click();
  await page.evaluate(() => window.updateFixture.publish({ phase: 'current' }));
  await expect(updates.getByRole('status')).toHaveText("You're up to date.");
  await page.evaluate(() =>
    window.updateFixture.publish({ phase: 'failed', version: '2.0.2', error: 'download_failed' }),
  );
  await expect(updates.getByRole('alert')).toContainText('Try downloading again.');
  await expect(updates.getByRole('button', { name: 'Update', exact: true })).toBeEnabled();
  await page.evaluate(() =>
    window.updateFixture.publish({ error: 'install_failed', restart_required: true }),
  );
  await expect(updates.getByRole('alert')).toContainText('Restart the app to resume mining');
  await expect(check).toBeDisabled();
  await page.screenshot({ path: '../artifacts/desktop-updates-recovery.png' });
  await page.getByRole('link', { name: 'Mining', exact: true }).click();
  const calls = await page.evaluate(() => window.updateFixture.calls);
  await page.evaluate(() => window.updateFixture.emit('desktop-update-open'));
  await expect(updates.getByRole('button', { name: 'Restart app' })).toBeVisible();
  expect(await page.evaluate(() => window.updateFixture.calls)).toEqual(calls);
  await updates.getByRole('button', { name: 'Restart app' }).click();
  expect(await page.evaluate(() => window.updateFixture.calls.at(-1))).toBe('restart_app');
});

test('compact desktop folder actions retain accessible names and native destinations', async ({
  page,
}) => {
  await page.getByRole('link', { name: 'Desktop', exact: true }).click();
  for (const [label, folder] of [
    ['Data', 'data'],
    ['Logs', 'logs'],
  ] as const) {
    const button = page.getByRole('button', {
      name: `Open ${folder} folder`,
      exact: true,
    });
    await expect(button).toHaveText(label);
    await expect(button.locator('path')).toHaveAttribute('d', mdiFolderOutline);
    await button.focus();
    await button.press('Enter');
    await expect
      .poll(() => page.evaluate(() => window.updateFixture.calls.at(-1)))
      .toBe(`folder:${folder}`);
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: '../artifacts/desktop-folder-actions.png' });
});
