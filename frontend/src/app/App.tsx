import { DesktopUpdateLink, DesktopUpdateProvider } from '../features/settings/DesktopUpdates';
import { isDesktop } from '../shared/lib/platform';
import { DesktopStartupError, DesktopStatus } from '../features/settings/Desktop';
import { Icon } from '@mdi/react';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import {
  NavLink,
  Link,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useNavigationType,
} from 'react-router';
import {
  mdiPlayCircleOutline,
  mdiReload,
  mdiGiftOutline,
  mdiTextBoxOutline,
  mdiCogOutline,
  mdiLogout,
  mdiGithub,
} from '@mdi/js';
import type { AuthStatus } from '../shared/lib/types';
import { request } from '../shared/lib/api';
import { I18n, storedLanguage, useT } from '../shared/lib/i18n';
import { MinerProvider, useMiner } from './MinerProvider';
import { miningTitle } from './title';
import { Button, Empty, Notice, IconButton } from '../shared/ui/index';
import Mining from '../features/mining/Mining';
import { useUpNextState } from '../features/mining/UpNext';
import Campaigns from '../features/campaigns/Campaigns';
import Activity from '../features/activity/Activity';
import Settings from '../features/settings/Settings';
import AccountIdentity from '../features/account/Account';
import Login from '../features/settings/Login';
import logo from '../assets/twitch-drops-miner-logo.svg?no-inline';
function Shell({ auth, onLogout }: { auth: AuthStatus; onLogout: () => Promise<void> }) {
  const { data, connected, incompatible, autosave } = useMiner();
  const upNext = useUpNextState(data);
  const t = useT();
  const title = miningTitle(data, connected, t);
  useEffect(() => {
    document.title = title;
    if (isDesktop()) void invoke('desktop_title', { title }).catch(() => {});
    return () => {
      document.title = 'Drops Miner';
    };
  }, [title]);
  const location = useLocation();
  const navigationType = useNavigationType();
  const previousLocation = useRef(location);
  const [logoutError, setLogoutError] = useState(false);
  const links = [
    ['/', 'mining', mdiPlayCircleOutline],
    ['/campaigns', 'campaigns', mdiGiftOutline],
    ['/activity', 'activity', mdiTextBoxOutline],
    ['/settings', 'gui.tabs.settings', mdiCogOutline],
  ] as const;
  useEffect(() => {
    const previous = previousLocation.current;
    previousLocation.current = location;
    const origin = previous.state?.campaignReturn;
    if (
      navigationType === 'POP' &&
      origin?.path === location.pathname + location.search + location.hash
    ) {
      const frame = requestAnimationFrame(() => {
        for (const [id, top, width] of origin.lists as [string, number, number][]) {
          const list = document.getElementById(id);
          if (list?.clientWidth === width) list.scrollTop = top;
        }
        window.scrollTo(0, origin.width === window.innerWidth ? origin.top : 0);
        const trigger = document.getElementById(origin.focus);
        trigger?.focus({ preventScroll: true });
        trigger?.scrollIntoView({ block: 'nearest' });
      });
      return () => cancelAnimationFrame(frame);
    }
    if (previous.pathname === location.pathname) return;
    if (location.search.includes('campaign=')) return;
    window.scrollTo(0, 0);
  }, [location, navigationType]);
  return (
    <div className="app-shell min-h-dvh">
      <a href="#main" className="sr-only fixed z-50 bg-soft p-3 text-canvas focus:not-sr-only">
        {t('skip_content')}
      </a>
      <aside className="app-sidebar">
        <div className="brand-row flex shrink-0 items-center px-4">
          <NavLink
            to="/"
            className="flex items-center gap-2 whitespace-nowrap text-base font-semibold tracking-tight"
          >
            <img
              src={logo}
              alt=""
              width={31}
              height={40}
              className="h-10 w-[31px] shrink-0 object-contain"
            />
            Drops Miner
          </NavLink>
        </div>
        <nav aria-label={t('navigation')} className="primary-nav">
          {links.map(([to, label, icon]) => (
            <NavLink
              key={to}
              to={to}
              end={to === '/'}
              className={({ isActive }) => `nav-item ${isActive ? 'active' : ''}`}
            >
              <Icon path={icon} className="mdi-icon" />
              {t(label)}
            </NavLink>
          ))}
        </nav>
        <div className="min-w-0 lg:mt-auto lg:p-4">
          <div className="flex min-w-0 items-center gap-2">
            <a
              className="icon-button hidden size-11 shrink-0 lg:inline-flex"
              href="https://github.com/ohne-b/twitch-drops-miner"
              target="_blank"
              rel="noreferrer"
              aria-label="GitHub repository"
              title="GitHub"
            >
              {/* The glyph fills 20 of its 24 viewBox units: render its artwork at 32px. */}
              <Icon path={mdiGithub} className="mdi-icon size-[38.4px]!" />
            </a>
            {isDesktop() && <DesktopUpdateLink />}
          </div>
          <div className="hidden lg:block">
            {data?.login.user_id != null && (
              <Link
                to="/settings#account"
                className="account-trigger mt-1 flex min-h-11 w-full items-center rounded text-start"
                aria-label={
                  data.login.profile
                    ? t('profile_open', { name: data.login.profile.display_name })
                    : t('account')
                }
                title={t('account')}
              >
                <AccountIdentity profile={data.login.profile} compact />
              </Link>
            )}
            {auth.enabled && (
              <Button
                className="mt-3 w-full"
                onClick={() => void onLogout().catch(() => setLogoutError(true))}
              >
                <Icon className="mdi-icon" path={mdiLogout} />
                {t('gui.auth.logout')}
              </Button>
            )}
          </div>
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="workspace min-w-0">
        <div
          className={`mx-auto max-w-[1440px] ${location.pathname === '/' ? (new URLSearchParams(location.search).get('edit') === 'priorities' ? 'preferences-frame' : 'mining-frame') : location.pathname === '/campaigns' ? 'campaigns-frame' : location.pathname === '/activity' ? 'activity-frame' : ''}`}
        >
          {!connected && (
            <div className="mb-5">
              <Notice>
                {t(incompatible ? 'client_outdated' : data ? 'disconnected_help' : 'connecting')}
                {isDesktop() && !incompatible && (
                  <Button
                    onClick={() => void invoke('restart_app').catch(() => setLogoutError(true))}
                  >
                    {t('gui.desktop.restart')}
                  </Button>
                )}
                {incompatible && (
                  <IconButton
                    path={mdiReload}
                    label={t('reload_dashboard')}
                    onClick={autosave.reload}
                  />
                )}
              </Notice>
            </div>
          )}
          {isDesktop() && <DesktopStatus />}
          {logoutError && <Notice error>{t('gui.auth.request_failed')}</Notice>}
          {auth.enabled && (
            <div className="mb-4 text-end lg:hidden">
              <Button onClick={() => void onLogout().catch(() => setLogoutError(true))}>
                {t('gui.auth.logout')}
              </Button>
            </div>
          )}
          {autosave.error && (
            <Notice error>
              {t(autosave.error)}{' '}
              <IconButton
                path={mdiReload}
                label={t('retry')}
                disabled={!connected || autosave.busy}
                onClick={() => void autosave.retry()}
              />
            </Notice>
          )}
          <Routes>
            <Route path="/" element={<Mining upNext={upNext} />} />
            <Route path="/campaigns" element={<Campaigns />} />
            <Route path="/history" element={<Navigate to="/campaigns?tab=history" replace />} />
            <Route path="/activity" element={<Activity />} />
            <Route
              path="/settings"
              element={
                location.hash === '#mining' ? (
                  <Navigate to="/?edit=priorities" replace />
                ) : (
                  <Settings auth={auth} />
                )
              }
            />
            <Route path="/login" element={<Navigate to="/" replace />} />
            <Route path="*" element={<Empty title={t('not_found')} />} />
          </Routes>
        </div>
      </main>
    </div>
  );
}
function BootSplash() {
  const t = useT();
  return <Empty title={t('loading')} />;
}
export default function App() {
  const [auth, setAuth] = useState<AuthStatus | null>(null);
  const [error, setError] = useState(false);
  const navigate = useNavigate();
  async function refresh() {
    const state = await request<AuthStatus>(`/api/auth/status?lang=${storedLanguage()}`);
    setAuth(state);
    setError(false);
    if (state.authenticated && location.pathname === '/login') navigate('/', { replace: true });
  }
  useEffect(() => {
    void refresh().catch(() => setError(true));
    const expire = () => {
      setAuth((current) => ({ ...current, enabled: true, authenticated: false }));
      navigate('/login', { replace: true });
    };
    window.addEventListener('auth-expired', expire);
    const updated = () => {
      void refresh().catch(() => setError(true));
    };
    window.addEventListener('auth-updated', updated);
    return () => {
      window.removeEventListener('auth-expired', expire);
      window.removeEventListener('auth-updated', updated);
    };
  }, []);
  if (!auth && error)
    return (
      <I18n language={storedLanguage()}>
        {isDesktop() ? <DesktopStartupError /> : <Login onLogin={refresh} statusError />}
      </I18n>
    );
  if (!auth)
    return (
      <I18n language={storedLanguage()}>
        <BootSplash />
      </I18n>
    );
  if (!auth.authenticated)
    return (
      <I18n messages={{ gui: { auth: auth.translations ?? {} } }} language={storedLanguage()}>
        <Login onLogin={refresh} />
      </I18n>
    );
  return (
    <MinerProvider>
      <DesktopUpdateProvider>
        <Shell
          auth={auth}
          onLogout={async () => {
            await request('/api/auth/logout', {});
            await refresh();
          }}
        />
      </DesktopUpdateProvider>
    </MinerProvider>
  );
}
