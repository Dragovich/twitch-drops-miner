import { createContext, useContext, useEffect, type ReactNode } from 'react';
import english from '../../../../lang/English.json';
import russian from '../../../../lang/Russian.json';
type Dictionary = { [key: string]: string | string[] | Dictionary };
export type Language = 'en' | 'ru';
type Translate = (key: string, parameters?: Record<string, string | number>) => string;
const dictionaries: Record<Language, Dictionary> = { en: english, ru: russian };
const storageKey = 'tdm-language';
export function resolveLanguage(value: unknown): Language {
  return value === 'ru' ? 'ru' : 'en';
}
export function storedLanguage(): Language {
  try {
    return resolveLanguage(localStorage.getItem(storageKey));
  } catch {
    return 'en';
  }
}
export function storeLanguage(language: Language) {
  try {
    localStorage.setItem(storageKey, language);
  } catch {
    /* Storage can be disabled. */
  }
}
function lookup(dictionary: Dictionary, key: string): string | undefined {
  let value: string | string[] | Dictionary | undefined = dictionary;
  for (const part of key.split('.'))
    value = value && typeof value === 'object' && !Array.isArray(value) ? value[part] : undefined;
  return typeof value === 'string' ? value : undefined;
}
// Keep log/status presentation consistent with the neutral interface.
export const plainText = (text: string) =>
  text
    .replace(/[\p{Extended_Pictographic}\uFE0F\u2713\u2714\u274C\u23F3]/gu, '')
    .replace(/ {2,}/g, ' ')
    .trim();
export function translator(dictionary: Dictionary): Translate {
  return (key, parameters = {}) => {
    const path = key.includes('.') ? key : `gui.redesign.${key}`;
    const text = lookup(dictionary, path) ?? lookup(english, path) ?? key;
    return plainText(
      text.replace(/\{(\w+)\}/g, (match: string, name: string) =>
        String(parameters[name] ?? match),
      ),
    );
  };
}
const Context = createContext<Translate>(translator(english));
export function I18n({
  messages,
  language = 'en',
  children,
}: {
  messages?: Dictionary;
  language?: Language;
  children: ReactNode;
}) {
  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);
  const base = dictionaries[language];
  let dictionary = base;
  const auth =
    messages?.gui && typeof messages.gui === 'object' && !Array.isArray(messages.gui)
      ? messages.gui.auth
      : undefined;
  if (auth && typeof auth === 'object' && !Array.isArray(auth)) {
    const gui = base.gui;
    dictionary = {
      ...base,
      gui: { ...(typeof gui === 'object' && !Array.isArray(gui) ? gui : {}), auth },
    };
  }
  return <Context value={translator(dictionary)}>{children}</Context>;
}
export const useT = () => useContext(Context);
