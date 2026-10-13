import { useEffect, useRef } from 'react';
import { Icon } from '@mdi/react';
import { mdiClose, mdiOpenInNew, mdiReload } from '@mdi/js';
import { useLocation, useNavigate, useSearchParams } from 'react-router';
import { useMiner } from '../../app/MinerProvider';
import type { Campaign } from '../../shared/lib/types';
import type { HistoryCampaign } from './History';
import { safeUrl } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import { Art, Empty, IconButton, Notice, ProgressBar, dateTime } from '../../shared/ui/index';

export function CampaignDetail({
  campaign: liveCampaign,
  history,
  historyOnly,
  loading,
  historyError,
  retryHistory,
}: {
  campaign?: Campaign;
  history?: HistoryCampaign;
  historyOnly: boolean;
  loading: boolean;
  historyError: boolean;
  retryHistory: () => void;
}) {
  const campaign = historyOnly ? undefined : liveCampaign;
  const priority = campaign && !campaign.finished && !campaign.expired ? campaign.priority : null;
  const { data, connected } = useMiner();
  const mining =
    connected &&
    data?.login.user_id &&
    !data.settings.mining_paused &&
    ['watching', 'awaiting_progress', 'manual_watching'].includes(data.mining?.state ?? '')
      ? data.mining
      : undefined;
  const t = useT();
  const [params, setParams] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const heading = useRef<HTMLHeadingElement>(null);
  const id = params.get('campaign') ?? '';
  const triggerId = location.state?.campaignTrigger || `campaign-open-${id}`;
  const dropId = params.get('drop');
  const selectedLiveDrop = !!campaign?.drops.some((drop) => drop.id === dropId);
  const selectedHistoryDrop = !campaign && !!history?.entries.some((entry) => entry.id === dropId);
  const dropAnchor =
    dropId &&
    (selectedLiveDrop ? `drop-${dropId}` : selectedHistoryDrop ? `history-drop-${dropId}` : null);
  const close = () => {
    if (location.state?.campaignDetail) navigate(-1);
    else {
      const next = new URLSearchParams(params);
      next.delete('campaign');
      next.delete('drop');
      setParams(next, { replace: true });
    }
  };
  useEffect(() => {
    const top = window.scrollY;
    heading.current?.focus({ preventScroll: true });
    return () => {
      window.scrollTo(0, top);
      document.getElementById(triggerId)?.focus({ preventScroll: true });
    };
  }, [id, triggerId]);
  useEffect(() => {
    if (dropAnchor) document.getElementById(dropAnchor)?.scrollIntoView({ block: 'nearest' });
  }, [dropAnchor]);
  function selectDrop(id: string) {
    const next = new URLSearchParams(params);
    next.set('drop', id);
    setParams(next, { replace: true, state: location.state });
  }
  const title =
    campaign?.name ??
    history?.name ??
    t(historyOnly ? 'gui.tabs.history' : loading ? 'loading' : 'campaign_missing');
  return (
    <aside
      id="campaign-details"
      className="campaign-detail panel"
      aria-label={t('campaign_details')}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation();
          close();
        }
      }}
    >
      <header className="flex shrink-0 items-start gap-3 border-b border-divider p-5">
        <Art
          url={
            campaign?.game_box_art_url ||
            history?.metadata?.game_box_art_url ||
            history?.entries[0]?.image_url
          }
          className="size-12"
          fit={
            campaign?.game_box_art_url || history?.metadata?.game_box_art_url ? 'cover' : 'contain'
          }
        />
        <div className="min-w-0 flex-1 wrap-anywhere">
          <h2 ref={heading} tabIndex={-1} className="text-lg font-semibold">
            {title}
          </h2>
          <p className="muted">{campaign?.game_name ?? history?.game}</p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {safeUrl(campaign?.campaign_url) && (
            <a
              className="icon-button"
              aria-label={t('open_twitch_campaign')}
              title={t('open_twitch_campaign')}
              href={safeUrl(campaign?.campaign_url)}
              target="_blank"
              rel="noreferrer"
            >
              <Icon path={mdiOpenInNew} className="mdi-icon" />
            </a>
          )}
          <IconButton path={mdiClose} label={t('close_details')} onClick={close} />
        </div>
      </header>
      <div
        role="region"
        aria-label={title}
        tabIndex={0}
        className="detail-body p-5 focus-visible:bg-field"
      >
        {dropId &&
          (campaign || history) &&
          !loading &&
          !historyError &&
          !selectedLiveDrop &&
          !selectedHistoryDrop && (
            <p role="status" className="muted mb-4">
              {t(historyOnly ? 'history_drop_missing' : 'drop_missing')}
            </p>
          )}
        {historyError &&
          (historyOnly ||
            !campaign ||
            campaign.drops.some((drop) => drop.is_claimed) ||
            (dropId && !selectedLiveDrop)) && (
            <Notice error>
              {t('history_error')}
              <IconButton path={mdiReload} label={t('retry')} onClick={retryHistory} />
            </Notice>
          )}
        {!campaign && !history && !historyError && (
          <Empty
            title={t(
              loading ? 'loading' : historyOnly ? 'history_campaign_missing' : 'campaign_missing',
            )}
            detail={loading || historyOnly ? undefined : t('campaign_missing_help')}
          />
        )}
        {campaign && (
          <>
            <div className="space-y-3 text-[13px] pb-5">
              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-muted">
                <p>
                  {dateTime(campaign.starts_at)} — {dateTime(campaign.ends_at)}
                </p>
                {priority && (
                  <span title={priority.deadline ? dateTime(priority.deadline) : undefined}>
                    {t(`reason_${priority.reason}`)}
                  </span>
                )}
              </div>
              <p>
                {campaign.claimed_drops} / {campaign.total_drops} {t('gui.inventory.claimed_drops')}
              </p>
              <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <span className="muted">
                  {t(
                    campaign.linked === true
                      ? 'account_linked'
                      : campaign.linked === false
                        ? 'not_linked_detail'
                        : 'link_unknown',
                  )}
                </span>
                {campaign.linked !== true && safeUrl(campaign.link_url) && (
                  <a
                    className="text-link"
                    aria-label={t(campaign.linked === null ? 'check_account_link' : 'link_account')}
                    href={safeUrl(campaign.link_url)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {t(campaign.linked === null ? 'check_account_link' : 'connect')}
                  </a>
                )}
              </div>
            </div>
            <div className="divide-y divide-divider">
              {campaign.drops.map((drop) => {
                const claim = history?.entries.find((entry) => entry.id === drop.id);
                const benefits = drop.benefits.filter((benefit) => benefit.name !== drop.name);
                const eligibility = drop.is_claimed ? 'claimed' : (drop.eligibility ?? 'unknown');
                const status =
                  eligibility === 'ready'
                    ? mining?.campaign_id === campaign.id && mining.drop_id === drop.id
                      ? 'mining_state_watching'
                      : undefined
                    : `eligibility_${eligibility}`;
                return (
                  <section
                    id={`drop-${drop.id}`}
                    aria-current={drop.id === dropId ? 'true' : undefined}
                    key={drop.id}
                    className={`reward-detail py-5 ${drop.id === dropId ? 'selected' : ''}`}
                  >
                    <div className="flex items-start gap-3">
                      <Art url={drop.benefits[0]?.image_url} className="size-12" fit="contain" />
                      <div className="min-w-0 flex-1">
                        <h3 className="font-medium">{drop.name}</h3>
                        {status && (
                          <p className="muted mt-1">
                            {drop.is_claimed &&
                              claim?.claimed_at_is_observed &&
                              `${t('eligibility_claimed')}: `}
                            {drop.is_claimed && claim
                              ? t(claim.claimed_at_is_observed ? 'first_observed' : 'claimed_at', {
                                  time: dateTime(claim.claimed_at),
                                })
                              : t(status)}
                          </p>
                        )}
                      </div>
                    </div>
                    {!!benefits.length && (
                      <p className="muted mt-3">
                        {benefits.map((benefit) => benefit.name).join(', ')}
                      </p>
                    )}
                    {!drop.is_claimed && drop.eligibility !== 'awaiting_claim' && (
                      <div className="mt-3 space-y-2">
                        <ProgressBar
                          current={drop.confirmed_at ? (drop.confirmed_minutes ?? 0) : 0}
                          total={drop.required_minutes}
                          label={drop.name}
                          description={drop.confirmed_at ? undefined : t('progress_unknown')}
                        />
                        <p
                          className="muted tabular-nums"
                          title={drop.confirmed_at ? undefined : t('progress_unknown')}
                          aria-description={drop.confirmed_at ? undefined : t('progress_unknown')}
                        >
                          {t('minutes_progress', {
                            current: drop.confirmed_at ? (drop.confirmed_minutes ?? 0) : 0,
                            total: drop.required_minutes,
                          })}
                        </p>
                      </div>
                    )}
                    {(Date.parse(drop.effective_starts_at ?? drop.starts_at) !==
                      Date.parse(campaign.starts_at) ||
                      Date.parse(drop.effective_ends_at ?? drop.ends_at) !==
                        Date.parse(campaign.ends_at)) && (
                      <p className="muted mt-3 text-xs">
                        {dateTime(drop.effective_starts_at ?? drop.starts_at)} —{' '}
                        {dateTime(drop.effective_ends_at ?? drop.ends_at)}
                      </p>
                    )}
                    {!!drop.prerequisites?.length && (
                      <div className="mt-3">
                        <p className="muted text-xs">{t('requires_claims')}</p>
                        <div className="flex flex-wrap gap-2">
                          {drop.prerequisites.map((id) => {
                            const prerequisite = campaign.drops.find((item) => item.id === id);
                            return prerequisite ? (
                              <button
                                type="button"
                                className="text-link text-[13px]"
                                key={id}
                                onClick={() => selectDrop(id)}
                              >
                                {prerequisite.name}
                                {prerequisite.is_claimed
                                  ? ` (${t('gui.inventory.status.claimed')})`
                                  : ''}
                              </button>
                            ) : (
                              <span className="muted" key={id}>
                                {t('prerequisite_missing')}
                              </span>
                            );
                          })}
                        </div>
                      </div>
                    )}
                    {drop.ignored_keyword && (
                      <p className="muted mt-2">
                        {t('gui.inventory.ignored_keyword_reason', {
                          keyword: drop.ignored_keyword,
                        })}
                      </p>
                    )}
                    {drop.ignored_precondition && (
                      <p className="muted mt-2">
                        {t('gui.inventory.ignored_precondition_reason', {
                          drop: drop.ignored_precondition,
                        })}
                      </p>
                    )}
                  </section>
                );
              })}
            </div>
          </>
        )}
        {!campaign && history && (
          <section>
            <h3 className="section-title mb-3">
              {t(historyOnly ? 'recorded_claims' : 'gui.tabs.history', {
                count: history.entries.length,
              })}
            </h3>
            {history.entries.map((entry) => (
              <div
                id={`history-drop-${entry.id}`}
                aria-current={entry.id === dropId ? 'true' : undefined}
                key={entry.id}
                className={`reward-detail flex gap-3 py-3 ${entry.id === dropId ? 'selected' : ''}`}
              >
                <Art
                  fit="contain"
                  url={
                    entry.image_url ||
                    history.metadata?.drops.find((drop) => drop.id === entry.id)?.benefits[0]
                      ?.image_url
                  }
                />
                <div>
                  <p className="font-medium">{entry.drop_name}</p>
                  <p className="muted">
                    {entry.benefits.filter((name) => name !== entry.drop_name).join(', ')}
                  </p>
                  <p className="muted mt-1">
                    {t(entry.claimed_at_is_observed ? 'first_observed' : 'claimed_at', {
                      time: dateTime(entry.claimed_at),
                    })}
                  </p>
                </div>
              </div>
            ))}
          </section>
        )}
      </div>
    </aside>
  );
}
