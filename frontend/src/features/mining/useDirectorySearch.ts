import { useEffect, useState } from 'react';
import { request } from '../../shared/lib/api';
import type { GameMetadata } from '../../shared/lib/types';

export async function lookupGames(names: string[], signal: AbortSignal): Promise<GameMetadata[]> {
  const games: GameMetadata[] = [];
  let batch: string[] = [];
  let bytes = 0;
  const encoder = new TextEncoder();
  for (const name of [...names, '']) {
    const length = encoder.encode(name).length;
    if (batch.length && (!name || batch.length === 100 || bytes + length > 4000)) {
      games.push(
        ...(await request<GameMetadata[]>('/api/games', { names: batch }, 'POST', signal)),
      );
      batch = [];
      bytes = 0;
    }
    if (name) {
      batch.push(name);
      bytes += length;
    }
  }
  return games;
}

export function useDirectorySearch<T>(
  path: '/api/games' | '/api/channels/search',
  query: string,
  connected: boolean,
  user: number | null | undefined,
) {
  const [attempt, retry] = useState(0);
  const [result, setResult] = useState<{
    query: string;
    user: number;
    items: T[];
    error: boolean;
  } | null>(null);
  const term = query.trim();
  useEffect(() => {
    setResult(null);
    if (!term || !connected || !user) return;
    const controller = new AbortController();
    const timer = window.setTimeout(async () => {
      try {
        const items = await request<T[]>(path, { search: term }, 'POST', controller.signal);
        if (!controller.signal.aborted) setResult({ query: term, user, items, error: false });
      } catch {
        if (!controller.signal.aborted) setResult({ query: term, user, items: [], error: true });
      }
    }, 300);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [path, term, connected, user, attempt]);
  const current =
    result?.query === term && result.user === user && connected && user ? result : null;
  return {
    items: current?.items ?? [],
    loading: !!term && connected && !!user && !current,
    error: current?.error ?? false,
    complete: !!current,
    retry: () => retry((value) => value + 1),
  };
}
