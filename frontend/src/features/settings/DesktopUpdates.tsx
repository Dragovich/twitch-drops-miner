import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { mdiDownload } from '@mdi/js';
import { Icon } from '@mdi/react';
import { Link, useNavigate } from 'react-router';
import { useT } from '../../shared/lib/i18n';
import { isDesktop } from '../../shared/lib/platform';
import { Button, ProgressBar } from '../../shared/ui';
import UpdateCheck from './UpdateCheck';

export type UpdateStatus = {
  revision: number;
  phase:
    | 'idle'
    | 'checking'
    | 'current'
    | 'available'
    | 'downloading'
    | 'ready'
    | 'installing'
    | 'failed';
  current_version: string;
  version: string | null;
  downloaded: number;
  total: number | null;
  error: string | null;
  restart_required: boolean;
};

export function newerUpdate(current: UpdateStatus | null, next: UpdateStatus) {
  return current && current.revision > next.revision ? current : next;
}

type UpdateAction = 'check' | 'download' | 'cancel' | 'install' | 'restart';
const Context = createContext<{
  status: UpdateStatus | null;
  error: boolean;
  action: (action?: UpdateAction) => Promise<UpdateStatus | undefined>;
} | null>(null);

export function DesktopUpdateProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [error, setError] = useState(false);
  const navigate = useNavigate();
  const navigation = useRef(navigate);
  navigation.current = navigate;
  const accept = (value: UpdateStatus) => setStatus((current) => newerUpdate(current, value));
  const action = useCallback(async (action?: UpdateAction) => {
    setError(false);
    try {
      if (action === 'restart') {
        await invoke('restart_app');
        return;
      }
      const current = await invoke<UpdateStatus>('desktop_update', { action });
      accept(current);
      return current;
    } catch {
      setError(true);
    }
  }, []);
  useEffect(() => {
    if (!isDesktop()) return;
    let disposed = false;
    const listener = listen<UpdateStatus>('desktop-update', ({ payload }) => {
      if (!disposed) accept(payload);
    });
    // Checking is quiet; installation always needs an explicit click.
    void listener
      .then(async () => {
        if (disposed) return;
        const initial = await invoke<UpdateStatus>('desktop_update');
        if (disposed) return;
        accept(initial);
        if (initial.phase === 'idle') await action('check');
      })
      .catch(() => {
        if (!disposed) setError(true);
      });
    return () => {
      disposed = true;
      void listener.then((stop) => stop()).catch(() => {});
    };
  }, [action]);
  useEffect(() => {
    if (!isDesktop()) return;
    let disposed = false;
    const listener = listen('desktop-update-open', () => {
      if (disposed) return;
      navigation.current('/settings#maintenance');
      void action().then((current) => {
        if (
          !disposed &&
          current &&
          (current.phase === 'idle' ||
            current.phase === 'current' ||
            (current.phase === 'failed' && current.error === 'check_failed'))
        )
          void action('check');
      });
    });
    void listener.catch(() => {
      if (!disposed) setError(true);
    });
    return () => {
      disposed = true;
      void listener.then((stop) => stop()).catch(() => {});
    };
  }, [action]);
  return <Context value={{ status, error, action }}>{children}</Context>;
}

function useUpdates() {
  const value = useContext(Context);
  if (!value) throw new Error('DesktopUpdateProvider is missing');
  return value;
}

export function DesktopUpdateLink() {
  const t = useT();
  const { status } = useUpdates();
  return (
    status?.version && (
      <div
        className="desktop-update-link flex min-w-0 flex-1 justify-end px-4 py-2 lg:justify-start lg:p-0"
        aria-live="polite"
      >
        <Link
          to="/settings#maintenance"
          className="flex min-h-9 max-w-full items-center rounded px-1 text-xs text-muted hover:text-text max-lg:min-h-11"
          title={t('gui.desktop.update_available', { version: status.version })}
        >
          <span className="truncate">
            {t('gui.desktop.update_link', { version: status.version })}
          </span>
        </Link>
      </div>
    )
  );
}

export function DesktopUpdates() {
  const t = useT();
  const { status, error, action } = useUpdates();
  const busy =
    status?.phase === 'checking' ||
    status?.phase === 'downloading' ||
    status?.phase === 'installing';
  const failure = error || status?.error;
  const downloadTotal = status?.total && status.total > 0 ? status.total : null;
  return (
    <div className="space-y-3 text-[13px]" aria-label={t('gui.desktop.updates')} role="region">
      <UpdateCheck
        version={status?.current_version}
        checking={status?.phase === 'checking'}
        current={!failure && status?.phase === 'current'}
        disabled={busy || status?.phase === 'ready' || status?.restart_required}
        onCheck={() => void action('check')}
      >
        <p role={failure ? 'alert' : 'status'} className="min-w-0 text-muted">
          {failure
            ? t(error ? 'gui.desktop.error' : `gui.desktop.${status?.error}`)
            : status &&
              status.phase !== 'idle' &&
              t(`gui.desktop.update_${status.phase}`, { version: status.version ?? '' })}
        </p>
      </UpdateCheck>
      {status?.version && (
        // Reserve the bar and byte label so download changes do not move the controls below.
        <div className="min-h-7.5 max-w-sm">
          {status.phase === 'downloading' && (
            <>
              <div className="h-1.5">
                {downloadTotal !== null && (
                  <ProgressBar
                    current={status.downloaded}
                    total={downloadTotal}
                    label={t('gui.desktop.update_downloading', { version: status.version })}
                  />
                )}
              </div>
              <p className="mt-2 text-xs text-muted tabular-nums">
                {downloadTotal !== null
                  ? t('gui.desktop.download_progress', {
                      downloaded: (status.downloaded / 1_000_000).toFixed(1),
                      total: (downloadTotal / 1_000_000).toFixed(1),
                      percent: Math.min(100, Math.floor((status.downloaded / downloadTotal) * 100)),
                    })
                  : t('gui.desktop.download_received', {
                      downloaded: (status.downloaded / 1_000_000).toFixed(1),
                    })}
              </p>
            </>
          )}
        </div>
      )}
      {status?.version && (
        <div className="flex flex-wrap items-center gap-3">
          {status.restart_required ? (
            <Button primary onClick={() => void action('restart')}>
              {t('gui.desktop.restart')}
            </Button>
          ) : status.phase === 'ready' ? (
            <Button primary onClick={() => void action('install')}>
              {t('gui.desktop.install')}
            </Button>
          ) : status.phase === 'available' ||
            (status.phase === 'failed' && status.error === 'download_failed') ? (
            <Button primary onClick={() => void action('download')}>
              <Icon path={mdiDownload} className="mdi-icon" />
              {t('gui.desktop.download')}
            </Button>
          ) : null}
          {status.phase === 'downloading' && (
            <Button onClick={() => void action('cancel')}>{t('cancel')}</Button>
          )}
          {status.version && (
            <a
              className="text-link"
              href={`https://github.com/ohne-b/twitch-drops-miner/releases/tag/v${encodeURIComponent(status.version)}`}
              target="_blank"
              rel="noreferrer"
            >
              {t('release_notes')}
            </a>
          )}
        </div>
      )}
    </div>
  );
}
