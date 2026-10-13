import { useEffect, useRef, useState, type PointerEvent, type RefObject } from 'react';
import { mdiClose } from '@mdi/js';
import { moveGame } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import type { Campaign, GameMetadata } from '../../shared/lib/types';
import { Art, IconButton, Empty } from '../../shared/ui/index';

export function GamePriorities({
  games,
  campaigns,
  metadata,
  gameKeys,
  onChange,
  scrollContainer,
}: {
  games: string[];
  campaigns: Campaign[];
  metadata: GameMetadata[];
  gameKeys?: Record<string, string>;
  onChange: (games: string[]) => void;
  scrollContainer: RefObject<HTMLElement | null>;
}) {
  const t = useT();
  const gameKey = (name: string) => gameKeys?.[name] ?? name.toLowerCase();
  const list = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    game: string;
    games: string[];
    original: string;
    x: number;
    y: number;
    active: boolean;
  } | null>(null);
  const [preview, setPreview] = useState<{
    game: string;
    games: string[];
    x: number;
    y: number;
  } | null>(null);
  const [announcement, setAnnouncement] = useState('');
  const signature = JSON.stringify(games);
  const cancel = () => {
    drag.current = null;
    setPreview(null);
  };
  useEffect(() => {
    if (drag.current && drag.current.original !== signature) cancel();
    list.current?.querySelector<HTMLElement>(':focus')?.scrollIntoView({ block: 'nearest' });
  }, [signature]);
  function move(event: PointerEvent<HTMLDivElement>) {
    const current = drag.current;
    if (!current) return;
    if (!current.active && Math.hypot(event.clientX - current.x, event.clientY - current.y) < 5)
      return;
    current.active = true;
    const target = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>('[data-game]');
    if (target && list.current?.contains(target)) {
      const from = current.games.indexOf(current.game);
      const to = current.games.indexOf(target.dataset.game ?? '');
      if (to !== -1 && to !== from) current.games = moveGame(current.games, from, to);
    }
    const container = scrollContainer.current;
    const scrolling = container && container.scrollHeight > container.clientHeight;
    const bounds = scrolling ? container.getBoundingClientRect() : null;
    const edge = Math.min(70, (bounds?.height ?? window.innerHeight) / 3);
    const scrollBy =
      event.clientY < (bounds?.top ?? 0) + edge
        ? -12
        : event.clientY > (bounds?.bottom ?? window.innerHeight) - edge
          ? 12
          : 0;
    if (scrollBy) (scrolling ? container : window).scrollBy(0, scrollBy);
    setPreview({ game: current.game, games: current.games, x: event.clientX, y: event.clientY });
  }
  return (
    <>
      <p id="priority-instructions" className="sr-only">
        {t('reorder_help')}
      </p>
      <div
        ref={list}
        role="list"
        tabIndex={-1}
        aria-label={t('game_priorities')}
        onPointerMove={move}
        onPointerUp={() => {
          const current = drag.current;
          if (current?.active) onChange(current.games);
          cancel();
        }}
        onPointerCancel={cancel}
        onLostPointerCapture={cancel}
        onKeyDown={(event) => {
          if (event.key === 'Escape') cancel();
        }}
      >
        {(preview?.games ?? games).map((game, index) => (
          <div
            className={`row group ${preview?.game === game ? 'opacity-40' : ''}`}
            data-game={game}
            key={game}
            role="listitem"
          >
            <button
              type="button"
              className="drag-handle"
              aria-label={t('reorder_game', { game })}
              aria-describedby="priority-instructions"
              title={t('reorder_game', { game })}
              onPointerDown={(event) => {
                if (event.button !== 0 || !event.isPrimary) return;
                event.preventDefault();
                // Capture on the stable list: moving a row can release its capture.
                list.current?.focus({ preventScroll: true });
                list.current?.setPointerCapture(event.pointerId);
                drag.current = {
                  game,
                  games,
                  original: signature,
                  x: event.clientX,
                  y: event.clientY,
                  active: false,
                };
              }}
              onKeyDown={(event) => {
                if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
                event.preventDefault();
                const target = Math.max(
                  0,
                  Math.min(games.length - 1, index + (event.key === 'ArrowUp' ? -1 : 1)),
                );
                onChange(moveGame(games, index, target));
                setAnnouncement(
                  t('reordered_game', { game, position: target + 1, total: games.length }),
                );
              }}
            >
              <span aria-hidden="true" />
            </button>
            <Art
              url={
                metadata.find((item) => gameKey(item.name) === gameKey(game))?.box_art_url ||
                campaigns.find((campaign) => gameKey(campaign.game_name) === gameKey(game))
                  ?.game_box_art_url
              }
              className="size-9"
            />
            <span className="min-w-0 flex-1 break-words text-[13px]">{game}</span>
            <IconButton
              path={mdiClose}
              label={t('gui.settings.remove_game', { game })}
              onClick={() => onChange(games.filter((item) => item !== game))}
            />
          </div>
        ))}
        {!games.length && <Empty title={t('no_game_priorities')} />}
      </div>
      <span className="sr-only" role="status">
        {announcement}
      </span>
      {preview && (
        <div
          aria-hidden="true"
          className="pointer-events-none fixed z-50 max-w-64 rounded border border-control bg-raised px-4 py-3 text-[13px] shadow-lg"
          style={{ left: preview.x + 12, top: preview.y - 12 }}
        >
          {preview.game}
        </div>
      )}
    </>
  );
}
