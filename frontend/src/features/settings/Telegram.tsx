import { useEffect, useState } from 'react';
import { mdiContentSave, mdiDeleteOutline, mdiOpenInNew } from '@mdi/js';
import { Icon } from '@mdi/react';
import { ApiError, request } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import {
  ActionResult,
  Button,
  Check,
  Field,
  HelpButton,
  Input,
  IconButton,
  Notice,
  useAction,
} from '../../shared/ui';

interface Configuration {
  enabled: boolean;
  include_image: boolean;
  token_configured: boolean;
  chat_id: string;
  revision: string;
  unreadable: boolean;
}

interface BotIdentity {
  revision: string;
  bot: { name: string; username: string } | null;
}

export default function Telegram({ connected }: { connected: boolean }) {
  const t = useT();
  const action = useAction();
  const [saved, setSaved] = useState<Configuration>();
  const [identity, setIdentity] = useState<BotIdentity>();
  const [enabled, setEnabled] = useState(false);
  const [includeImage, setIncludeImage] = useState(false);
  const [chat, setChat] = useState('');
  const [token, setToken] = useState('');
  const [loadFailed, setLoadFailed] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setSaved(undefined);
    setToken('');
    setLoadFailed(false);
    void request<Configuration>('/api/telegram', undefined, 'GET', controller.signal)
      .then((value) => {
        if (controller.signal.aborted) return;
        setSaved(value);
        setEnabled(value.enabled);
        setIncludeImage(value.include_image);
        setChat(value.chat_id);
      })
      .catch(() => {
        if (!controller.signal.aborted) setLoadFailed(true);
      });
    return () => controller.abort();
  }, [reload]);
  useEffect(() => {
    if (!saved?.token_configured || !connected) return;
    const controller = new AbortController();
    void request<BotIdentity>('/api/telegram/bot', undefined, 'GET', controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) setIdentity(value);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [saved?.revision, saved?.token_configured, connected]);
  const bot = !token && identity?.revision === saved?.revision ? identity?.bot : undefined;
  const dirty =
    !!saved &&
    (enabled !== saved.enabled ||
      includeImage !== saved.include_image ||
      chat !== saved.chat_id ||
      token !== '');
  async function save(remove = false) {
    if (!saved) return;
    await action.run(async () => {
      const value = await request<Configuration>('/api/telegram', {
        revision: saved.revision,
        enabled,
        include_image: includeImage,
        chat_id: chat,
        token,
        remove,
      }).catch((error: unknown) => {
        if (error instanceof ApiError && error.message === 'settings_conflict')
          throw new ApiError(error.status, 'telegram_conflict');
        throw error;
      });
      setSaved(value);
      setEnabled(value.enabled);
      setIncludeImage(value.include_image);
      setChat(value.chat_id);
      setToken('');
    });
  }
  return (
    <section id="notifications" className="max-w-md space-y-4">
      <div className="flex items-center gap-1">
        <h2 className="font-semibold">Telegram</h2>
        <HelpButton label="Telegram" text={t('telegram_help')} />
        {bot && (
          <a
            href={`https://t.me/${encodeURIComponent(bot.username)}`}
            target="_blank"
            rel="noreferrer"
            aria-label={t('telegram_open_bot', { name: bot.name })}
            title={`@${bot.username}`}
            className="ml-auto inline-flex min-w-0 max-w-[60%] items-center gap-1 text-muted hover:text-text"
          >
            <span className="truncate underline underline-offset-4">{bot.name}</span>
            <Icon path={mdiOpenInNew} className="mdi-icon" />
          </a>
        )}
      </div>
      {loadFailed ? (
        <Notice error>{t('gui.auth.request_failed')}</Notice>
      ) : (
        !saved && <p className="text-muted">{t('loading')}</p>
      )}
      {loadFailed && (
        <Button onClick={() => setReload((value) => value + 1)}>{t('telegram_retry')}</Button>
      )}
      {saved && (
        <form
          className="space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          {saved.unreadable && <Notice error>{t('gui.auth.telegram_storage')}</Notice>}
          <fieldset disabled={!connected || action.busy || saved.unreadable} className="space-y-4">
            <Check
              label={t('telegram_enabled')}
              checked={enabled}
              onChange={(value) => {
                action.clear();
                setEnabled(value);
              }}
            />
            <Check
              label={t('telegram_include_image')}
              checked={includeImage}
              onChange={(value) => {
                action.clear();
                setIncludeImage(value);
              }}
            />
            <Field label={t('telegram_bot_token')}>
              <Input
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                maxLength={256}
                placeholder={saved.token_configured ? t('telegram_token_saved') : ''}
                value={token}
                onChange={(event) => {
                  action.clear();
                  setToken(event.target.value);
                }}
              />
            </Field>
            <Field label={t('telegram_chat_id')}>
              <Input
                value={chat}
                maxLength={24}
                spellCheck={false}
                onChange={(event) => {
                  action.clear();
                  setChat(event.target.value);
                }}
              />
            </Field>
            <div className="flex flex-wrap items-center gap-2">
              <Button type="submit" primary disabled={!dirty}>
                <Icon path={mdiContentSave} className="mdi-icon" />
                {t('telegram_save')}
              </Button>
              <Button
                disabled={dirty || !saved.enabled}
                onClick={() =>
                  void action.run(() => request('/api/telegram/test', {}), t('telegram_test_sent'))
                }
              >
                {t('telegram_test')}
              </Button>
              {saved.token_configured && (
                <IconButton
                  path={mdiDeleteOutline}
                  label={t('telegram_remove')}
                  onClick={() => void save(true)}
                />
              )}
            </div>
          </fieldset>
          <ActionResult action={action} />
        </form>
      )}
    </section>
  );
}
