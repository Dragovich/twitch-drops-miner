import type { Filters } from '../../shared/lib/types';

const flags: (keyof Omit<Filters, 'game_name_search'>)[] = [
  'show_active',
  'show_upcoming',
  'show_finished',
  'show_only_not_linked',
  'hide_selected_games',
  'show_benefit_badge',
  'show_benefit_emote',
  'show_benefit_item',
  'show_benefit_other',
];

export function displayFilters(params: URLSearchParams, saved: Filters): Filters {
  const filters = { ...saved };
  for (const flag of flags) {
    if (params.get(flag) === '1') filters[flag] = true;
    if (params.get(flag) === '0') filters[flag] = false;
  }
  if (params.has('game'))
    filters.game_name_search = params.getAll('game').filter(Boolean).slice(0, 1000);
  return filters;
}

export function writeFilters(params: URLSearchParams, filters: Filters): URLSearchParams {
  const next = new URLSearchParams(params);
  next.delete('show_expired');
  for (const flag of flags) next.set(flag, filters[flag] ? '1' : '0');
  next.delete('game');
  for (const game of filters.game_name_search.length ? filters.game_name_search : [''])
    next.append('game', game);
  next.delete('page');
  return next;
}
