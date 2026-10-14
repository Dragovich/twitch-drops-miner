import { expect, it } from 'vitest';
import russian from '../../lang/Russian.json';
import { resolveLanguage, storedLanguage, translator } from '../src/shared/lib/i18n';

it('resolves saved language codes with an English default', () => {
  expect(resolveLanguage('ru')).toBe('ru');
  expect(resolveLanguage('en')).toBe('en');
  for (const value of [undefined, null, '', 'de', 'RU']) expect(resolveLanguage(value)).toBe('en');
  expect(storedLanguage()).toBe('en');
});

it('translates dashboard text to Russian with an English fallback', () => {
  const t = translator(russian);
  expect(t('watch_channel', { channel: 'northwind' })).toBe('Смотреть northwind');
  expect(t('gui.channels.viewers')).toBe('зрителей');
  expect(t('minutes_progress', { current: 42, total: 60 })).toBe('42 / 60 мин');
  expect(t('language')).toBe('Язык');
  expect(t('telegram.claim_title')).toBe('Дроп получен');
  expect(translator({})('language')).toBe('Language');
});
