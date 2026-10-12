import { afterEach, expect, it, vi } from 'vitest';
import { lookupGames } from '../src/features/mining/useDirectorySearch';

afterEach(() => vi.unstubAllGlobals());

it('batches saved-name lookups by Twitch count and UTF-8 request limits', async () => {
  const fetch = vi.fn<typeof globalThis.fetch>(
    async () => new Response('[]', { headers: { 'Content-Type': 'application/json' } }),
  );
  vi.stubGlobal('fetch', fetch);
  const names = [
    ...Array.from({ length: 105 }, (_, i) => `Game ${i}`),
    ...Array(10).fill('遊'.repeat(200)),
  ];
  await lookupGames(names, new AbortController().signal);
  const batches = fetch.mock.calls.map(
    (call) => JSON.parse(call[1]!.body as string).names as string[],
  );
  expect(batches.flat()).toEqual(names);
  expect(batches.length).toBeGreaterThan(1);
  for (const batch of batches) {
    expect(batch.length).toBeLessThanOrEqual(100);
    expect(new TextEncoder().encode(batch.join('')).length).toBeLessThanOrEqual(4000);
  }
});
