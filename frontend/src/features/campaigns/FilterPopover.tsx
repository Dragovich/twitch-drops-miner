import { Icon } from '@mdi/react';
import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

export default function FilterPopover({
  label,
  icon,
  open,
  active = false,
  count,
  onToggle,
  onClose,
  children,
}: {
  label: string;
  icon: string;
  open: boolean;
  active?: boolean;
  count?: number;
  onToggle: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [native] = useState(() => typeof HTMLElement.prototype.showPopover === 'function');

  function position() {
    const anchor = trigger.current!.getBoundingClientRect();
    const viewport = window.visualViewport;
    const left = (viewport?.offsetLeft ?? 0) + 8;
    const top = (viewport?.offsetTop ?? 0) + 8;
    const right = left + (viewport?.width ?? innerWidth) - 16;
    const bottom = top + (viewport?.height ?? innerHeight) - 16;
    if (anchor.bottom <= top || anchor.top >= bottom) {
      onClose();
      return;
    }
    const width = Math.min(320, right - left);
    const below = Math.max(0, bottom - anchor.bottom - 4);
    const above = Math.max(0, anchor.top - top - 4);
    const upward = below < 160 && above > below;
    Object.assign(popup.current!.style, {
      left: `${Math.max(left, Math.min(anchor.right - width, right - width))}px`,
      width: `${width}px`,
      maxHeight: `${Math.min(360, upward ? above : below)}px`,
      top: `${upward ? anchor.top - 4 : anchor.bottom + 4}px`,
      transform: upward ? 'translateY(-100%)' : '',
    });
  }

  useLayoutEffect(() => {
    if (open) position();
  });
  useLayoutEffect(() => {
    const element = popup.current!;
    if (!open) {
      element.hidePopover?.();
      return;
    }
    position();
    element.showPopover?.();
    function dismiss(event: PointerEvent | FocusEvent) {
      if (
        !element.contains(event.target as Node) &&
        !trigger.current?.contains(event.target as Node)
      )
        onClose();
    }
    function escape(event: KeyboardEvent) {
      if (event.key !== 'Escape' || event.isComposing) return;
      event.preventDefault();
      event.stopPropagation();
      onClose();
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
  }, [open, onClose]);

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`icon-button ${active ? 'text-soft' : ''}`}
        title={count ? `${label} (${count})` : label}
        aria-label={count ? `${label} (${count})` : label}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={onToggle}
      >
        <Icon path={icon} className="mdi-icon" />
        {!!count && (
          <span
            aria-hidden="true"
            className="absolute -right-1 -top-1 rounded-full bg-raised px-1 text-[10px] tabular-nums"
          >
            {count}
          </span>
        )}
      </button>
      <div
        ref={popup}
        id={id}
        role="group"
        aria-label={label}
        popover={native ? 'manual' : undefined}
        hidden={!open}
        className="campaign-popover panel fixed z-50 m-0 flex flex-col overflow-hidden shadow-lg"
      >
        {children}
      </div>
    </>
  );
}
