import Telegram from './Telegram';
import { DesktopUpdates } from './DesktopUpdates';
import UpdateCheck from './UpdateCheck';
import { isDesktop } from '../../shared/lib/platform';
import { DesktopSettings } from './Desktop';
import { Icon } from '@mdi/react';
import { Link, useLocation } from 'react-router';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import {
  mdiOpenInNew,
  mdiLogout,
  mdiContentCopy,
  mdiCheck,
  mdiAccountOutline,
  mdiShieldLockOutline,
  mdiLanConnect,
  mdiWrenchOutline,
  mdiMonitor,
  mdiBellOutline,
  mdiTranslate,
} from '@mdi/js';
import type {
  AuthStatus,
  ReleaseInfo,
  Result,
  Settings as SettingsData,
} from '../../shared/lib/types';
import { request, safeUrl } from '../../shared/lib/api';
import { useMiner } from '../../app/MinerProvider';
import { plainText, useT } from '../../shared/lib/i18n';
import AccountIdentity, { AccountBadges } from '../account/Account';
import {
  ActionResult,
  Button,
  Dialog,
  Empty,
  Field,
  IconButton,
  Input,
  Notice,
  useAction,
} from '../../shared/ui/index';
function Section({
  id,
  help,
  children,
  hidden = false,
}: {
  id: string;
  help?: string;
  children: ReactNode;
  hidden?: boolean;
}) {
  return (
    <section id={id} hidden={hidden} className="scroll-mt-6 space-y-5 pb-8">
      {help && <p className="max-w-2xl text-[13px] leading-relaxed text-muted">{help}</p>}
      <div className="space-y-4">{children}</div>
    </section>
  );
}
function Access({
  initial,
  disabled,
  hidden,
}: {
  initial: AuthStatus;
  disabled: boolean;
  hidden: boolean;
}) {
  const t = useT();
  const [auth, setAuth] = useState(initial);
  useEffect(() => setAuth(initial), [initial]);
  const [current, setCurrent] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [disableDialog, setDisableDialog] = useState(false);
  const action = useAction();
  async function save(kind: 'enable' | 'change' | 'disable') {
    await action.run(async () => {
      const result = await request<AuthStatus>('/api/auth/settings', {
        action: kind,
        current_password: current,
        password,
        confirm_password: confirm,
      });
      setAuth({ ...auth, enabled: result.enabled });
      setCurrent('');
      setPassword('');
      setConfirm('');
      setDisableDialog(false);
      window.dispatchEvent(new Event('auth-updated'));
    });
  }
  return (
    <Section hidden={hidden} id="access" help={t('gui.auth.help')}>
      <p className="muted">{t(auth.enabled ? 'gui.auth.enabled' : 'gui.auth.disabled')}</p>
      {disabled && <Notice>{t('save_first')}</Notice>}
      <form
        className="max-w-md space-y-4"
        onSubmit={(event) => {
          event.preventDefault();
          void save(auth.enabled ? 'change' : 'enable');
        }}
      >
        {auth.enabled && (
          <Field label={t('gui.auth.current_password')}>
            <Input
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(event) => setCurrent(event.target.value)}
              required
              maxLength={1024}
            />
          </Field>
        )}
        <Field label={t('gui.auth.new_password')}>
          <Input
            type="password"
            autoComplete="new-password"
            minLength={8}
            maxLength={1024}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
          />
        </Field>
        <Field label={t('gui.auth.confirm_password')}>
          <Input
            type="password"
            autoComplete="new-password"
            maxLength={1024}
            value={confirm}
            onChange={(event) => setConfirm(event.target.value)}
            required
          />
        </Field>
        <ActionResult action={action} />
        <div className="flex flex-wrap gap-2">
          <Button type="submit" disabled={disabled || action.busy} primary>
            {t(auth.enabled ? 'gui.auth.change' : 'gui.auth.enable')}
          </Button>
          {auth.enabled && (
            <Button
              disabled={disabled || action.busy || !current}
              onClick={() => setDisableDialog(true)}
            >
              {t('disable_protection')}
            </Button>
          )}
        </div>
      </form>
      <Dialog
        open={disableDialog}
        title={t('disable_protection')}
        onClose={() => setDisableDialog(false)}
      >
        <p className="text-muted">{t('disable_help')}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setDisableDialog(false)}>{t('cancel')}</Button>
          <Button primary disabled={action.busy} onClick={() => void save('disable')}>
            {t('disable_protection')}
          </Button>
        </div>
        <ActionResult action={action} />
      </Dialog>
    </Section>
  );
}
function ReleaseNotice({ disabled }: { disabled: boolean }) {
  const t = useT();
  const [release, setRelease] = useState<ReleaseInfo | null>(null);
  const [busy, setBusy] = useState(true);
  const controller = useRef<AbortController | null>(null);
  async function check() {
    controller.current?.abort();
    const next = new AbortController();
    controller.current = next;
    setBusy(true);
    try {
      const result = await request<ReleaseInfo>('/api/version', undefined, 'GET', next.signal);
      if (!next.signal.aborted) setRelease(result);
    } catch {
      if (!next.signal.aborted)
        setRelease((previous) => previous && { ...previous, check_succeeded: false });
    } finally {
      if (!next.signal.aborted) setBusy(false);
    }
  }
  useEffect(() => {
    void check();
    return () => controller.current?.abort();
  }, []);
  const available = release?.check_succeeded && release.update_available;
  const releaseUrl = available ? safeUrl(release.download_url) : undefined;
  return (
    <div className="space-y-3 text-[13px]">
      <UpdateCheck
        version={release?.current_version}
        checking={busy}
        current={!busy && release?.check_succeeded && !available}
        disabled={disabled}
        onCheck={() => void check()}
      >
        {busy ? (
          <p role="status" className="text-muted">
            {t('checking_updates')}
          </p>
        ) : !release?.check_succeeded ? (
          <p role="status" className="text-muted">
            {t('update_check_failed')}
          </p>
        ) : available ? (
          <p role="status" className="text-muted">
            {t('update_available', { version: release.latest_version ?? '' })}
          </p>
        ) : (
          <p role="status" className="text-muted">
            {t('up_to_date')}
          </p>
        )}
      </UpdateCheck>
      {available && !busy && releaseUrl && (
        <a className="text-link inline-block" href={releaseUrl} target="_blank" rel="noreferrer">
          {t('release_notes')}
        </a>
      )}
    </div>
  );
}

