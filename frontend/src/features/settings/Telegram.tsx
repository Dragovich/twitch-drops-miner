import { useEffect, useState } from 'react';
import { request } from '../../shared/lib/api';
import { useT } from '../../shared/lib/i18n';
import {
  ActionResult,
  Button,
  Check,
  Field,
  HelpButton,
  Input,
  Notice,
  useAction,
} from '../../shared/ui';

interface Configuration {
  enabled: boolean;
  token_configured: boolean;
  chat_id: string;
  revision: string;
  unreadable: boolean;
}

export default function Telegram({ connected }: { connected: boolean }) {
  const t = useT();
  const action = useAction();
  const [saved, setSaved] = useState<Configuration>();
  const [enabled, setEnabled] = useState(false);
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
        setChat(value.chat_id);
      })
      .catch(() => {
        if (!controller.signal.aborted) setLoadFailed(true);
      });
    return () => controller.abort();
  }, [reload]);
  const dirty = !!saved && (enabled !== saved.enabled || chat !== saved.chat_id || token !== '');
  async function save(remove = false) {
    if (!saved) return;
    await action.run(async () => {
      const value = await request<Configuration>('/api/telegram', {
        revision: saved.revision,
        enabled,
        chat_id: chat,
        token,
        remove,
      });
      setSaved(value);
      setEnabled(value.enabled);
      setChat(value.chat_id);
      setToken('');
    }, t('telegram_saved'));
  }
  return (
    <section id="notifications" className="max-w-md space-y-4">
      <div className="flex items-center gap-1">
        <h2 className="font-semibold">Telegram</h2>
        <HelpButton label="Telegram" text={t('telegram_help')} />
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
            <div className="flex flex-wrap gap-2">
              <Button type="submit" primary disabled={!dirty}>
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
                <Button onClick={() => void save(true)}>{t('telegram_remove')}</Button>
              )}
            </div>
          </fieldset>
          <ActionResult action={action} />
          {action.error && (
            <Button
              className="mt-3"
              disabled={action.busy}
              onClick={() => {
                action.clear();
                setReload((value) => value + 1);
              }}
            >
              {t('telegram_reload')}
            </Button>
          )}
        </form>
      )}
    </section>
  );
}
