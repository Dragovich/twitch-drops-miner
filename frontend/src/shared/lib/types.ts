export interface Benefit {
  name: string;
  type: string;
  image_url: string;
}

export interface GameMetadata {
  id: string;
  name: string;
  box_art_url: string;
}
export interface Drop {
  eligibility?: Eligibility;
  prerequisites?: string[];
  effective_starts_at?: string | null;
  effective_ends_at?: string | null;
  priority_deadline?: string | null;
  confirmed_at?: string | null;
  id: string;
  name: string;
  current_minutes: number;
  confirmed_minutes?: number;
  required_minutes: number;
  progress: number;
  is_claimed: boolean;
  can_claim: boolean;
  is_ignored: boolean;
  is_mineable: boolean;
  is_skipped: boolean;
  ignored_reason?: string | null;
  ignored_keyword?: string | null;
  ignored_precondition?: string | null;
  benefits: Benefit[];
  starts_at: string;
  ends_at: string;
}
export interface Campaign {
  game_key?: string;
  selected?: boolean;
  saved_rank?: number | null;
  priority?: PriorityContext;
  allowed_channels?: { login: string; name: string }[];
  id: string;
  name: string;
  game_name: string;
  game_box_art_url: string;
  campaign_url: string;
  link_url: string;
  starts_at: string;
  ends_at: string;
  linked: boolean | null;
  active: boolean;
  upcoming: boolean;
  expired: boolean;
  finished: boolean;
  mining_finished?: boolean;
  claimed_drops: number;
  total_drops: number;
  ignored_drops: number;
  skipped_drops: number;
  drops: Drop[];
}
export interface ChannelSearchResult {
  id: number;
  login: string;
  name: string;
  avatar_url: string | null;
  online: boolean;
  game: string | null;
}
export interface Channel {
  avatar_url?: string | null;
  id: number;
  login?: string;
  name: string;
  game: string | null;
  game_id: number | null;
  game_icon: string | null;
  viewers: number | null;
  online: boolean;
  drops_enabled: boolean;
  acl_based: boolean;
  watching: boolean;
}
export interface Filters {
  game_name_search: string[];
  hide_selected_games: boolean;
  show_benefit_badge: boolean;
  show_benefit_emote: boolean;
  show_benefit_item: boolean;
}
export interface Settings {
  game_metadata: GameMetadata[];
  revision?: string;
  games_to_watch: string[];
  mining_paused: boolean;
  language: 'en' | 'ru';
  auto_claim_drops: boolean;
  show_progress_percentage: boolean;
  mining_priority_mode: 'manual' | 'short_events' | 'ending_soonest';
  auto_mine_badges: boolean;
  auto_mine_emotes: boolean;
  games_available: string[];
  game_keys?: Record<string, string>;
  drop_name_blacklist: string[];
  proxy: string;
  connection_quality: number;
  minimum_refresh_interval_minutes: number;
  mining_benefits: Record<string, boolean>;
  inventory_filters: Filters;
  inventory_list_view: boolean;
}
export interface OAuth {
  url: string;
  code: string;
}
export interface TwitchLogin {
  status: string;
  user_id: number | null;
  profile?: AccountProfile;
  oauth_pending?: OAuth;
}
export interface AccountBadge {
  id: string;
  title: string;
  description: string | null;
  image_url: string | null;
}
export interface AccountProfile {
  login?: string;
  display_name: string;
  avatar_url: string | null;
  color: string | null;
  badges: AccountBadge[];
  available_badges: AccountBadge[] | null;
}
export interface Progress {
  drop_id: string;
  drop_name: string;
  campaign_id: string;
  campaign_name: string;
  game_name: string;
  current_minutes: number;
  confirmed_minutes?: number;
  confirmed_at?: string | null;
  required_minutes: number;
  progress: number;
  remaining_seconds: number;
}
export interface WantedGame {
  game_key?: string;
  saved_rank?: number | null;
  game_name: string;
  game_icon?: string;
  game_id?: number;
  campaigns: {
    priority?: PriorityContext;
    id: string;
    name: string;
    url: string;
    drops: {
      id?: string;
      name: string;
      benefits: string[];
      image_url?: string;
      eligibility?: Eligibility;
      starts_at?: string;
      ends_at?: string;
      prerequisites?: string[];
      priority_deadline?: string | null;
    }[];
  }[];
}
export interface ManualMode {
  channel_id?: number;
  active: boolean;
  game_name?: string;
  channel_name?: string;
  expires_at?: string;
  pending_channel?: string;
  error?: string;
}
export interface InventoryStatus {
  available: boolean;
  checked_at: string | null;
  catalog_updated_at?: string | null;
}
export interface InventoryRefresh {
  sequence: number;
  state: 'idle' | 'refreshing' | 'refreshed' | 'failed';
  error: string | null;
}
export interface Snapshot {
  mining?: {
    state:
      | 'unknown'
      | 'account_required'
      | 'paused'
      | 'no_selection'
      | 'discovering'
      | 'watching'
      | 'awaiting_claim'
      | 'awaiting_progress'
      | 'waiting_channel'
      | 'no_rewards'
      | 'manual_offline'
      | 'manual_watching';
    channel_id: number | null;
    campaign_id: string | null;
    drop_id: string | null;
    priority: PriorityContext;
  };
  protocol: number;
  instance: string;
  revision: number;
  history_revision: number;
  history_clear_revision: number;
  activity: ActivityEvent[];
  status: string;
  channels: Channel[];
  campaigns: Campaign[];
  console: string[];
  settings: Settings;
  login: TwitchLogin;
  manual_mode: ManualMode;
  current_drop: Progress | null;
  wanted_items: WantedGame[];
  inventory_status?: InventoryStatus;
  inventory_refresh?: InventoryRefresh;
}
export interface HistoryEntry {
  image_url?: string;
  id: string;
  claimed_at: string;
  claimed_at_is_observed?: boolean;
  game: string;
  campaign: string;
  drop_name: string;
  benefits: string[];
  required_minutes: number;
  campaign_id: string;
}
export interface AuthStatus {
  enabled: boolean;
  authenticated: boolean;
  translations?: Record<string, string>;
}
export interface Result {
  success: boolean;
  message?: string;
}
export interface ServerEvents {
  state_snapshot: (data: Snapshot) => void;
  state_patch: (data: StatePatch) => void;
  protocol_mismatch: (data: { protocol: number }) => void;
  initial_state: (data: Snapshot) => void;
  status_update: (data: { status: string }) => void;
  console_output: (data: { message: string }) => void;
  channel_add: (data: Channel) => void;
  channel_update: (data: Channel) => void;
  channel_remove: (data: { id: number }) => void;
  channels_clear: () => void;
  channels_batch_update: (data: { channels: Channel[] }) => void;
  channel_watching: (data: { id: number }) => void;
  channel_watching_clear: () => void;
  drop_progress: (data: Progress) => void;
  drop_progress_stop: () => void;
  campaign_add: (data: Campaign) => void;
  inventory_clear: () => void;
  history_cleared: () => void;
  inventory_batch_update: (data: { campaigns: Campaign[] }) => void;
  inventory_status: (data: InventoryStatus) => void;
  inventory_refresh: (data: InventoryRefresh) => void;
  drop_update: (data: {
    campaign_id: string;
    campaign: Partial<Campaign>;
    drop: Drop;
    drops?: Drop[];
  }) => void;
  login_required: () => void;
  oauth_code_required: (data: OAuth) => void;
  login_status: (data: TwitchLogin) => void;
  login_clear: (data: { login: boolean; password: boolean; token: boolean }) => void;
  settings_updated: (data: Settings) => void;
  games_available: (data: { games: string[] }) => void;
  manual_mode_update: (data: ManualMode) => void;
  wanted_items_update: (data: WantedGame[]) => void;
  notification: (data: { title: string; message: string }) => void;
  attention_required: (data: { sound: boolean }) => void;
}
export interface ReleaseInfo {
  current_version: string;
  latest_version: string | null;
  update_available: boolean;
  check_succeeded: boolean;
  download_url: string;
}
export type Eligibility =
  | 'unknown'
  | 'ready'
  | 'claimed'
  | 'awaiting_claim'
  | 'upcoming'
  | 'expired'
  | 'ignored'
  | 'prerequisite'
  | 'filtered'
  | 'unselected'
  | 'awaiting_confirmation';
export interface PriorityContext {
  reason: 'saved_order' | 'short_event' | 'ending_soonest' | 'automatic_reward' | 'not_selected';
  deadline: string | null;
  target_ids: string[];
}
export interface ActivityEvent {
  id: number;
  first_at: string;
  last_at: string;
  category: 'account' | 'mining' | 'inventory' | 'claims' | 'connection';
  severity: 'info' | 'warning' | 'error';
  code: string;
  args: Record<string, string>;
  message: string;
  campaign_id: string | null;
  drop_id: string | null;
  channel_id: number | null;
  count: number;
  recovered: boolean;
}
export interface StatePatch {
  protocol: number;
  instance: string;
  base_revision: number;
  revision: number;
  changes: Partial<Omit<Snapshot, 'protocol' | 'instance' | 'revision'>>;
}