function SettingsContent({ settings, auth }: { settings: SettingsData; auth: AuthStatus }) {
  const { data, connected, autosave } = useMiner();
  const t = useT();
  const draft = autosave.draft ?? settings;
  const location = useLocation();
  const section =
    [
      'account',
      isDesktop() ? 'desktop' : 'access',
      'interface',
      'connection',
      'notifications',
      'maintenance',
    ].find((id) => `#${id}` === location.hash) ?? 'account';
  const [confirmation, setConfirmation] = useState<{
    title: string;
    text: string;
    action: () => Promise<unknown>;
  } | null>(null);
  const command = useAction();
  const proxyAction = useAction();
  const oauthAction = useAction();
  const logoutAction = useAction();
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const copyTimer = useRef<number | undefined>(undefined);
  const copyRequest = useRef(0);
  useEffect(() => {
    setCopyState('idle');
    return () => {
      copyRequest.current += 1;
      window.clearTimeout(copyTimer.current);
    };
  }, [data?.login.oauth_pending?.code]);
  async function copyCode(code: string) {
    const request = ++copyRequest.current;
    let state: 'copied' | 'failed' = 'copied';
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      state = 'failed';
    }
    if (request !== copyRequest.current) return;
    window.clearTimeout(copyTimer.current);
    setCopyState(state);
    if (state === 'copied') {
      copyTimer.current = window.setTimeout(() => setCopyState('idle'), 3000);
    }
  }
  const dirty = autosave.pending || autosave.busy;
  const change = autosave.change;
  const oauth = data?.login.oauth_pending;
  async function test(path: string, payload: unknown) {
    const result = await request<Result>(path, payload);
    if (!result.success) throw new Error(result.message);
  }
  return (
    <div className="flex max-w-4xl flex-col gap-8">
      <div>
        <h1 className="text-[22px] font-semibold">{t('gui.tabs.settings')}</h1>
      </div>
      <nav
        aria-label={t('settings_sections')}
        className="flex flex-wrap gap-x-5 gap-y-2 text-[13px] text-muted"
      >
        {(
          [
            ['account', mdiAccountOutline],
            isDesktop() ? ['desktop', mdiMonitor] : ['access', mdiShieldLockOutline],
            ['interface', mdiTranslate],
            ['connection', mdiLanConnect],
            ['notifications', mdiBellOutline],
            ['maintenance', mdiWrenchOutline],
          ] as const
        ).map(([id, icon]) => (
          <Link
            className={`settings-tab ${section === id ? 'active' : ''}`}
            aria-current={section === id ? 'page' : undefined}
            key={id}
            to={`#${id}`}
          >
            <Icon path={icon} className="mdi-icon" />
            {t(id === 'desktop' ? 'gui.desktop.tab' : id)}
          </Link>
        ))}
      </nav>
      <Section hidden={section !== 'account'} id="account">
        <div className="flex min-h-9 items-center gap-2 max-md:min-h-11">
          {data?.login.user_id ? (
            <AccountIdentity profile={data.login.profile} />
          ) : (
            <p className="text-soft">{plainText(data?.login.status ?? '')}</p>
          )}
          {data?.login.user_id && (
            <>
              {data.login.profile?.login && (
                <a
                  className="icon-button"
                  href={`https://www.twitch.tv/${encodeURIComponent(data.login.profile.login)}`}
                  target="_blank"
                  rel="noreferrer"
                  aria-label={t('open_twitch_profile')}
                  title={t('open_twitch_profile')}
                >
                  <Icon path={mdiOpenInNew} className="mdi-icon" />
                </a>
              )}
              <IconButton
                path={mdiLogout}
                label={t('twitch_logout')}
                disabled={!connected || logoutAction.busy}
                onClick={() => void logoutAction.run(() => request('/api/twitch/logout', {}))}
              />
            </>
          )}
        </div>
        <ActionResult action={logoutAction} />
        {data?.login.user_id && (
          <AccountBadges key={data.login.user_id} profile={data.login.profile} />
        )}
        {oauth ? (
          <div className="grid gap-3">
            <div className="authorization-row">
              <div className="flex items-center gap-2 rounded border border-divider bg-field ps-3 pe-1">
                <code className="select-all text-lg tracking-[.2em]">{oauth.code}</code>
                <IconButton
                  path={copyState === 'copied' ? mdiCheck : mdiContentCopy}
                  label={t('copy_code')}
                  title={t(copyState === 'copied' ? 'code_copied' : 'copy_code')}
                  className="size-7 max-md:size-9"
                  onClick={() => void copyCode(oauth.code)}
                />
              </div>
              <a className="button" href={safeUrl(oauth.url)} target="_blank" rel="noreferrer">
                {t('gui.login.oauth_activate')}
                <Icon className="mdi-icon" path={mdiOpenInNew} />
              </a>
              <Button
                disabled={!connected || oauthAction.busy}
                onClick={() => void oauthAction.run(() => request('/api/oauth/confirm', {}))}
              >
                {t('gui.login.oauth_confirm')}
              </Button>
            </div>
            {copyState === 'copied' && (
              <span className="sr-only" role="status">
                {t('code_copied')}
              </span>
            )}
            {copyState === 'failed' && <Notice error>{t('copy_code_failed')}</Notice>}
            <ActionResult action={oauthAction} />
          </div>
        ) : (
          !data?.login.user_id && <Notice>{t('authorization_pending')}</Notice>
        )}
      </Section>
      <form
        hidden={section !== 'connection'}
        onSubmit={(event) => {
          event.preventDefault();
        }}
      >
        <fieldset disabled={!connected} className="min-w-0 space-y-8">
          <Section id="connection">
            <Field label={t('gui.settings.minimum_refresh')}>
              <Input
                type="number"
                min={1}
                max={1440}
                step={1}
                required
                value={
                  Number.isFinite(draft.minimum_refresh_interval_minutes)
                    ? draft.minimum_refresh_interval_minutes
                    : ''
                }
                onChange={(event) =>
                  change('minimum_refresh_interval_minutes', event.target.valueAsNumber)
                }
              />
            </Field>

            {!isDesktop() && <p className="muted">{t(connected ? 'connected' : 'connecting')}</p>}
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label={t('proxy')} help={t('proxy_help')}>
                <Input
                  type="url"
                  autoComplete="off"
                  value={draft.proxy}
                  placeholder="http://127.0.0.1:8080"
                  onChange={(event) => change('proxy', event.target.value)}
                />
              </Field>
              <Field label={t('gui.settings.connection_quality')}>
                <select
                  className="field"
                  value={draft.connection_quality}
                  onChange={(event) => change('connection_quality', Number(event.target.value))}
                >
                  {[1, 2, 3, 4, 5, 6].map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            <Button
              disabled={!connected || !draft.proxy || proxyAction.busy}
              onClick={() =>
                void proxyAction.run(
                  () => test('/api/settings/verify-proxy', { proxy: draft.proxy }),
                  t('connection_verified'),
                )
              }
            >
              {t('verify_proxy')}
            </Button>
            <ActionResult action={proxyAction} />
          </Section>
        </fieldset>
      </form>
      <Section hidden={section !== 'interface'} id="interface">
        <div className="grid max-w-md gap-4">
          <Field label={t('language')} help={t('language_help')}>
            <select
              className="field"
              value={draft.language ?? 'en'}
              disabled={!connected}
              onChange={(event) => change('language', event.target.value === 'ru' ? 'ru' : 'en')}
            >
              <option value="en">{t('language_english')}</option>
              <option value="ru">{t('language_russian')}</option>
            </select>
          </Field>
          <p className="muted">{t('autosave_help')}</p>
        </div>
      </Section>
      {isDesktop() ? (
        <Section hidden={section !== 'desktop'} id="desktop">
          <DesktopSettings />
        </Section>
      ) : (
        <Access hidden={section !== 'access'} initial={auth} disabled={dirty || !connected} />
      )}
      {section === 'notifications' && <Telegram connected={connected} />}
      <Section hidden={section !== 'maintenance'} id="maintenance">
        {isDesktop() ? <DesktopUpdates /> : <ReleaseNotice disabled={!connected} />}
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!connected || command.busy}
            onClick={() =>
              setConfirmation({
                title: t('gui.settings.clear_all_cache'),
                text: t('cache_help'),
                action: () => request('/api/cache/clear', {}),
              })
            }
          >
            {t('gui.settings.clear_all_cache')}
          </Button>
        </div>
        {!isDesktop() && (
          <details>
            <summary className="text-[13px] text-muted">{t('advanced')}</summary>
            <Button
              className="mt-3"
              disabled={!connected || command.busy}
              onClick={() =>
                setConfirmation({
                  title: t('shutdown'),
                  text: t('shutdown_help'),
                  action: () => request('/api/close', {}),
                })
              }
            >
              {t('shutdown')}
            </Button>
          </details>
        )}
        <ActionResult action={command} />
        <div className="space-y-2 pt-2 text-[13px] text-muted">
          <p>
            {t('help_link_accounts')}{' '}
            <a
              className="text-link"
              href="https://www.twitch.tv/drops/campaigns"
              target="_blank"
              rel="noreferrer"
            >
              Twitch
            </a>
          </p>
          <a
            className="text-link inline-block"
            href="https://github.com/ohne-b/twitch-drops-miner"
            target="_blank"
            rel="noreferrer"
          >
            {t('source_license')}
          </a>
        </div>
      </Section>
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
export default function Settings({ auth }: { auth: AuthStatus }) {
  const { data } = useMiner();
  const t = useT();
  return data ? (
    <SettingsContent settings={data.settings} auth={auth} />
  ) : (
    <Empty title={t('loading')} />
  );
}
