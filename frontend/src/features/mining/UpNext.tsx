import { useId, useState } from 'react';
import { Icon } from '@mdi/react';
import { mdiChevronDown, mdiChevronRight } from '@mdi/js';
import type { Snapshot, WantedGame } from '../../shared/lib/types';
import { useT } from '../../shared/lib/i18n';
import { Art, Empty, dateTime } from '../../shared/ui/index';
import { CampaignLink } from '../campaigns/CampaignLink';

const gameKey = (game: WantedGame) => String(game.game_id ?? game.game_key ?? game.game_name);

// Owned by the shell so campaign navigation keeps disclosure and return-focus state.
export function useUpNextState(data: Snapshot | null) {
  const keys = data?.wanted_items.map(gameKey) ?? [];
  const scope = JSON.stringify([data?.instance, data?.login.user_id]);
  const [state, setState] = useState({ scope, keys, expanded: keys.slice(0, 1) });
  let current = state;
  if (
    scope !== state.scope ||
    keys.length !== state.keys.length ||
    keys.some((key, index) => key !== state.keys[index])
  ) {
    current = {
      scope,
      keys,
      expanded:
        scope !== state.scope
          ? keys.slice(0, 1)
          : keys.filter(
              (key) => state.expanded.includes(key) || (key === keys[0] && key !== state.keys[0]),
            ),
    };
    setState(current);
  }
  return {
    expanded: current.expanded,
    toggle(key: string) {
      setState((previous) => ({
        ...previous,
        expanded: previous.expanded.includes(key)
          ? previous.expanded.filter((item) => item !== key)
          : [...previous.expanded, key],
      }));
    },
  };
}

export default function UpNext({
  games,
  disclosure,
}: {
  games: WantedGame[];
  disclosure: ReturnType<typeof useUpNextState>;
}) {
  const t = useT();
  const prefix = useId();
  return (
    <>
      {games.map((game, index) => {
        const key = gameKey(game);
        const id = `${prefix}-${encodeURIComponent(key)}`;
        const expanded = disclosure.expanded.includes(key);
        const drops = game.campaigns.flatMap((campaign) => campaign.drops);
        const deadline = drops.reduce((earliest, drop) => {
          const end = Date.parse(drop.ends_at ?? '');
          return Number.isFinite(end) ? Math.min(earliest, end) : earliest;
        }, Infinity);
        return (
          <div key={key} className="mx-4 border-b border-divider last:border-0">
            <h3>
              <button
                type="button"
                className="flex min-h-11 w-full items-center gap-3 rounded py-3 text-start hover:bg-field"
                aria-expanded={expanded}
                aria-controls={id}
                aria-labelledby={`${id}-name`}
                aria-describedby={`${id}-summary`}
                onClick={() => disclosure.toggle(key)}
              >
                <span className="w-4 shrink-0 text-[13px] tabular-nums text-muted">
                  {index + 1}
                </span>
                <Art url={game.game_icon} className="size-8" />
                <span className="min-w-0 flex-1">
                  <span className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                    <span id={`${id}-name`} className="wrap-anywhere font-medium">
                      {game.game_name}
                    </span>
                    <span className="muted ms-auto whitespace-nowrap text-xs">
                      {game.saved_rank
                        ? t('saved_position', { count: game.saved_rank })
                        : t('automatic')}
                    </span>
                  </span>
                  <span
                    id={`${id}-summary`}
                    className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted"
                  >
                    <span>
                      {t(drops.length === 1 ? 'queued_reward' : 'queued_rewards', {
                        count: drops.length,
                      })}
                    </span>
                    {!expanded && Number.isFinite(deadline) && (
                      <span>
                        {t('gui.inventory.ends', {
                          time: dateTime(new Date(deadline).toISOString()),
                        })}
                      </span>
                    )}
                  </span>
                </span>
                <Icon
                  className="mdi-icon shrink-0 text-muted"
                  path={expanded ? mdiChevronDown : mdiChevronRight}
                />
              </button>
            </h3>
            <div id={id} hidden={!expanded} className="pb-4 ps-7">
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
                  dates.length > 0 &&
                  dates.every(
                    (date) =>
                      date.start === dates[0]?.start &&
                      date.end === dates[0]?.end &&
                      date.upcoming === dates[0]?.upcoming,
                  );
                return (
                  <div className="mt-3 text-[13px] first:mt-0" key={item.id}>
                    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                      <CampaignLink
                        id={`up-next-campaign-${item.id}`}
                        className="text-link min-w-0 wrap-anywhere"
                        to={`/campaigns?campaign=${encodeURIComponent(item.id)}`}
                      >
                        {item.name}
                      </CampaignLink>
                      {sharedDate && <span className="muted text-xs">{dates[0]?.text}</span>}
                    </div>
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
                          <div className="min-w-0 flex-1 wrap-anywhere">
                            <CampaignLink
                              id={`up-next-drop-${item.id}-${drop.id || position}`}
                              className="hover:underline text-soft"
                              to={`/campaigns?campaign=${encodeURIComponent(item.id)}${drop.id ? `&drop=${encodeURIComponent(drop.id)}` : ''}`}
                            >
                              {drop.name}
                            </CampaignLink>
                            {drop.eligibility && drop.eligibility !== 'ready' && (
                              <p className="text-xs mt-1">{t(`eligibility_${drop.eligibility}`)}</p>
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
          </div>
        );
      })}
      {!games.length && <Empty title={t('gui.wanted.none')} />}
    </>
  );
}
