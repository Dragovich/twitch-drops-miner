import { useEffect, useState } from 'react';
import {
  mdiChevronLeft,
  mdiChevronRight,
  mdiPlayCircleOutline,
  mdiRefreshAuto,
  mdiReload,
} from '@mdi/js';
import { useMiner } from '../../app/MinerProvider';
import { request } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import type { Campaign } from '../../shared/lib/types';
import {
  ActionResult,
  HelpButton,
  IconButton,
  Notice,
  dateTime,
  useAction,
} from '../../shared/ui/index';

interface Participant {
  id: number;
  login: string;
  name: string;
  online: boolean | null;
  eligible: boolean | null;
  game: string | null;
  checked_at: string;
}
interface Page {
  campaign_id: string;
  offset: number;
  total: number;
  channels: Participant[];
}

export function CampaignChannels({ campaign }: { campaign: Campaign }) {
  const { data, connected } = useMiner();
  const t = useT();
  const action = useAction();
  const [offset, setOffset] = useState(0);
  const [retry, setRetry] = useState(0);
  const [visible, setVisible] = useState(document.visibilityState !== 'hidden');
  const [result, setResult] = useState<{ scope: string; page: Page }>();
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const channels = campaign.allowed_channels ?? [];
  const start = offset < channels.length ? offset : 0;
  const scope = JSON.stringify([campaign.id, campaign.game_id, channels]);
  const account = data?.login.user_id;
  const enabled = connected && !!account && visible && !campaign.finished && !campaign.expired;
  useEffect(() => {
    const changed = () => setVisible(document.visibilityState !== 'hidden');
    document.addEventListener('visibilitychange', changed);
    return () => document.removeEventListener('visibilitychange', changed);
  }, []);
  useEffect(() => {
    setResult(undefined);
    setFailed(false);
    setLoading(false);
    if (!enabled || !channels.length) return;
    let stopped = false;
    let controller: AbortController;
    let next: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 15000);
      let delay = 30000;
      setLoading(true);
      try {
        const page = await request<Page>(
          '/api/campaigns/channels',
          { campaign_id: campaign.id, offset: start },
          'POST',
          controller.signal,
        );
        if (!stopped) {
          setResult({ scope, page });
          setFailed(false);
          const oldest = Math.min(
            ...page.channels.map((channel) => Date.parse(channel.checked_at)),
          );
          delay = Math.max(1000, Math.min(30000, oldest + 30000 - Date.now()));
        }
      } catch {
        if (!stopped) {
          setResult(undefined);
          setFailed(true);
        }
      } finally {
        clearTimeout(timeout);
        if (!stopped) {
          setLoading(false);
          next = setTimeout(() => void refresh(), delay);
        }
      }
    };
    void refresh();
    return () => {
      stopped = true;
      controller?.abort();
      clearTimeout(next);
    };
  }, [enabled, account, scope, start, retry, campaign.id, channels.length]);
  if (!channels.length) return null;
  const page = result?.scope === scope && result.page.offset === start ? result.page : undefined;
  const manual = data?.manual_mode;
  return (
    <section aria-label={t('participating_channels')} className="min-w-0">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1">
          <h3 className="font-medium">{t('participating_channels')}</h3>
          <HelpButton label={t('participating_channels')} text={t('participant_help')} />
        </div>
        <IconButton
          path={mdiReload}
          label={t('refresh_channel_status')}
          disabled={!enabled || loading}
          aria-busy={loading}
          onClick={() => setRetry((value) => value + 1)}
        />
      </div>
      {failed && (
        <p role="status" className="muted mb-2">
          {t('channel_status_failed')}
        </p>
      )}
      <ul
        className="max-h-64 overflow-y-auto divide-y divide-divider"
        tabIndex={0}
        aria-label={t('participating_channels')}
      >
        {channels.slice(start, start + 10).map((channel) => {
          const status = page?.channels.find(
            (entry) => entry.login.toLowerCase() === channel.login.toLowerCase(),
          );
          const fresh = status && Date.now() - Date.parse(status.checked_at) < 45000;
          const online = fresh ? status.online : null;
          const watching =
            manual?.active &&
            data?.channels.some((entry) => entry.watching && entry.id === status?.id);
          const pending = manual?.pending_channel?.toLowerCase() === channel.login.toLowerCase();
          const label =
            online === false
              ? 'channel_offline'
              : online === true
                ? status?.eligible === true
                  ? 'channel_live_eligible'
                  : status?.eligible === false
                    ? 'channel_live_ineligible'
                    : 'channel_live_unknown'
                : loading
                  ? 'channel_checking'
                  : 'channel_status_unknown';
          return (
            <li key={channel.login} className="flex min-w-0 items-center gap-3 py-2">
              <div className="min-w-0 flex-1">
                <a
                  className="block truncate text-link"
                  href={`https://www.twitch.tv/${encodeURIComponent(channel.login)}`}
                  target="_blank"
                  rel="noreferrer"
                  title={channel.name}
                >
                  {channel.name}
                </a>
                <p
                  className="truncate text-xs text-muted"
                  title={
                    fresh ? t('channel_checked', { time: dateTime(status.checked_at) }) : undefined
                  }
                >
                  <span className={online && status?.eligible ? 'text-accent' : ''}>
                    {t(label)}
                  </span>
                  {online && status?.game && ` · ${status.game}`}
                </p>
              </div>
              <IconButton
                path={mdiPlayCircleOutline}
                label={t(
                  watching ? 'watching' : pending ? 'participant_selecting' : 'participant_watch',
                  { channel: channel.name },
                )}
                disabled={
                  !enabled ||
                  !campaign.active ||
                  action.busy ||
                  !!manual?.pending_channel ||
                  !!watching ||
                  online !== true ||
                  status?.eligible !== true
                }
                aria-busy={pending}
                onClick={() =>
                  void action.run(() => request('/api/channels/select', { channel: channel.login }))
                }
              />
            </li>
          );
        })}
      </ul>
      {channels.length > 10 && (
        <div className="flex items-center justify-end gap-2 pt-1 text-xs text-muted">
          <span>
            {t('participant_page', {
              first: start + 1,
              last: Math.min(start + 10, channels.length),
              total: channels.length,
            })}
          </span>
          <IconButton
            path={mdiChevronLeft}
            label={t('previous_channels')}
            disabled={start === 0}
            onClick={() => setOffset(start - 10)}
          />
          <IconButton
            path={mdiChevronRight}
            label={t('next_channels')}
            disabled={start + 10 >= channels.length}
            onClick={() => setOffset(start + 10)}
          />
        </div>
      )}
      {(manual?.active || manual?.pending_channel) && (
        <div className="flex items-center justify-between gap-2 pt-2 text-xs text-muted">
          <span>
            {t('participant_manual', {
              channel: manual.pending_channel || manual.channel_name || '',
            })}
          </span>
          <IconButton
            path={mdiRefreshAuto}
            label={t('gui.progress.return_to_auto')}
            disabled={!connected || action.busy}
            onClick={() => void action.run(() => request('/api/mode/exit-manual', {}))}
          />
        </div>
      )}
      {data?.settings.mining_paused && <p className="muted mt-2">{t('participant_paused')}</p>}
      <ActionResult action={action} />
      {manual?.error && <Notice error>{manual.error}</Notice>}
    </section>
  );
}
