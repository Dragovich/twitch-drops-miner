import { useCallback, useId, useLayoutEffect, useRef, useState } from 'react';
import { mdiPlus, mdiPlayCircleOutline, mdiReload } from '@mdi/js';
import { useMiner } from '../../app/MinerProvider';
import { useT } from '../../shared/lib/i18n';
import { request } from '../../shared/lib/api';
import type { ChannelSearchResult } from '../../shared/lib/types';
import { IconButton, Input, Notice, useAction } from '../../shared/ui';
import SearchPicker from '../../shared/ui/SearchPicker';
import { useDirectorySearch } from './useDirectorySearch';

export default function ChannelPicker({
  open,
  onOpenChange,
  action,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  action: ReturnType<typeof useAction>;
}) {
  const { data, connected } = useMiner();
  const t = useT();
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const form = useRef<HTMLFormElement>(null);
  const opening = useRef(0);
  const [native] = useState(() => typeof HTMLElement.prototype.showPopover === 'function');
  const [channel, setChannel] = useState('');
  const [minutes, setMinutes] = useState('');
  const close = useCallback(() => onOpenChange(false), [onOpenChange]);
  const query = channel
    .trim()
    .replace(/^https?:\/\/(?:www\.|m\.)?twitch\.tv\/([a-z0-9_]{1,25})\/?$/i, '$1')
    .replace(/^@/, '');
  const directory = useDirectorySearch<ChannelSearchResult>(
    '/api/channels/search',
    query.length <= 100 ? query : '',
    connected && open,
    data?.login.user_id,
  );
  const busy = action.busy || !!data?.manual_mode.pending_channel;
  const disabled = !connected || !data?.login.user_id || busy;
  const error = action.error || data?.manual_mode.error;

  function position() {
    const anchor = trigger.current!.getBoundingClientRect();
    const viewport = window.visualViewport;
    const left = (viewport?.offsetLeft ?? 0) + 8;
    const top = (viewport?.offsetTop ?? 0) + 8;
    const right = left + (viewport?.width ?? innerWidth) - 16;
    const bottom = top + (viewport?.height ?? innerHeight) - 16;
    if (anchor.bottom <= top || anchor.top >= bottom) return close();
    const width = Math.min(360, right - left);
    const below = Math.max(0, bottom - anchor.bottom - 4);
    const above = Math.max(0, anchor.top - top - 4);
    const upward = below < 240 && above > below;
    Object.assign(popup.current!.style, {
      left: `${Math.max(left, Math.min(anchor.right - width, right - width))}px`,
      width: `${width}px`,
      maxHeight: `${Math.min(420, upward ? above : below)}px`,
      top: `${upward ? anchor.top - 4 : anchor.bottom + 4}px`,
      transform: upward ? 'translateY(-100%)' : '',
    });
  }
  useLayoutEffect(() => {
    if (open) position();
  });
  useLayoutEffect(() => {
    opening.current++;
    const element = popup.current!;
    if (!open) {
      if (native) element.hidePopover();
      return;
    }
    position();
    if (native) element.showPopover();
    element.querySelector('input')?.focus({ preventScroll: true });
    function dismiss(event: PointerEvent | FocusEvent) {
      if (
        !element.contains(event.target as Node) &&
        !trigger.current?.contains(event.target as Node)
      )
        close();
    }
    function escape(event: KeyboardEvent) {
      if (event.key !== 'Escape' || event.isComposing) return;
      event.preventDefault();
      event.stopPropagation();
      close();
      trigger.current?.focus({ preventScroll: true });
    }
    const observer = new ResizeObserver(position);
    observer.observe(trigger.current!);
    document.addEventListener('pointerdown', dismiss);
    document.addEventListener('focusin', dismiss);
    document.addEventListener('keydown', escape, true);
    document.addEventListener('scroll', position, true);
    window.addEventListener('resize', position);
    window.visualViewport?.addEventListener('resize', position);
    window.visualViewport?.addEventListener('scroll', position);
    return () => {
      observer.disconnect();
      document.removeEventListener('pointerdown', dismiss);
      document.removeEventListener('focusin', dismiss);
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('scroll', position, true);
      window.removeEventListener('resize', position);
      window.visualViewport?.removeEventListener('resize', position);
      window.visualViewport?.removeEventListener('scroll', position);
    };
  }, [open, native, close]);

  async function mine(value: string) {
    if (disabled || !value.trim() || !form.current?.reportValidity()) return;
    const started = opening.current;
    // Keep focus reachable while the submitted inputs are disabled.
    popup.current?.focus({ preventScroll: true });
    if (
      await action.run(() =>
        request('/api/channels/select', {
          channel: value,
          duration_minutes: minutes ? Number(minutes) : null,
        }),
      )
    ) {
      if (started !== opening.current || !popup.current) return;
      if (popup.current?.contains(document.activeElement))
        trigger.current?.focus({ preventScroll: true });
      close();
    }
  }

  return (
    <>
      <IconButton
        ref={trigger}
        path={mdiPlus}
        label={t('gui.channels.mine_channel')}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => onOpenChange(!open)}
      />
      <div
        ref={popup}
        id={id}
        role="dialog"
        tabIndex={-1}
        aria-label={t('gui.channels.mine_channel')}
        popover={native ? 'manual' : undefined}
        hidden={!open}
        className="channel-picker panel fixed inset-auto z-50 m-0 flex flex-col overflow-hidden shadow-lg"
      >
        {open && (
          <form
            ref={form}
            className="flex min-h-0 flex-col gap-3 overflow-y-auto p-3"
            onSubmit={(event) => {
              event.preventDefault();
              void mine(channel);
            }}
          >
            <SearchPicker
              inline
              label={t('gui.channels.channel_input')}
              actionLabel={t('mine')}
              actionIcon={mdiPlayCircleOutline}
              submit
              busy={busy}
              roundImages
              maxLength={256}
              value={channel}
              onChange={setChannel}
              disabled={disabled}
              options={directory.items.map((item) => ({
                id: item.login,
                name: item.name,
                image: item.avatar_url,
                description: `@${item.login}, ${t(item.online ? 'gui.channels.live' : 'gui.channels.offline')}${item.online && item.game ? `, ${item.game}` : ''}`,
              }))}
              onSelect={(item) => {
                setChannel(item.id);
                void mine(item.id);
              }}
              onResolve={() => void mine(channel)}
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
                  <IconButton path={mdiReload} label={t('retry')} onClick={directory.retry} />
                </div>
              )}
            </SearchPicker>
            <label className="muted flex shrink-0 items-center justify-between gap-3">
              {t('gui.channels.timer_label')}
              <span className="flex items-center gap-2">
                <Input
                  type="number"
                  min={1}
                  max={1440}
                  step={1}
                  className="w-25!"
                  aria-label={t('gui.channels.manual_timer')}
                  placeholder={t('gui.channels.timer_none')}
                  value={minutes}
                  disabled={disabled}
                  onChange={(event) => setMinutes(event.target.value)}
                />
                {t('gui.channels.timer_unit')}
              </span>
            </label>
            {error && <Notice error>{error}</Notice>}
          </form>
        )}
      </div>
    </>
  );
}
