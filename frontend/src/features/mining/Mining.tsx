import MiningPreferences from './MiningPreferences';
import SearchPicker from '../../shared/ui/SearchPicker';
import { useDirectorySearch } from './useDirectorySearch';
import type { ChannelSearchResult } from '../../shared/lib/types';
import { CampaignLink } from '../campaigns/CampaignLink';
import { Icon } from '@mdi/react';
import { useEffect, useRef, useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import {
  mdiPencil,
  mdiReload,
  mdiRefreshAuto,
  mdiPlayCircleOutline,
  mdiPlus,
  mdiPause,
  mdiPlay,
  mdiChevronLeft,
  mdiChevronRight,
} from '@mdi/js';
import { InventoryRefreshButton } from '../../shared/ui/InventoryRefreshButton';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../../shared/lib/i18n';
import { request } from '../../shared/lib/api';
import { confirmedPercent } from '../../shared/lib/progress';
import {
  Art,
  IconButton,
  Empty,
  ProgressBar,
  Search,
  Notice,
  useAction,
  ActionResult,
  dateTime,
  ConfirmedTime,
  Input,
} from '../../shared/ui/index';
export default function Mining() {
  const { data, connected, autosave } = useMiner();
  const t = useT();
  const [params, setParams] = useSearchParams();
  const edit = params.get('edit') === 'priorities';
  const editLink = useRef<HTMLAnchorElement>(null);
  const previousEdit = useRef(edit);
  useEffect(() => {
    if (previousEdit.current && !edit) editLink.current?.focus({ preventScroll: true });
    previousEdit.current = edit;
  }, [edit]);
  const search = params.get('q') ?? '';
  const channelList = useRef<HTMLDivElement>(null);
  function setSearch(value: string) {
    const next = new URLSearchParams(params);
    value ? next.set('q', value) : next.delete('q');
    next.delete('channels_page');
    channelList.current?.scrollTo({ top: 0 });
    setParams(next, { replace: true });
  }
  function setChannelPage(page: number) {
    const next = new URLSearchParams(params);
    page ? next.set('channels_page', String(page)) : next.delete('channels_page');
    channelList.current?.scrollTo({ top: 0 });
    setParams(next, { replace: true });
  }
  const [channelInput, setChannelInput] = useState('');
  const [manualMinutes, setManualMinutes] = useState('');
  const [enterChannel, setEnterChannel] = useState(false);
  const action = useAction();
  const searchTerm = channelInput
    .trim()
    .replace(/^https?:\/\/(?:www\.|m\.)?twitch\.tv\/([a-z0-9_]{1,25})\/?$/i, '$1')
    .replace(/^@/, '');
  const directory = useDirectorySearch<ChannelSearchResult>(
    '/api/channels/search',
    searchTerm.length <= 100 ? searchTerm : '',
    connected && enterChannel && !edit,
    data?.login.user_id,
  );
  const manualForm = useRef<HTMLFormElement>(null);
  function mineChannel(channel: string) {
    if (!manualForm.current?.reportValidity()) return;
    void action.run(() =>
      request('/api/channels/select', {
        channel,
        duration_minutes: manualMinutes ? Number(manualMinutes) : null,
      }),
    );
  }
  if (!data) return <Empty title={t('loading')} />;
  if (edit) return <MiningPreferences />;
  const progress = data.current_drop;
  const percent = (autosave.draft ?? data.settings).show_progress_percentage
    ? confirmedPercent(progress)
    : null;
  const resuming = !data.settings.mining_paused && data.mining?.state === 'paused';
  const paused = data.settings.mining_paused || resuming;
  const pauseBusy = autosave.pending || autosave.busy || resuming;
  const campaign = data.campaigns.find((item) => item.id === progress?.campaign_id);
  const reward = campaign?.drops.find((drop) => drop.id === progress?.drop_id);
  const watching = data.channels.find((channel) => channel.watching);
  const watchingLabel = watching && (
    <span>
      {t('watching_now')}{' '}
      <a
        className="hover:underline"
        href={`https://www.twitch.tv/${encodeURIComponent(watching.login || watching.name)}`}
        target="_blank"
        rel="noreferrer"
      >
        {watching.name}
      </a>
    </span>
  );
  const manualChannel = data.channels.find((channel) => channel.id === data.manual_mode.channel_id);
  const channels = data.channels
    .filter((channel) =>
      `${channel.name} ${channel.game ?? ''}`
        .toLocaleLowerCase()
        .includes(search.toLocaleLowerCase()),
    )
    .sort(
      (a, b) => Number(b.watching) - Number(a.watching) || (b.viewers ?? -1) - (a.viewers ?? -1),
    );
  const channelPages = Math.max(1, Math.ceil(channels.length / 25));
  const requestedPage = Number(params.get('channels_page') ?? 0);
  const channelPage = Number.isSafeInteger(requestedPage)
    ? Math.max(0, Math.min(requestedPage, channelPages - 1))
    : 0;
  return (
    <div className="mining-workspace flex flex-col gap-5 xl:min-h-0 xl:flex-1">
      <header className="flex shrink-0 flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <h1 className="text-[22px] font-semibold">{t('mining')}</h1>
        <InventoryRefreshButton />
      </header>
      <ActionResult action={action} />
      <section className="panel shrink-0 p-4 md:px-5 md:py-6" aria-labelledby="mining-heading">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 md:mb-5">
          <h2 id="mining-heading" className="section-title">
            {t(paused ? 'paused' : 'now_mining')}
          </h2>
          <div className="ms-auto flex items-center gap-3">
            {progress?.confirmed_at && (
              <ConfirmedTime
                value={progress.confirmed_at}
                className="text-right text-xs text-[#888888]"
              />
            )}
            {data.manual_mode.active ? (
              <span className="muted">{t('manual')}</span>
            ) : (
              <span
                role="img"
                aria-label={t('automatic')}
                title={t('automatic')}
                className="text-muted"
              >
                <Icon className="mdi-icon" path={mdiRefreshAuto} />
              </span>
            )}
            <IconButton
              path={paused ? mdiPlay : mdiPause}
              label={t(paused ? 'resume_mining' : 'pause_mining')}
              disabled={!connected || !data.login.user_id || pauseBusy}
              aria-busy={pauseBusy}
              onClick={() => autosave.change('mining_paused', !paused)}
            />
          </div>
        </div>
        {progress ? (
          <>
            <div className="flex items-center gap-3 md:gap-4">
              <Art
                url={reward?.benefits[0]?.image_url || campaign?.game_box_art_url}
                className="size-22"
                fit={reward?.benefits[0]?.image_url ? 'contain' : 'cover'}
              />
              <div className="min-w-0 flex-1">
                <CampaignLink
                  id="mining-drop-details"
                  className="text-lg font-semibold hover:underline"
                  to={`/campaigns?campaign=${encodeURIComponent(progress.campaign_id)}&drop=${encodeURIComponent(progress.drop_id)}`}
                >
                  {progress.drop_name}
                </CampaignLink>
                <p className="muted mt-1">
                  {progress.game_name} / {progress.campaign_name}
                </p>
                {watching && !paused && (
                  <p className="muted mt-2 flex items-center gap-2">
                    <Art url={watching.avatar_url} className="size-8! rounded-full" />
                    {watchingLabel}
                  </p>
                )}
              </div>
            </div>
            <div className="mt-4 md:mt-5">
              {!paused &&
                data.mining?.state !== 'watching' &&
                data.mining?.state !== 'awaiting_progress' && (
                  <p className="muted mb-3">
                    {t(
                      data.mining && data.mining.state !== 'unknown'
                        ? `mining_state_${data.mining.state}`
                        : `eligibility_${reward?.eligibility ?? 'unknown'}`,
                    )}
                  </p>
                )}
              <ProgressBar
                current={progress.confirmed_at ? (progress.confirmed_minutes ?? 0) : 0}
                total={progress.required_minutes}
                label={progress.drop_name}
                description={progress.confirmed_at ? undefined : t('progress_unknown')}
              />
              <div className="mt-2 flex flex-wrap justify-between gap-2 text-[13px]">
                <span
                  className="tabular-nums"
                  title={progress.confirmed_at ? undefined : t('progress_unknown')}
                  aria-description={progress.confirmed_at ? undefined : t('progress_unknown')}
                >
                  {t('minutes_progress', {
                    current: progress.confirmed_at ? (progress.confirmed_minutes ?? 0) : 0,
                    total: progress.required_minutes,
                  })}
                  {percent !== null && ` (${percent}%)`}
                </span>
              </div>
            </div>
          </>
        ) : paused ? (
          <Empty className="py-5" title={t('paused_help')} />
        ) : data.manual_mode.active ? (
          <div className="flex items-center gap-3">
            <Art url={(watching ?? manualChannel)?.avatar_url} className="size-8! rounded-full" />
            <p className="font-medium">
              {watching
                ? watchingLabel
                : t('gui.channels.waiting_for_live', {
                    channel: data.manual_mode.channel_name ?? '',
                  })}
            </p>
          </div>
        ) : (
          <Empty
            className="py-5"
            title={t(
              data.mining?.state === 'watching'
                ? 'progress_unknown'
                : data.mining && data.mining.state !== 'unknown'
                  ? `mining_state_${data.mining.state}`
                  : 'gui.progress.no_drop',
            )}
            detail={t(
              !data.login.user_id
                ? 'connect_help'
                : data.settings.games_to_watch.length ||
                    data.settings.auto_mine_badges ||
                    data.settings.auto_mine_emotes
                  ? 'waiting_help'
                  : 'select_games_help',
            )}
          >
            <Link
              className="button"
              to={data.login.user_id ? '/?edit=priorities' : '/settings#account'}
            >
              {data.login.user_id ? t('choose_games') : t('account')}
            </Link>
          </Empty>
        )}
        {(data.manual_mode.active ||
          data.manual_mode.pending_channel ||
          data.manual_mode.expires_at) && (
          <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2">
            {(data.manual_mode.active || data.manual_mode.pending_channel) && (
              <IconButton
                path={mdiRefreshAuto}
                label={t('gui.progress.return_to_auto')}
                disabled={!connected || action.busy}
                onClick={() => void action.run(() => request('/api/mode/exit-manual', {}))}
              />
            )}
            {data.manual_mode.expires_at && (
              <p className="muted">
                {t('gui.channels.auto_at', { time: dateTime(data.manual_mode.expires_at) })}
              </p>
            )}
          </div>
        )}
      </section>
      {/* Long lists must not contribute to the page's intrinsic minimum height. */}
      <div className="grid gap-5 xl:min-h-[260px] xl:grid-cols-2 xl:flex-1 xl:[contain:size]">
        <section className="panel order-2 flex min-h-0 flex-col overflow-hidden xl:order-1">
          <div className="shrink-0 space-y-4 border-b border-divider p-4">
            <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
              <h2 id="channels-heading" className="section-title">
                {t('gui.channels.name')}
              </h2>
              <div className="flex items-center gap-3">
                <span className="muted tabular-nums">{channels.length}</span>
                {channelPages > 1 && (
                  <nav className="flex items-center gap-1" aria-label={t('channel_pages')}>
                    <IconButton
                      path={mdiChevronLeft}
                      label={t('gui.history.previous')}
                      disabled={channelPage === 0}
                      onClick={() => setChannelPage(channelPage - 1)}
                    />
                    <span className="muted whitespace-nowrap tabular-nums">
                      {channelPage + 1} / {channelPages}
                    </span>
                    <IconButton
                      path={mdiChevronRight}
                      label={t('gui.history.next')}
                      disabled={channelPage + 1 >= channelPages}
                      onClick={() => setChannelPage(channelPage + 1)}
                    />
                  </nav>
                )}
                <IconButton
                  path={mdiPlus}
                  label={t('gui.channels.mine_channel')}
                  aria-expanded={enterChannel}
                  onClick={() => setEnterChannel(!enterChannel)}
                />
              </div>
            </div>
            <Search value={search} onChange={setSearch} label={t('search_channels')} />
          </div>
          <div
            id="channels-list"
            ref={channelList}
            data-restore-scroll
            className="scroll-list min-h-0 max-h-[440px] overflow-y-auto focus-visible:bg-field xl:max-h-none xl:flex-1"
            role="region"
            aria-labelledby="channels-heading"
            tabIndex={0}
          >
            {(enterChannel || data.manual_mode.error) && (
              <div className="mx-4 space-y-3 border-b border-divider py-4">
                {enterChannel && (
                  <form
                    ref={manualForm}
                    className="space-y-2"
                    onSubmit={(event) => {
                      event.preventDefault();
                      mineChannel(channelInput);
                    }}
                  >
                    <SearchPicker
                      label={t('gui.channels.channel_input')}
                      actionLabel={t('mine')}
                      actionIcon={mdiPlayCircleOutline}
                      submit
                      busy={action.busy || !!data.manual_mode.pending_channel}
                      roundImages
                      maxLength={256}
                      value={channelInput}
                      onChange={setChannelInput}
                      options={directory.items.map((channel) => ({
                        id: channel.login,
                        name: channel.name,
                        image: channel.avatar_url,
                        description: `@${channel.login}, ${t(channel.online ? 'gui.channels.live' : 'gui.channels.offline')}${channel.online && channel.game ? `, ${channel.game}` : ''}`,
                      }))}
                      onSelect={(channel) => {
                        setChannelInput(channel.id);
                        mineChannel(channel.id);
                      }}
                      onResolve={() => mineChannel(channelInput)}
                      disabled={
                        !connected ||
                        !data.login.user_id ||
                        action.busy ||
                        !!data.manual_mode.pending_channel
                      }
                      loading={directory.loading}
                      ready={directory.complete || directory.loading}
                    >
                      {directory.loading && (
                        <p role="status" className="muted px-3 py-2">
                          {t('loading')}
                        </p>
                      )}
                      {directory.complete && !directory.error && !directory.items.length && (
                        <p role="status" className="muted px-3 py-2">
                          {t('gui.channels.search_empty')}
                        </p>
                      )}
                      {directory.error && (
                        <div
                          role="status"
                          className="flex items-center gap-2 px-3 py-2 text-[13px] text-muted"
                        >
                          <span>{t('gui.channels.search_failed')}</span>
                          <IconButton
                            path={mdiReload}
                            label={t('retry')}
                            onClick={directory.retry}
                          />
                        </div>
                      )}
                    </SearchPicker>
                    <Input
                      type="number"
                      min={1}
                      max={1440}
                      step={1}
                      aria-label={t('gui.channels.manual_timer')}
                      placeholder={t('gui.channels.manual_timer')}
                      value={manualMinutes}
                      onChange={(event) => setManualMinutes(event.target.value)}
                    />
                  </form>
                )}
                {data.manual_mode.error && <Notice error>{data.manual_mode.error}</Notice>}
              </div>
            )}
            {channels.slice(channelPage * 25, (channelPage + 1) * 25).map((channel) => (
              <div className="row flex-wrap sm:flex-nowrap" key={channel.id}>
                <Art
                  url={channel.avatar_url ?? channel.game_icon}
                  className={channel.avatar_url ? 'rounded-full' : ''}
                />
                <div className="min-w-0 flex-1">
                  <a
                    className="font-medium hover:underline"
                    href={`https://www.twitch.tv/${encodeURIComponent(channel.login || channel.name)}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {channel.name}
                  </a>
                  <p className="muted truncate">
                    {channel.online ? (
                      <>
                        {channel.game ?? t('unknown_game')},{' '}
                        {channel.viewers?.toLocaleString() ?? '—'} {t('gui.channels.viewers')}
                      </>
                    ) : (
                      t('gui.channels.offline')
                    )}
                  </p>
                </div>
                {channel.watching ? (
                  <span className="muted">{t('watching_now')}</span>
                ) : (
                  <IconButton
                    path={mdiPlayCircleOutline}
                    label={t('watch_channel', { channel: channel.name })}
                    disabled={!connected || !data.login.user_id || action.busy}
                    onClick={() =>
                      void action.run(() =>
                        request('/api/channels/select', { channel_id: channel.id }),
                      )
                    }
                  />
                )}
              </div>
            ))}
            {!channels.length && (
              <Empty title={t(search ? 'no_matches' : 'gui.channels.no_channels')} />
            )}
          </div>
        </section>
        <section className="panel order-1 flex min-h-0 flex-col overflow-hidden xl:order-2">
          <div className="flex shrink-0 items-center justify-between border-b border-divider p-4">
            <h2 id="up-next-heading" className="section-title">
              {t('up_next')}
            </h2>
            <Link
              ref={editLink}
              className="icon-button"
              aria-label={t('edit')}
              title={t('edit')}
              to="/?edit=priorities"
            >
              <Icon className="mdi-icon" path={mdiPencil} />
            </Link>
          </div>
          <div
            id="up-next-list"
            data-restore-scroll
            className="scroll-list min-h-0 max-h-[440px] overflow-y-auto focus-visible:bg-field xl:max-h-none xl:flex-1"
            role="region"
            aria-labelledby="up-next-heading"
            tabIndex={0}
          >
            {data.wanted_items.map((game, index) => (
              <div key={game.game_name} className="mx-4 border-b border-divider py-4 last:border-0">
                <div className="flex items-center gap-3">
                  <span className="w-4 text-[13px] tabular-nums text-muted">{index + 1}</span>
                  <Art url={game.game_icon} className="size-8" />
                  <p className="font-medium">{game.game_name}</p>
                  <span className="muted ms-auto text-xs">
                    {game.saved_rank
                      ? t('saved_position', { count: game.saved_rank })
                      : t('automatic')}
                  </span>
                </div>
                {game.campaigns.map((item) => {
                  const dates = item.drops.map((drop) => {
                    const upcoming = drop.eligibility === 'upcoming';
                    const time = upcoming ? drop.starts_at : drop.ends_at;
                    return {
                      start: Date.parse(drop.starts_at ?? ''),
                      end: Date.parse(drop.ends_at ?? ''),
                      upcoming,
                      text: time
                        ? t(upcoming ? 'gui.inventory.starts' : 'gui.inventory.ends', {
                            time: dateTime(time),
                          })
                        : '',
                    };
                  });
                  const sharedDate =
                    dates.length > 1 &&
                    dates.every(
                      (date) =>
                        date.start === dates[0]?.start &&
                        date.end === dates[0]?.end &&
                        date.upcoming === dates[0]?.upcoming,
                    );
                  return (
                    <div className="mt-3 ps-7 text-[13px]" key={item.id}>
                      <CampaignLink
                        id={`up-next-campaign-${item.id}`}
                        className="text-link"
                        to={`/campaigns?campaign=${encodeURIComponent(item.id)}`}
                      >
                        {item.name}
                      </CampaignLink>
                      {sharedDate && <p className="muted mt-1 text-xs">{dates[0]?.text}</p>}
                      {item.priority && item.priority.reason !== 'saved_order' && (
                        <p className="muted mt-1">
                          {t(`reason_${item.priority.reason}`)}
                          {item.priority.deadline && `: ${dateTime(item.priority.deadline)}`}
                        </p>
                      )}
                      <ul className="mt-2 space-y-2 text-muted">
                        {item.drops.map((drop, position) => (
                          <li
                            className="flex items-start gap-3"
                            key={drop.id || `${drop.name}/${position}`}
                          >
                            <Art url={drop.image_url} className="size-9" fit="contain" />
                            <div className="min-w-0 flex-1">
                              <CampaignLink
                                id={`up-next-drop-${item.id}-${drop.id || position}`}
                                className="hover:underline text-soft"
                                to={`/campaigns?campaign=${encodeURIComponent(item.id)}${drop.id ? `&drop=${encodeURIComponent(drop.id)}` : ''}`}
                              >
                                {drop.name}
                              </CampaignLink>
                              {drop.eligibility && drop.eligibility !== 'ready' && (
                                <p className="text-xs mt-1">
                                  {t(`eligibility_${drop.eligibility}`)}
                                </p>
                              )}
                              {!sharedDate && dates[position]?.text && (
                                <p className="text-xs mt-1">{dates[position]?.text}</p>
                              )}
                              {drop.benefits.some((benefit) => benefit !== drop.name) && (
                                <p className="mt-0.5 text-xs">
                                  {drop.benefits
                                    .filter((benefit) => benefit !== drop.name)
                                    .join(', ')}
                                </p>
                              )}
                            </div>
                          </li>
                        ))}
                      </ul>
                    </div>
                  );
                })}
              </div>
            ))}
            {!data.wanted_items.length && <Empty title={t('gui.wanted.none')} />}
          </div>
        </section>
      </div>
    </div>
  );
}
