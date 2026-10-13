import { Icon } from '@mdi/react';
import { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router';
import { mdiArrowLeft, mdiPriorityHigh, mdiReload, mdiPlus } from '@mdi/js';
import type { Settings as SettingsData, GameMetadata } from '../../shared/lib/types';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../../shared/lib/i18n';
import { GamePriorities } from './GamePriorities';
import SearchPicker from '../../shared/ui/SearchPicker';
import { lookupGames, useDirectorySearch } from './useDirectorySearch';
import {
  ActionResult,
  Button,
  Check,
  Dialog,
  Field,
  HelpButton,
  IconButton,
  useAction,
} from '../../shared/ui/index';

export default function MiningPreferences() {
  const { data, connected, autosave } = useMiner();
  const t = useT();
  const settings = data!.settings;
  const draft = autosave.draft ?? settings;
  const gameKey = (name: string) => settings.game_keys?.[name] ?? name.toLowerCase();
  const selectedNames = JSON.stringify(draft.games_to_watch);
  const change = autosave.change;
  const heading = useRef<HTMLHeadingElement>(null);
  const gameList = useRef<HTMLDivElement>(null);
  useEffect(() => {
    window.scrollTo(0, 0);
    heading.current?.focus({ preventScroll: true });
  }, []);
  const [ignoredText, setIgnoredText] = useState(draft.drop_name_blacklist.join('\n'));
  const [editingIgnored, setEditingIgnored] = useState(false);
  useEffect(() => {
    if (!editingIgnored) setIgnoredText(draft.drop_name_blacklist.join('\n'));
  }, [draft.drop_name_blacklist, editingIgnored]);
  const [search, setSearch] = useState('');
  const [gameError, setGameError] = useState('');
  const directory = useDirectorySearch<GameMetadata>(
    '/api/games',
    search,
    connected,
    data?.login.user_id,
  );
  useEffect(() => {
    if (!connected || !data?.login.user_id) return;
    const known = new Set((draft.game_metadata ?? []).map((game) => gameKey(game.name)));
    const missing = draft.games_to_watch.filter((name) => !known.has(gameKey(name)));
    if (!missing.length) return;
    const controller = new AbortController();
    void lookupGames(missing, controller.signal)
      .then((games) => {
        if (!controller.signal.aborted && games.length) {
          change('game_metadata', (current) => [
            ...current,
            ...games.filter(
              (game) => !current.some((saved) => gameKey(saved.name) === gameKey(game.name)),
            ),
          ]);
        }
      })
      .catch(() => {
        /* Existing covers and manual names remain usable; retry on the next visit. */
      });
    return () => controller.abort();
  }, [connected, data?.login.user_id, selectedNames]);
  const [confirmation, setConfirmation] = useState<{
    title: string;
    text: string;
    action: () => Promise<unknown>;
  } | null>(null);
  const command = useAction();
  function addGame(name: string, metadata?: GameMetadata) {
    change('games_to_watch', (games) =>
      games.some((game) => gameKey(game) === gameKey(name)) ? games : [...games, name],
    );
    if (metadata)
      change('game_metadata', (games) => [
        metadata,
        ...games.filter((game) => gameKey(game.name) !== gameKey(name)),
      ]);
    setSearch('');
    setGameError('');
  }
  function resolveGame() {
    const name = search.trim();
    if (!name || directory.loading) return;
    if (draft.games_to_watch.some((game) => gameKey(game) === gameKey(name))) {
      setSearch('');
      return;
    }
    const matches = available.map((game) => game.name);
    const exact = matches.find((item) => gameKey(item) === gameKey(name));
    const selected = exact ?? (matches.length === 1 ? matches[0] : undefined);
    if (selected) {
      if (!draft.games_to_watch.includes(selected))
        addGame(selected, available.find((game) => game.name === selected)?.metadata);
      return;
    }
    if (matches.length > 1) {
      setGameError(t('gui.settings.multiple_games_found'));
      return;
    }
    setConfirmation({
      title: t('gui.settings.add_game'),
      text: t('gui.settings.manual_game_warning', { game: name }),
      action: async () => {
        addGame(name);
      },
    });
  }
  const available = [
    ...new Map(
      [
        ...(settings.games_available ?? [])
          .filter((game) => game.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()))
          .map((name) => ({
            name,
            metadata: undefined as GameMetadata | undefined,
            image: data?.campaigns.find((c) => c.game_name === name)?.game_box_art_url,
          })),
        ...directory.items.map((metadata) => ({
          name: metadata.name,
          metadata,
          image: metadata.box_art_url,
        })),
      ].map((game) => [gameKey(game.name), game]),
    ).values(),
  ].filter((game) => !draft.games_to_watch.some((name) => gameKey(name) === gameKey(game.name)));
  return (
    <div className="flex flex-col gap-5 lg:min-h-0 lg:flex-1">
      <header className="flex shrink-0 items-center gap-3">
        <Link
          to="/"
          className="icon-button"
          aria-label={t('back_to_mining')}
          title={t('back_to_mining')}
        >
          <Icon path={mdiArrowLeft} className="mdi-icon" />
        </Link>
        <h1 ref={heading} tabIndex={-1} className="text-[22px] font-semibold">
          {t('mining_preferences')}
        </h1>
      </header>
      <section
        id="priorities"
        className="panel relative p-5 lg:min-h-0 lg:flex-1 lg:overflow-hidden"
        aria-label={t('mining_preferences')}
      >
        <form className="lg:h-full" onSubmit={(event) => event.preventDefault()}>
          <div className="grid min-w-0 gap-6 lg:h-full lg:grid-cols-2 lg:grid-rows-[minmax(0,1fr)]">
            <div
              role="group"
              aria-label={t('game_priorities')}
              tabIndex={!connected ? 0 : undefined}
              className="preferences-games relative flex min-h-0 min-w-0 flex-col gap-4 focus-visible:bg-field"
            >
              <div className="flex min-h-0 flex-col gap-2">
                <div className="flex shrink-0 flex-wrap justify-end gap-2">
                  <SearchPicker
                    label={t('gui.settings.search_games')}
                    actionLabel={t('gui.settings.add_game')}
                    actionIcon={mdiPlus}
                    maxLength={100}
                    disabled={!connected}
                    loading={directory.loading}
                    ready={!!gameError || available.length > 0 || directory.complete}
                    options={available.map((game) => ({ ...game, id: game.name }))}
                    onSelect={(game) => addGame(game.name, game.metadata)}
                    onResolve={resolveGame}
                    value={search}
                    onChange={(value) => {
                      setSearch(value);
                      setGameError('');
                    }}
                  >
                    {gameError && (
                      <p role="alert" className="muted px-3 py-2">
                        {gameError}
                      </p>
                    )}
                    {directory.complete && !directory.error && !available.length && (
                      <p role="status" className="muted px-3 py-2">
                        {t('game_search_empty')}
                      </p>
                    )}
                    {directory.error && (
                      <div
                        role="status"
                        className="flex items-center gap-2 px-3 py-2 text-[13px] text-muted"
                      >
                        <span>{t('game_search_failed')}</span>
                        <IconButton path={mdiReload} label={t('retry')} onClick={directory.retry} />
                      </div>
                    )}
                  </SearchPicker>
                  <div
                    className="icon-button has-[:disabled]:opacity-50"
                    title={`${t('mining_priority')}: ${t(`priority_${draft.mining_priority_mode}`)}`}
                  >
                    <Icon className="mdi-icon pointer-events-none" path={mdiPriorityHigh} />
                    <select
                      className="icon-select absolute inset-0 size-full cursor-pointer opacity-0 disabled:cursor-default"
                      aria-label={t('mining_priority')}
                      aria-describedby="mining-priority-details-text"
                      disabled={!connected}
                      value={draft.mining_priority_mode}
                      onChange={(event) =>
                        change(
                          'mining_priority_mode',
                          event.target.value as SettingsData['mining_priority_mode'],
                        )
                      }
                    >
                      {(['manual', 'short_events', 'ending_soonest'] as const).map((mode) => (
                        <option key={mode} value={mode}>
                          {t(`priority_${mode}`)}
                        </option>
                      ))}
                    </select>
                  </div>
                  <HelpButton
                    id="mining-priority-details"
                    label={t('mining_priority')}
                    text={t(`priority_${draft.mining_priority_mode}_help`)}
                  />
                </div>
              </div>
              <div
                ref={gameList}
                role="region"
                aria-label={t('game_priorities')}
                tabIndex={0}
                className="scroll-list relative max-h-[440px] min-h-0 scroll-py-1 overflow-y-auto focus-visible:bg-field focus-visible:[&_.panel]:bg-field lg:max-h-none lg:flex-1 lg:overscroll-y-contain"
              >
                <fieldset disabled={!connected}>
                  <GamePriorities
                    games={draft.games_to_watch}
                    campaigns={data?.campaigns ?? []}
                    metadata={draft.game_metadata ?? []}
                    gameKeys={settings.game_keys}
                    onChange={(games) => change('games_to_watch', games)}
                    scrollContainer={gameList}
                  />
                </fieldset>
              </div>
            </div>
            <div
              role="group"
              aria-label={t('mining_preferences')}
              tabIndex={!connected ? 0 : undefined}
              className="relative min-h-0 min-w-0 focus-visible:bg-field lg:overflow-y-auto"
            >
              <div className="space-y-5">
                <div className="flex items-center gap-1">
                  <Check
                    label={t('auto_claim_drops')}
                    checked={draft.auto_claim_drops}
                    disabled={!connected}
                    onChange={(value) => change('auto_claim_drops', value)}
                  />
                  <HelpButton label={t('auto_claim_drops')} text={t('auto_claim_drops_help')} />
                </div>
                <Check
                  label={t('show_progress_percentage')}
                  checked={draft.show_progress_percentage}
                  disabled={!connected}
                  onChange={(value) => change('show_progress_percentage', value)}
                />
                <fieldset
                  disabled={!connected}
                  aria-label={t('auto_mine_types')}
                  className="space-y-2"
                >
                  <legend className="text-[13px] font-medium">
                    <span className="flex items-center gap-1">
                      {t('auto_mine_types')}
                      <HelpButton label={t('auto_mine_types')} text={t('auto_mine_types_help')} />
                    </span>
                  </legend>
                  <div className="flex flex-wrap gap-x-6">
                    <Check
                      label={t('reward_badges')}
                      checked={draft.auto_mine_badges}
                      onChange={(value) => change('auto_mine_badges', value)}
                    />
                    <Check
                      label={t('reward_emotes')}
                      checked={draft.auto_mine_emotes}
                      onChange={(value) => change('auto_mine_emotes', value)}
                    />
                  </div>
                </fieldset>
                <fieldset disabled={!connected}>
                  <legend className="mb-2 text-[13px] font-medium">
                    {t('gui.settings.mining_benefits')}
                  </legend>
                  <div className="flex flex-wrap gap-x-6">
                    {[
                      ['BADGE', 'badges'],
                      ['EMOTE', 'emotes'],
                      ['DIRECT_ENTITLEMENT', 'items'],
                      ['UNKNOWN', 'other'],
                    ].map(
                      ([key, label]) =>
                        key && (
                          <Check
                            key={key}
                            label={t(`reward_${label}`)}
                            checked={draft.mining_benefits[key] ?? true}
                            onChange={(value) =>
                              change('mining_benefits', { ...draft.mining_benefits, [key]: value })
                            }
                          />
                        ),
                    )}
                  </div>
                </fieldset>
                <Field
                  label={t('gui.settings.drop_name_blacklist')}
                  detail={t('gui.settings.drop_name_blacklist_help')}
                >
                  <textarea
                    className="field"
                    disabled={!connected}
                    value={ignoredText}
                    onFocus={() => setEditingIgnored(true)}
                    onBlur={() => setEditingIgnored(false)}
                    onChange={(event) => {
                      setIgnoredText(event.target.value);
                      change('drop_name_blacklist', event.target.value.split('\n'));
                    }}
                  />
                </Field>
              </div>
            </div>
          </div>
        </form>
      </section>
      <Dialog
        open={confirmation !== null}
        title={confirmation?.title ?? ''}
        onClose={() => setConfirmation(null)}
      >
        <p className="text-muted">{confirmation?.text}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setConfirmation(null)}>{t('cancel')}</Button>
          <Button
            primary
            disabled={command.busy}
            onClick={() =>
              void command.run(async () => {
                await confirmation?.action();
                setConfirmation(null);
              })
            }
          >
            {t('gui.settings.confirm_btn')}
          </Button>
        </div>
        <ActionResult action={command} />
      </Dialog>
    </div>
  );
}
