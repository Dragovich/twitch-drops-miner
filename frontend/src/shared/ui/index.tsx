import {
  cloneElement,
  useEffect,
  useId,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ComponentProps,
  type ReactNode,
  type ReactElement,
  type RefObject,
} from 'react';
import {
  mdiClose,
  mdiMagnify,
  mdiImageOutline,
  mdiAlertCircleOutline,
  mdiLoading,
  mdiInformationOutline,
} from '@mdi/js';
import { Icon } from '@mdi/react';
import { ApiError, safeUrl } from '../lib/api';
import { useT } from '../lib/i18n';
export function IconButton({
  path,
  label,
  className = '',
  ...props
}: Omit<ComponentProps<'button'>, 'children'> & { path: string; label: string }) {
  return (
    <button
      type="button"
      className={`icon-button ${className}`}
      aria-label={label}
      title={label}
      {...props}
    >
      <Icon path={path} className="mdi-icon" />
    </button>
  );
}
export function Button({
  children,
  primary = false,
  className = '',
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean }) {
  return (
    <button
      type="button"
      className={`button ${primary ? 'button-primary' : ''} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
}
export function Input({ className = '', ...props }: ComponentProps<'input'>) {
  return <input className={`field ${className}`} {...props} />;
}
export function HelpButton({ label, text, id }: { label: string; text: string; id?: string }) {
  const generatedId = useId();
  const target = id ?? generatedId;
  const t = useT();
  return (
    <>
      <IconButton
        path={mdiInformationOutline}
        label={t('help_for', { topic: label })}
        popoverTarget={target}
      />
      <span
        id={target}
        popover="auto"
        role="note"
        aria-label={t('help_for', { topic: label })}
        className="help-popover"
      >
        <span id={`${target}-text`}>{text}</span>
      </span>
    </>
  );
}
export function Field({
  label,
  help,
  detail,
  children,
}: {
  label: string;
  help?: string;
  detail?: string;
  children: ReactElement<{ id?: string; 'aria-describedby'?: string }>;
}) {
  const id = useId();
  return (
    <div className="grid content-start gap-2 text-soft">
      <div className="flex items-center gap-1">
        <label htmlFor={id} className="text-[13px] font-medium">
          {label}
        </label>
        {detail && <HelpButton label={label} text={detail} />}
      </div>
      {cloneElement(children, { id, 'aria-describedby': help ? `${id}-help` : undefined })}
      {help && (
        <p id={`${id}-help`} className="text-[13px] leading-relaxed text-muted">
          {help}
        </p>
      )}
    </div>
  );
}
export function Search({
  value,
  onChange,
  label,
  inputProps,
  inputRef,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  inputProps?: Omit<ComponentProps<'input'>, 'value' | 'onChange' | 'type' | 'ref'>;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  const ownRef = useRef<HTMLInputElement>(null);
  const ref = inputRef ?? ownRef;
  const t = useT();
  return (
    <div className="relative min-w-0">
      <span className="pointer-events-none absolute start-2.5 top-1/2 -translate-y-1/2 text-muted">
        <Icon path={mdiMagnify} className="mdi-icon size-4" />
      </span>
      <Input
        ref={ref}
        type="search"
        aria-label={label}
        placeholder={label}
        value={value}
        className="ps-9 pe-10"
        onChange={(event) => onChange(event.target.value)}
        {...inputProps}
        onKeyDown={(event) => {
          inputProps?.onKeyDown?.(event);
          if (!event.defaultPrevented && event.key === 'Escape') onChange('');
        }}
      />
      {value && (
        <IconButton
          path={mdiClose}
          label={t('clear_search')}
          className="absolute end-1 top-1/2 size-7 -translate-y-1/2 max-md:size-9"
          onClick={() => {
            onChange('');
            ref.current?.focus();
          }}
        />
      )}
    </div>
  );
}
export function Check({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="flex min-h-9 cursor-pointer items-center gap-2.5 text-soft max-md:min-h-11">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        className="size-4 accent-soft"
      />
      <span>{label}</span>
    </label>
  );
}
export function Art({
  url,
  className = '',
  fit = 'cover',
}: {
  url?: string | null;
  className?: string;
  fit?: 'cover' | 'contain';
}) {
  const [failed, setFailed] = useState(false);
  const source = safeUrl(url?.replaceAll('{width}', '80').replaceAll('{height}', '112'));
  useEffect(() => setFailed(false), [url]);
  return (
    <span
      className={`grid size-10 shrink-0 place-items-center overflow-hidden rounded bg-raised text-muted ${className}`}
    >
      {source && !failed ? (
        <img
          loading="lazy"
          className={`size-full ${fit === 'contain' ? 'object-contain' : 'object-cover'}`}
          src={source}
          alt=""
          onError={() => setFailed(true)}
        />
      ) : (
        <Icon className="mdi-icon" path={mdiImageOutline} />
      )}
    </span>
  );
}
export function ProgressBar({
  current,
  total,
  label,
  description,
}: {
  current: number;
  total: number;
  label: string;
  description?: string;
}) {
  const value = Math.min(100, Math.max(0, total > 0 ? (current / total) * 100 : 0));
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-description={description}
      title={description}
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={Math.min(total, Math.max(0, current))}
      className="h-1.5 overflow-hidden rounded-sm bg-divider"
    >
      <div
        className="h-full bg-soft transition-[width] motion-reduce:transition-none"
        style={{ width: `${value}%` }}
      />
    </div>
  );
}
export function Empty({
  title,
  detail,
  children,
  className = 'py-10',
}: {
  title: string;
  detail?: string;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`${className} text-center`}>
      <p className="font-medium text-soft">{title}</p>
      {detail && (
        <p className="mx-auto mt-2 max-w-md text-[13px] leading-relaxed text-muted">{detail}</p>
      )}
      {children && <div className="mt-5 flex justify-center">{children}</div>}
    </div>
  );
}
export function Notice({ children, error = false }: { children: ReactNode; error?: boolean }) {
  return (
    <div
      role={error ? 'alert' : 'status'}
      className="flex items-start gap-2 rounded border border-divider bg-field px-3 py-2 text-[13px] leading-relaxed text-soft"
    >
      {error && <Icon className="mdi-icon" path={mdiAlertCircleOutline} />}
      {children}
    </div>
  );
}
export function useAction() {
  const t = useT();
  const pending = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');
  async function run(action: () => Promise<unknown>, message = '') {
    if (pending.current) return false;
    pending.current = true;
    setBusy(true);
    setError('');
    setSuccess('');
    try {
      await action();
      setSuccess(message);
      return true;
    } catch (error) {
      setError(
        error instanceof ApiError && error.message === 'settings_conflict'
          ? t('settings_conflict')
          : error instanceof ApiError &&
              t(`gui.auth.${error.message}`) !== `gui.auth.${error.message}`
            ? t(`gui.auth.${error.message}`)
            : t('gui.auth.request_failed'),
      );
      return false;
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }
  return {
    busy,
    error,
    success,
    run,
    clear: () => {
      setError('');
      setSuccess('');
    },
  };
}
export function ActionResult({ action }: { action: ReturnType<typeof useAction> }) {
  return action.error ? (
    <Notice error>{action.error}</Notice>
  ) : action.success ? (
    <Notice>{action.success}</Notice>
  ) : null;
}
export function Busy({ label }: { label: string }) {
  return (
    <span className="flex items-center gap-2 text-muted">
      <Icon path={mdiLoading} className="mdi-icon animate-spin motion-reduce:animate-none" />
      {label}
    </span>
  );
}
export function Dialog({
  open,
  title,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  const t = useT();
  useEffect(() => {
    const dialog = ref.current;
    if (open && !dialog?.open) dialog?.showModal();
    else if (!open && dialog?.open) dialog.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      aria-labelledby={id}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      className="m-auto w-[calc(100%_-_32px)] max-w-md rounded-md border border-divider bg-raised p-6 text-text shadow-xl backdrop:bg-black/65"
    >
      <div className="mb-4 flex items-center justify-between gap-4">
        <h2 id={id} className="text-lg font-semibold">
          {title}
        </h2>
        <IconButton path={mdiClose} label={t('close')} onClick={onClose} />
      </div>
      {children}
    </dialog>
  );
}
export const dateTime = (value: string, compact = false) =>
  new Intl.DateTimeFormat(undefined, {
    ...(compact
      ? ({ month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' } as const)
      : ({ dateStyle: 'medium', timeStyle: 'short' } as const)),
    hourCycle: 'h23',
  }).format(new Date(value));

export function ConfirmedTime({ value, className }: { value: string; className?: string }) {
  const t = useT();
  const [today, setToday] = useState(() => new Date());
  useEffect(() => {
    const midnight = new Date();
    midnight.setHours(24, 0, 0, 0);
    const timer = window.setTimeout(() => setToday(new Date()), midnight.getTime() - Date.now());
    return () => window.clearTimeout(timer);
  }, [today]);
  const date = new Date(value);
  const time =
    date.toDateString() === today.toDateString()
      ? new Intl.DateTimeFormat(undefined, {
          hour: '2-digit',
          minute: '2-digit',
          hourCycle: 'h23',
        }).format(date)
      : dateTime(value, date.getFullYear() === today.getFullYear());
  const full = t('last_confirmed', { time: dateTime(value) });
  return (
    <time dateTime={value} title={full} aria-label={full} className={className}>
      {t('last_confirmed', { time })}
    </time>
  );
}
