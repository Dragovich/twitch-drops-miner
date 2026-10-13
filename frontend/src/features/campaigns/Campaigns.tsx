import { CampaignDetail } from './CampaignDetail';
import FilterPopover from './FilterPopover';
import { displayFilters, writeFilters } from './query';
import { Icon } from '@mdi/react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Link, useLocation, useNavigate, useSearchParams } from 'react-router';
import {
  mdiFilterOutline,
  mdiGamepadVariantOutline,
  mdiViewList,
  mdiViewGridOutline,
  mdiSortAscending,
  mdiFilterOffOutline,
  mdiPlayCircleOutline,
  mdiStopCircleOutline,
  mdiChevronLeft,
  mdiChevronRight,
  mdiReload,
  mdiGiftOutline,
  mdiHistory,
} from '@mdi/js';
import { useMiner } from '../../app/MinerProvider';
import { InventoryRefreshButton } from '../../shared/ui/InventoryRefreshButton';
import { useT } from '../../shared/lib/i18n';
import type { Campaign as CampaignData, Filters } from '../../shared/lib/types';
import {
  Button,
  IconButton,
  Check,
  Empty,
  Search,
  useAction,
  ActionResult,
  Notice,
} from '../../shared/ui/index';
import { Campaign } from './Campaign';
import History, { useHistory, groupHistory, matchesHistory, historyOrder } from './History';
export function matchesCampaign(
  campaign: CampaignData,
  filters: Filters,
  search: string,
  selectedForMining = false,
): boolean {
  if (campaign.finished || campaign.expired) return false;
  if (filters.hide_selected_games && selectedForMining) return false;
  if (
    search &&
    !`${campaign.name} ${campaign.game_name} ${campaign.drops.map((drop) => drop.name).join(' ')}`
      .toLocaleLowerCase()
      .includes(search.toLocaleLowerCase())
  )
    return false;
  if (
    filters.game_name_search.length &&
    !filters.game_name_search.some(
      (game) => game.toLocaleLowerCase() === campaign.game_name.toLocaleLowerCase(),
    )
  )
    return false;
  if (filters.show_benefit_badge && filters.show_benefit_emote && filters.show_benefit_item)
    return true;
  const types: Record<string, boolean> = {
    BADGE: filters.show_benefit_badge,
    EMOTE: filters.show_benefit_emote,
    DIRECT_ENTITLEMENT: filters.show_benefit_item,
  };
  return campaign.drops.some(
    (drop) =>
      drop.required_minutes > 0 &&
      !drop.is_claimed &&
      drop.eligibility !== 'expired' &&
      drop.benefits.some((benefit) => types[benefit.type.toUpperCase()]),
  );
}
const campaignSorts = ['default', 'newest', 'ending', 'drops', 'name'] as const;
export type CampaignSort = (typeof campaignSorts)[number];
export function campaignOrder(
  a: CampaignData,
  b: CampaignData,
  sort: CampaignSort = 'default',
): number {
  const difference =
    sort === 'newest'
      ? Date.parse(b.starts_at) - Date.parse(a.starts_at)
      : sort === 'ending'
        ? Date.parse(a.ends_at) - Date.parse(b.ends_at)
        : sort === 'drops'
          ? b.total_drops - a.total_drops
          : sort === 'name'
            ? a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true })
            : 0;
  if (difference) return difference;
  const rank = (campaign: CampaignData) =>
    campaign.active &&
    campaign.drops.some((drop) => drop.is_claimed || (drop.confirmed_minutes ?? 0) > 0)
      ? 0
      : campaign.active
        ? 1
        : campaign.upcoming
          ? 2
          : 3;
  return rank(a) - rank(b) || a.ends_at.localeCompare(b.ends_at) || a.id.localeCompare(b.id);
}
export default function Campaigns() {
  const { data, connected, autosave } = useMiner();
  const t = useT();
  const [params, setParams] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const detailId = params.get('campaign');
  const list = params.has('view')
    ? params.get('view') === 'list'
    : ((autosave.draft ?? data?.settings)?.inventory_list_view ?? false);
  const results = useRef<HTMLDivElement>(null);
  const returnPosition = useRef<{
    top: number;
    width: number;
    list: boolean;
    id: string;
  } | null>(null);
  const hydrated = !!data;
  useLayoutEffect(() => {
    const region = results.current;
    if (!region || getComputedStyle(region).overflowY !== 'auto') return;
    if (detailId) {
      document.getElementById(`campaign-open-${detailId}`)?.scrollIntoView({ block: 'nearest' });
    } else if (returnPosition.current) {
      const { top, width, list: previousList, id } = returnPosition.current;
      if (width === region.clientWidth && previousList === list) region.scrollTop = top;
      else document.getElementById(`campaign-open-${id}`)?.scrollIntoView({ block: 'nearest' });
      returnPosition.current = null;
    }
  }, [detailId, hydrated, list]);
  const [popover, setPopover] = useState<'games' | 'filters' | null>(null);
  const [gameSearch, setGameSearch] = useState('');
  const [filterDraft, setFilterDraft] = useState<{ key: string; filters: Filters } | null>(null);
  const historyTab = ['history', 'finished'].includes(params.get('tab') ?? '');
  useEffect(() => setPopover(null), [historyTab, detailId]);
  const history = useHistory(historyTab || !!detailId);
  const action = useAction();
  if (!data) return <Empty title={t('loading')} />;
  const filters =
    filterDraft?.key === location.key
      ? filterDraft.filters
      : displayFilters(params, (autosave.draft ?? data.settings).inventory_filters);
  // Navigation can still be pending when another control edits the query.
  const currentParams =
    filterDraft?.key === location.key ? writeFilters(params, filterDraft.filters) : params;
  const selectedGames = (autosave.draft ?? data.settings).games_to_watch;
  const gameKey = (game: string) => data.settings.game_keys?.[game] ?? game.toLowerCase();
  const isSelected = (name: string) =>
    selectedGames.some((game) => gameKey(game) === gameKey(name));
  const settingsBusy = action.busy;
  const search = params.get('q') ?? '';

  const sort = campaignSorts.find((value) => value === params.get('sort')) ?? 'default';
  const groups = groupHistory(history.entries, data.campaigns);
  const available = data.campaigns.filter((campaign) => !campaign.finished && !campaign.expired);
  const total = historyTab ? groups.length : available.length;
  const campaigns = available
    .filter((campaign) =>
      matchesCampaign(campaign, filters, search, isSelected(campaign.game_name)),
    )
    .sort((a, b) => campaignOrder(a, b, sort));
  const historical = groups
    .filter((group) => matchesHistory(group, filters.game_name_search, search))
    .sort((a, b) => historyOrder(a, b, sort));
  const shown = historyTab ? historical.length : campaigns.length;
  const page = Math.min(
    Math.max(0, Math.trunc(Number(params.get('page'))) || 0),
    Math.max(0, Math.ceil((historyTab ? historical : campaigns).length / 25) - 1),
  );
  function setQuery(key: string, value: string) {
    const next = new URLSearchParams(currentParams);
    if (key !== 'page' && key !== 'campaign' && key !== 'drop') next.delete('page');
    value ? next.set(key, value) : next.delete(key);
    setParams(next, { replace: true, state: location.state });
  }
  function changeFilters(next: Filters, clearSearch = false) {
    setFilterDraft({ key: location.key, filters: next });
    const query = writeFilters(currentParams, next);
    if (clearSearch) query.delete('q');
    setParams(query, { replace: true, state: location.state });
    const touched = Object.fromEntries(
      Object.entries(next).filter(
        ([key, value]) => JSON.stringify(value) !== JSON.stringify(filters[key as keyof Filters]),
      ),
    );
    autosave.change('inventory_filters', (previous) => ({ ...previous, ...touched }));
  }
  function openCampaign(id: string, trigger: string) {
    returnPosition.current = {
      top: results.current?.scrollTop ?? 0,
      width: results.current?.clientWidth ?? 0,
      list,
      id,
    };
    const next = new URLSearchParams(currentParams);
    next.set('campaign', id);
    next.delete('drop');
    navigate(
      { pathname: location.pathname, search: next.toString() },
      {
        replace: !!detailId,
        state: {
          ...location.state,
          campaignDetail: location.state?.campaignDetail || !detailId,
          campaignTrigger: trigger,
        },
      },
    );
  }
  const tabQuery = (history: boolean) => {
    const next = new URLSearchParams(currentParams);
    next.delete('page');
    next.delete('campaign');
    next.delete('drop');
    history ? next.set('tab', 'history') : next.delete('tab');
    return next.toString();
  };
  const games = [
    ...new Set([
      ...(historyTab ? data.campaigns : available).map((campaign) => campaign.game_name),
      ...(historyTab ? history.entries.map((entry) => entry.game) : []),
      ...filters.game_name_search,
    ]),
  ].sort();
  const filterOptions: [keyof Omit<Filters, 'game_name_search'>, string][] = [
    ['show_benefit_badge', 'badge'],
    ['show_benefit_emote', 'emote'],
    ['show_benefit_item', 'item'],
  ];
  return (
    <div className={`campaign-workspace ${detailId ? 'with-detail' : ''}`}>
      <div className="campaign-tools min-w-0">
        <header className="campaign-toolbar">
          <h1 className="sr-only">{t('campaigns')}</h1>
          <nav
            aria-label={t('campaign_views')}
            className="campaign-tabs flex gap-5 text-[13px] text-muted"
          >
            {[false, true].map((value) => (
              <Link
                key={String(value)}
                className={`settings-tab ${historyTab === value ? 'active' : ''}`}
                aria-current={historyTab === value ? 'page' : undefined}
                to={{
                  pathname: '/campaigns',
                  search: tabQuery(value),
                }}
              >
                <Icon path={value ? mdiHistory : mdiGiftOutline} className="mdi-icon" />
                {t(value ? 'gui.tabs.history' : 'available_campaigns')}
              </Link>
            ))}
          </nav>
          {(historyTab ? historical : campaigns).length > 25 && (
            <nav
              className="campaign-pages flex shrink-0 items-center justify-center gap-1"
              aria-label={t('campaign_pages')}
            >
              <IconButton
                path={mdiChevronLeft}
                label={t('gui.history.previous')}
                disabled={page === 0}
                onClick={() => setQuery('page', String(page - 1))}
              />
              <span className="muted whitespace-nowrap tabular-nums">
                {page + 1} / {Math.ceil((historyTab ? historical : campaigns).length / 25)}
              </span>
              <IconButton
                path={mdiChevronRight}
                label={t('gui.history.next')}
                disabled={(page + 1) * 25 >= (historyTab ? historical : campaigns).length}
                onClick={() => setQuery('page', String(page + 1))}
              />
            </nav>
          )}
          <div className="campaign-refresh flex items-center justify-self-end gap-3">
            <p className="text-xs whitespace-nowrap text-[#888888]">
              {t(shown === total ? 'campaign_count' : 'campaign_filtered_count', { shown, total })}
            </p>
            <InventoryRefreshButton />
          </div>
          <div className="campaign-search min-w-0">
            <Search
              value={search}
              onChange={(value) => setQuery('q', value)}
              label={t('search_campaigns')}
            />
          </div>
          <div className="campaign-controls flex items-center justify-end gap-2">
            <FilterPopover
              icon={mdiGamepadVariantOutline}
              label={t('games')}
              maxHeight={360}
              count={filters.game_name_search.length}
              active={!!filters.game_name_search.length}
              open={popover === 'games'}
              onToggle={() => setPopover(popover === 'games' ? null : 'games')}
              onClose={() => setPopover((current) => (current === 'games' ? null : current))}
            >
              <div className="shrink-0 p-3">
                <Search
                  value={gameSearch}
                  onChange={setGameSearch}
                  label={t('gui.settings.search_games')}
                />
              </div>
              <div
                role="group"
                aria-label={t('game')}
                tabIndex={0}
                className="scroll-list min-h-0 overflow-y-auto overscroll-contain px-3 focus-visible:bg-field"
              >
                {games
                  .filter((game) =>
                    game.toLocaleLowerCase().includes(gameSearch.trim().toLocaleLowerCase()),
                  )
                  .map((game) => (
                    <Check
                      key={game}
                      label={game}
                      checked={filters.game_name_search.includes(game)}
                      disabled={!connected || settingsBusy}
                      onChange={(checked) =>
                        changeFilters({
                          ...filters,
                          game_name_search: checked
                            ? [...filters.game_name_search, game]
                            : filters.game_name_search.filter((name) => name !== game),
                        })
                      }
                    />
                  ))}
                {!games.some((game) =>
                  game.toLocaleLowerCase().includes(gameSearch.trim().toLocaleLowerCase()),
                ) && <p className="muted py-2">{t('no_matches')}</p>}
              </div>
              <div className="shrink-0 border-t border-divider p-3">
                <Button
                  disabled={!connected || settingsBusy || !filters.game_name_search.length}
                  onClick={() => changeFilters({ ...filters, game_name_search: [] })}
                >
                  {t('all_games')}
                </Button>
              </div>
            </FilterPopover>
            {!historyTab && (
              <FilterPopover
                icon={mdiFilterOutline}
                label={t('filters')}
                open={popover === 'filters'}
                active={
                  filters.hide_selected_games ||
                  !filters.show_benefit_badge ||
                  !filters.show_benefit_emote ||
                  !filters.show_benefit_item
                }
                onToggle={() => setPopover(popover === 'filters' ? null : 'filters')}
                onClose={() => setPopover((current) => (current === 'filters' ? null : current))}
              >
                <div
                  tabIndex={0}
                  className="min-h-0 overflow-y-auto overscroll-contain p-3 focus-visible:bg-field"
                >
                  <Check
                    label={t('gui.inventory.filters.hide_selected_games')}
                    checked={filters.hide_selected_games}
                    disabled={!connected || settingsBusy}
                    onChange={(value) => changeFilters({ ...filters, hide_selected_games: value })}
                  />
                  <fieldset className="mt-3">
                    <legend className="text-xs text-muted">{t('campaign_reward_types')}</legend>
                    {filterOptions.map(([key, name]) => (
                      <Check
                        key={key}
                        label={t(`gui.inventory.filters.${name}`)}
                        checked={filters[key]}
                        disabled={!connected || settingsBusy}
                        onChange={(value) => changeFilters({ ...filters, [key]: value })}
                      />
                    ))}
                  </fieldset>
                </div>
                <div className="shrink-0 border-t border-divider p-3">
                  <Button
                    disabled={!connected || settingsBusy}
                    onClick={() =>
                      changeFilters(
                        {
                          ...filters,
                          hide_selected_games: false,
                          game_name_search: [],
                          show_benefit_badge: true,
                          show_benefit_emote: true,
                          show_benefit_item: true,
                        },
                        true,
                      )
                    }
                  >
                    <Icon path={mdiFilterOffOutline} className="mdi-icon" />
                    {t('clear_filters')}
                  </Button>
                </div>
              </FilterPopover>
            )}
            <div
              className="icon-button"
              title={`${t(historyTab ? 'sort_history' : 'sort_campaigns')}: ${t(`sort_${sort}`)}`}
            >
              <Icon className="mdi-icon pointer-events-none" path={mdiSortAscending} />
              <select
                className="icon-select absolute inset-0 size-full cursor-pointer opacity-0"
                aria-label={t(historyTab ? 'sort_history' : 'sort_campaigns')}
                value={sort}
                onChange={(event) =>
                  setQuery('sort', event.target.value === 'default' ? '' : event.target.value)
                }
              >
                {campaignSorts.map((value) => (
                  <option key={value} value={value}>
                    {t(`sort_${value}`)}
                  </option>
                ))}
              </select>
            </div>
            <IconButton
              path={list ? mdiViewGridOutline : mdiViewList}
              label={t('toggle_view')}
              disabled={!connected || settingsBusy}
              onClick={() => {
                const next = new URLSearchParams(currentParams);
                next.set('view', list ? 'grid' : 'list');
                setParams(next, { replace: true, state: location.state });
                autosave.change('inventory_list_view', !list);
              }}
            />
          </div>
        </header>
        {!historyTab &&
          data.inventory_status?.available === false &&
          data.inventory_status.checked_at && <Notice error>{t('campaigns_unavailable')}</Notice>}
      </div>
      <div className="campaign-columns">
        <div className="campaign-browser min-w-0">
          <div
            ref={results}
            role="region"
            aria-label={t(historyTab ? 'gui.tabs.history' : 'campaigns')}
            tabIndex={0}
            className={`campaign-results relative space-y-5 focus-visible:bg-field focus-visible:border-control focus-visible:[&_.panel]:border-control ${list ? 'campaign-results-list panel max-h-[440px] overflow-y-auto overscroll-y-contain xl:max-h-none' : ''}`}
          >
            <ActionResult action={action} />
            {historyTab && history.error && (
              <Notice error>
                {t('history_error')}
                <IconButton path={mdiReload} label={t('retry')} onClick={history.retry} />
              </Notice>
            )}
            <div className={list ? 'campaign-list' : 'campaign-grid grid gap-4 md:grid-cols-2'}>
              {historyTab ? (
                <History
                  groups={historical.slice(page * 25, (page + 1) * 25)}
                  onOpen={openCampaign}
                  selected={detailId}
                />
              ) : (
                campaigns.slice(page * 25, (page + 1) * 25).map((campaign) => (
                  <Campaign
                    key={campaign.id}
                    campaign={campaign}
                    onOpen={(trigger) => openCampaign(campaign.id, trigger)}
                    selected={detailId === campaign.id}
                    action={
                      !campaign.finished &&
                      !campaign.expired && (
                        <IconButton
                          path={
                            isSelected(campaign.game_name)
                              ? mdiStopCircleOutline
                              : mdiPlayCircleOutline
                          }
                          disabled={!connected || action.busy}
                          label={t(
                            isSelected(campaign.game_name) ? 'stop_mining_game' : 'mine_game',
                            { game: campaign.game_name },
                          )}
                          data-mine-game={campaign.game_name}
                          onClick={(event) => {
                            const actions = [
                              ...(results.current?.querySelectorAll<HTMLButtonElement>(
                                '[data-mine-game]',
                              ) ?? []),
                            ];
                            const position = actions.indexOf(event.currentTarget);
                            const next = [
                              ...actions.slice(position + 1),
                              ...actions.slice(0, position).reverse(),
                            ];
                            const hiding =
                              filters.hide_selected_games && !isSelected(campaign.game_name);
                            autosave.change('games_to_watch', (games) =>
                              games.some((game) => gameKey(game) === gameKey(campaign.game_name))
                                ? games.filter(
                                    (game) => gameKey(game) !== gameKey(campaign.game_name),
                                  )
                                : [...games, campaign.game_name],
                            );
                            if (hiding)
                              requestAnimationFrame(() =>
                                (
                                  next.find((button) => button.isConnected) ?? results.current
                                )?.focus({ preventScroll: true }),
                              );
                          }}
                        />
                      )
                    }
                  />
                ))
              )}
            </div>
            {historyTab
              ? !historical.length &&
                !history.loading &&
                !history.error && (
                  <Empty title={t(history.entries.length ? 'no_matches' : 'history_empty')} />
                )
              : !campaigns.length && (
                  <Empty
                    title={t(available.length ? 'no_matches' : 'no_available_campaigns')}
                    detail={t('campaign_empty_help')}
                  >
                    {filters.hide_selected_games &&
                      available.some(
                        (campaign) =>
                          isSelected(campaign.game_name) &&
                          matchesCampaign(
                            campaign,
                            { ...filters, hide_selected_games: false },
                            search,
                          ),
                      ) && (
                        <Button
                          onClick={() => changeFilters({ ...filters, hide_selected_games: false })}
                          disabled={!connected || settingsBusy}
                        >
                          {t('show_selected_games')}
                        </Button>
                      )}
                  </Empty>
                )}
          </div>
        </div>
        {detailId && (
          <CampaignDetail
            campaign={data.campaigns.find((campaign) => campaign.id === detailId)}
            history={groups.find((group) => group.id === detailId)}
            historyOnly={historyTab}
            loading={history.loading}
            historyError={history.error}
            retryHistory={history.retry}
          />
        )}
      </div>
    </div>
  );
}
