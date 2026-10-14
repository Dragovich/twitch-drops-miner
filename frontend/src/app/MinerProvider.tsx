import { isDesktop } from '../shared/lib/platform';
import { createContext, useContext, useEffect, useReducer, useRef, type ReactNode } from 'react';
import type { Snapshot } from '../shared/lib/types';
import { connectBrowser, type Subscription } from './transport';
import { I18n, resolveLanguage, storeLanguage, storedLanguage } from '../shared/lib/i18n';
import { useAutosave } from './useAutosave';
import { initialState, reducer } from './reducer';

export function upsert<T extends { id: string | number }>(items: T[], item: T): T[] {
  return items.some((current) => current.id === item.id)
    ? items.map((current) => (current.id === item.id ? item : current))
    : [...items, item];
}
const Context = createContext<{
  data: Snapshot | null;
  connected: boolean;
  incompatible: boolean;
  historyRevision: number;
  autosave: ReturnType<typeof useAutosave>;
}>({
  data: null,
  connected: false,
  incompatible: false,
  historyRevision: 0,
  autosave: null as unknown as ReturnType<typeof useAutosave>,
});

export function MinerProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initialState);
  const transport = useRef<Subscription | null>(null);
  const autosave = useAutosave(state.data?.settings, state.hydrated, (settings, revision) => {
    dispatch({ type: 'settings', settings, revision });
  });
  useEffect(() => {
    if (state.resync) transport.current?.resync();
  }, [state.resync]);
  const savedLanguage = state.data?.settings.language;
  const language = savedLanguage ? resolveLanguage(savedLanguage) : storedLanguage();
  useEffect(() => {
    // The saved setting is authoritative once snapshots arrive.
    if (savedLanguage) storeLanguage(resolveLanguage(savedLanguage));
  }, [savedLanguage]);
  useEffect(() => {
    let disposed = false;
    const start = async () => {
      const connect = isDesktop()
        ? (await import('../shared/lib/desktop')).connectDesktop
        : connectBrowser;
      if (disposed) return;
      transport.current = connect(dispatch);
    };
    void start();
    return () => {
      disposed = true;
      transport.current?.close();
      transport.current = null;
    };
  }, []);
  return (
    <Context
      value={{
        data: state.data,
        connected: state.hydrated,
        incompatible: state.incompatible,
        historyRevision: state.data?.history_clear_revision ?? 0,
        autosave,
      }}
    >
      <I18n language={language}>{children}</I18n>
    </Context>
  );
}
export const useMiner = () => useContext(Context);
