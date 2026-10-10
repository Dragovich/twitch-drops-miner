import { invoke } from '@tauri-apps/api/core';
import { ApiError } from './api';
import type { StateAction } from '../../app/reducer';
import type { Subscription } from '../../app/transport';

export function nativeRequest(path: string, data: unknown, method: string) {
  const url = new URL(path, 'http://localhost');
  const reads: Record<string, string> = {
    '/api/auth/status': 'auth_status',
    '/api/settings': 'settings',
    '/api/telegram': 'telegram',
    '/api/history/stats': 'history_stats',
  };
  const writes: Record<string, string> = {
    '/api/settings': 'save_settings',
    '/api/telegram': 'save_telegram',
    '/api/telegram/test': 'test_telegram',
    '/api/channels/select': 'select_channel',
    '/api/games': 'games',
    '/api/settings/verify-proxy': 'verify_proxy',
    '/api/reload': 'reload',
    '/api/cache/clear': 'clear_cache',
    '/api/twitch/logout': 'logout',
    '/api/oauth/confirm': 'confirm_oauth',
    '/api/mode/exit-manual': 'exit_manual',
  };
  if (method === 'GET' && url.pathname === '/api/history') {
    const value: Record<string, string | number> = {};
    for (const name of ['game', 'campaign_id', 'since']) {
      const item = url.searchParams.get(name);
      if (item !== null) value[name] = item;
    }
    const limit = url.searchParams.get('limit');
    if (limit !== null) {
      if (!/^\d+$/.test(limit)) throw new ApiError(400, 'invalid_request');
      value.limit = Number(limit);
    }
    return { kind: 'history', value };
  }
  const kind =
    method === 'GET' ? reads[url.pathname] : method === 'POST' ? writes[url.pathname] : undefined;
  if (!kind) throw new ApiError(404, 'request_failed');
  return {
    kind,
    ...(['save_settings', 'save_telegram', 'select_channel', 'games', 'verify_proxy'].includes(kind)
      ? { value: data }
      : {}),
  };
}

export async function desktopRequest<T>(
  path: string,
  data: unknown,
  method: string,
  signal?: AbortSignal,
): Promise<T> {
  signal?.throwIfAborted();
  if (path === '/api/close' && method === 'POST') {
    await invoke('quit_app');
    return { success: true } as T;
  }
  const request = nativeRequest(path, data, method);
  const id = crypto.randomUUID();
  const abort = () => void invoke('cancel_request', { id }).catch(() => {});
  signal?.addEventListener('abort', abort, { once: true });
  try {
    const value = await invoke<T>('app_request', { request, id });
    signal?.throwIfAborted();
    return value;
  } catch (error) {
    signal?.throwIfAborted();
    const failure = error as { status?: number; detail?: string };
    throw new ApiError(failure?.status ?? 503, failure?.detail ?? 'request_failed');
  } finally {
    signal?.removeEventListener('abort', abort);
  }
}

// Serialize opening/closing across React remounts; an old open cannot replace a newer one.
let opening: Promise<unknown> = Promise.resolve();
export function connectDesktop(receive: (action: StateAction) => void): Subscription {
  let closed = false;
  let id = crypto.randomUUID();
  const close = (session: string) => invoke('state_close', { id: session }).catch(() => {});
  const start = () => {
    const session = id;
    const opened = opening.then(() => {
      if (closed || session !== id) return null;
      return invoke<StateAction>('state_open', { id: session });
    });
    opening = opened.catch(() => {});
    void (async () => {
      try {
        const first = await opened;
        if (!first || closed || session !== id) {
          await close(session);
          return;
        }
        receive(first);
        while (!closed && session === id) {
          const value = await invoke<StateAction>('state_next', { id: session, resync: false });
          if (!closed && session === id) receive(value);
        }
      } catch {
        if (!closed && session === id) receive({ type: 'disconnect' });
      }
    })();
  };
  start();
  return {
    resync: () => {
      const previous = id;
      id = crypto.randomUUID();
      void close(previous).then(start);
    },
    close: () => {
      closed = true;
      void close(id);
    },
  };
}
