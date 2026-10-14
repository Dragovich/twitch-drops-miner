use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_casefold::UnicodeCaseFold;

pub fn fold(value: &str) -> String {
    value.case_fold().collect()
}

pub fn normalize_names(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && seen.insert(fold(value)))
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameMetadata {
    pub id: String,
    pub name: String,
    pub box_art_url: String,
}

impl GameMetadata {
    pub(crate) fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 20
            && self.id.bytes().all(|b| b.is_ascii_digit())
            && self.id.parse::<u64>().is_ok_and(|id| id > 0)
            && !self.name.trim().is_empty()
            && self.name.len() <= 1024
            && !self.name.chars().any(char::is_control)
            && self.box_art_url.len() <= 2048
            && (self.box_art_url.is_empty()
                || url::Url::parse(&self.box_art_url).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.host_str() == Some("static-cdn.jtvnw.net")
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.port().is_none()
                        && url.path().starts_with("/ttv-boxart/")
                }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Filters {
    pub game_name_search: Vec<String>,
    pub hide_selected_games: bool,
    pub show_benefit_badge: bool,
    pub show_benefit_emote: bool,
    pub show_benefit_item: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            game_name_search: vec![],
            hide_selected_games: false,
            show_benefit_badge: true,
            show_benefit_emote: true,
            show_benefit_item: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MiningPriorityMode {
    #[default]
    Manual,
    ShortEvents,
    EndingSoonest,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ru,
}

impl Language {
    /// Best-effort match of a BCP 47 tag or bare code; anything else stays English.
    /// Compares at most three leading characters, so hostile input cannot panic
    /// on a UTF-8 boundary or force a large allocation.
    pub fn from_code(value: &str) -> Self {
        let mut chars = value.trim().chars().flat_map(|c| c.to_lowercase());
        match (chars.next(), chars.next(), chars.next()) {
            (Some('r'), Some('u'), None) => Self::Ru,
            (Some('r'), Some('u'), Some(sep)) if sep == '-' || sep == '_' => Self::Ru,
            _ => Self::En,
        }
    }

    pub fn from_accept_language(header: &str) -> Self {
        // Only the highest-priority tag matters for the login screen.
        let tag = header
            .split(',')
            .next()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("");
        Self::from_code(tag)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub games_to_watch: Vec<String>,
    pub game_metadata: Vec<GameMetadata>,
    pub mining_paused: bool,
    pub language: Language,
    pub auto_claim_drops: bool,
    pub show_progress_percentage: bool,
    pub mining_priority_mode: MiningPriorityMode,
    pub auto_mine_badges: bool,
    pub auto_mine_emotes: bool,
    pub drop_name_blacklist: Vec<String>,
    pub inventory_filters: Filters,
    pub inventory_list_view: bool,
    pub mining_benefits: BTreeMap<String, bool>,
    pub proxy: String,
    pub connection_quality: u8,
    pub minimum_refresh_interval_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            games_to_watch: vec![],
            game_metadata: vec![],
            mining_paused: false,
            language: Language::default(),
            auto_claim_drops: true,
            show_progress_percentage: false,
            mining_priority_mode: MiningPriorityMode::Manual,
            auto_mine_badges: false,
            auto_mine_emotes: false,
            drop_name_blacklist: vec![],
            inventory_filters: Filters::default(),
            inventory_list_view: false,
            mining_benefits: ["BADGE", "DIRECT_ENTITLEMENT", "EMOTE", "UNKNOWN"]
                .into_iter()
                .map(|kind| (kind.to_owned(), true))
                .collect(),
            proxy: String::new(),
            connection_quality: 3,
            minimum_refresh_interval_minutes: 30,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid_settings")]
pub struct InvalidSettings;

impl Settings {
    pub fn from_saved(value: Value) -> Result<Self, InvalidSettings> {
        let mut result: Self = serde_json::from_value(value).map_err(|_| InvalidSettings)?;
        result.connection_quality = result.connection_quality.clamp(1, 6);
        result.normalize()?;
        Ok(result)
    }

    fn normalize(&mut self) -> Result<(), InvalidSettings> {
        self.games_to_watch = normalize_names(&self.games_to_watch);
        self.drop_name_blacklist = normalize_names(&self.drop_name_blacklist);
        self.inventory_filters.game_name_search =
            normalize_names(&self.inventory_filters.game_name_search);
        self.proxy = self.proxy.trim().to_owned();
        if !(1..=6).contains(&self.connection_quality)
            || !(1..=1440).contains(&self.minimum_refresh_interval_minutes)
            || self.games_to_watch.len() > 1000
            || self.drop_name_blacklist.len() > 1000
            || self
                .games_to_watch
                .iter()
                .chain(&self.drop_name_blacklist)
                .any(|name| name.len() > 1024)
            || self.proxy.len() > 4096
            || self.game_metadata.len() > 1000
            || self.game_metadata.iter().any(|game| !game.valid())
        {
            return Err(InvalidSettings);
        }
        let selected: HashSet<_> = self.games_to_watch.iter().map(|name| fold(name)).collect();
        let mut seen = HashSet::new();
        self.game_metadata.retain(|game| {
            let key = fold(&game.name);
            selected.contains(&key) && seen.insert(key)
        });
        validate_proxy(&self.proxy)?;
        self.mining_benefits.retain(|key, _| {
            matches!(
                key.as_str(),
                "BADGE" | "DIRECT_ENTITLEMENT" | "EMOTE" | "UNKNOWN"
            )
        });
        for (kind, enabled) in Self::default().mining_benefits {
            self.mining_benefits.entry(kind).or_insert(enabled);
        }
        Ok(())
    }

    pub fn patched(&self, patch: &Value) -> Result<Self, InvalidSettings> {
        let updates = patch.as_object().ok_or(InvalidSettings)?;
        let mut current = serde_json::to_value(self).map_err(|_| InvalidSettings)?;
        for (key, value) in updates {
            if value.is_null() || current.get(key).is_none() {
                continue;
            }
            if matches!(key.as_str(), "inventory_filters" | "mining_benefits") {
                let fields = value.as_object().ok_or(InvalidSettings)?;
                let target = current[key].as_object_mut().ok_or(InvalidSettings)?;
                for (name, value) in fields {
                    if target.contains_key(name) {
                        target.insert(name.clone(), value.clone());
                    }
                }
            } else {
                current[key] = value.clone();
            }
        }
        let mut result: Self = serde_json::from_value(current).map_err(|_| InvalidSettings)?;
        result.normalize()?;
        Ok(result)
    }

    pub fn selected(&self, name: &str) -> bool {
        let name = fold(name);
        self.games_to_watch.iter().any(|game| fold(game) == name)
    }

    pub fn game_priority(&self, name: Option<&str>) -> usize {
        name.and_then(|name| {
            let name = fold(name);
            self.games_to_watch
                .iter()
                .position(|game| fold(game) == name)
        })
        .unwrap_or(usize::MAX)
    }
}

pub fn validate_proxy(proxy: &str) -> Result<(), InvalidSettings> {
    if proxy.is_empty() {
        return Ok(());
    }
    let url = url::Url::parse(proxy).map_err(|_| InvalidSettings)?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none() && !url.scheme().starts_with("socks")
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(InvalidSettings);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_metadata_is_optional_bounded_and_kept_only_for_selected_names() {
        let metadata = json!({"id":"33214", "name":"Fortnite", "box_art_url":"https://static-cdn.jtvnw.net/ttv-boxart/33214-{width}x{height}.jpg"});
        let original = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert!(original.game_metadata.is_empty());
        let saved = original.patched(&json!({"games_to_watch":["fortnite"], "game_metadata":[metadata.clone(), metadata.clone()]})).unwrap();
        let restored = Settings::from_saved(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(restored.game_metadata.len(), 1);
        assert_eq!(restored.game_metadata[0].name, "Fortnite");
        let unicode = original.patched(&json!({"games_to_watch":["STRASSE"], "game_metadata":[{"id":"1", "name":"Straße", "box_art_url":""}]})).unwrap();
        let mut view = crate::dto::SettingsView {
            values: unicode,
            ..Default::default()
        };
        view.refresh_game_keys();
        assert_eq!(view.game_keys["Straße"], view.game_keys["STRASSE"]);
        assert!(original.patched(&json!({"games_to_watch": (0..1001).map(|n| format!("Game {n}")).collect::<Vec<_>>(), "game_metadata":[metadata.clone()]})).is_err());
        assert!(
            restored
                .patched(&json!({"games_to_watch":[]}))
                .unwrap()
                .game_metadata
                .is_empty()
        );
        for (field, value) in [
            ("id", "0"),
            ("id", "invalid"),
            ("name", "bad\nname"),
            ("box_art_url", "https://other.example/ttv-boxart/game.jpg"),
            (
                "box_art_url",
                "https://user:secret@static-cdn.jtvnw.net/ttv-boxart/game.jpg",
            ),
        ] {
            let mut invalid = metadata.clone();
            invalid[field] = value.into();
            assert!(
                original
                    .patched(&json!({"game_metadata":[invalid]}))
                    .is_err()
            );
        }
    }
    use serde_json::json;

    #[test]
    fn auto_claim_defaults_on_and_preserves_a_saved_opt_out() {
        let original = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert!(original.auto_claim_drops);
        let disabled = original
            .patched(&json!({"auto_claim_drops":false}))
            .unwrap();
        let updated = disabled.patched(&json!({"mining_paused":true})).unwrap();
        let restored = Settings::from_saved(serde_json::to_value(updated).unwrap()).unwrap();
        assert!(!restored.auto_claim_drops);
        assert!(restored.mining_paused);
        assert_eq!(restored.games_to_watch, original.games_to_watch);
        assert!(
            restored
                .patched(&json!({"auto_claim_drops":true}))
                .unwrap()
                .auto_claim_drops
        );
        for invalid in [json!("false"), json!(0), json!([])] {
            assert!(
                original
                    .patched(&json!({"auto_claim_drops":invalid}))
                    .is_err()
            );
            assert!(Settings::from_saved(json!({"auto_claim_drops":invalid})).is_err());
        }
    }

    #[test]
    fn language_defaults_to_english_round_trips_and_rejects_unknown() {
        let original = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert_eq!(original.language, Language::En);
        assert_eq!(serde_json::to_value(&original).unwrap()["language"], "en");
        let russian = original.patched(&json!({"language":"ru"})).unwrap();
        assert_eq!(russian.language, Language::Ru);
        assert_eq!(russian.games_to_watch, ["Rust"]);
        let encoded = serde_json::to_value(&russian).unwrap();
        assert!(Settings::from_saved(encoded).unwrap() == russian);
        assert_eq!(
            russian.patched(&json!({"language":"en"})).unwrap().language,
            Language::En
        );
        for invalid in [json!("de"), json!(""), json!(true), json!(1)] {
            assert!(original.patched(&json!({"language":invalid})).is_err());
            assert!(Settings::from_saved(json!({"language":invalid})).is_err());
        }
    }

    #[test]
    fn language_tags_match_russian_and_fall_back_to_english() {
        for tag in ["ru", "RU", " ru-RU ", "ru_RU", "ru-RU,ru;q=0.9,en;q=0.8"] {
            assert_eq!(Language::from_code(tag), Language::Ru);
        }
        for tag in ["", "en", "en-US", "de", "rus", "rux", "fr;q=0.9"] {
            assert_eq!(Language::from_code(tag), Language::En);
        }
        // Hostile multibyte input must not panic on a UTF-8 boundary.
        for tag in ["русский язык интерфейса", "ру", "rü", "r\u{301}u"] {
            assert_eq!(Language::from_code(tag), Language::En);
        }
        assert_eq!(
            Language::from_accept_language("ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7"),
            Language::Ru
        );
        assert_eq!(
            Language::from_accept_language("en-US,en;q=0.9"),
            Language::En
        );
        assert_eq!(Language::from_accept_language(""), Language::En);
    }

    #[test]
    fn pause_defaults_off_and_survives_settings_updates_and_reload() {
        let original = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert!(!original.mining_paused);
        let paused = original.patched(&json!({"mining_paused":true})).unwrap();
        let updated = paused.patched(&json!({"connection_quality":4})).unwrap();
        let restored = Settings::from_saved(serde_json::to_value(updated).unwrap()).unwrap();
        assert!(restored.mining_paused);
        assert_eq!(restored.games_to_watch, original.games_to_watch);
        assert!(
            !restored
                .patched(&json!({"mining_paused":false}))
                .unwrap()
                .mining_paused
        );
        assert!(original.patched(&json!({"mining_paused":"true"})).is_err());
    }

    #[test]
    fn connection_quality_defaults_to_three_and_preserves_saved_values() {
        assert_eq!(Settings::default().connection_quality, 3);
        assert_eq!(
            Settings::from_saved(json!({})).unwrap().connection_quality,
            3
        );
        for quality in 1..=6 {
            let settings = Settings::from_saved(json!({"connection_quality": quality})).unwrap();
            assert_eq!(settings.connection_quality, quality);
        }
    }

    #[test]
    fn mining_priority_defaults_round_trips_and_rejects_unknown_modes() {
        let original = Settings::from_saved(json!({"games_to_watch":["Rust", "Other"]})).unwrap();
        assert_eq!(
            serde_json::to_value(&original).unwrap()["mining_priority_mode"],
            "manual"
        );
        for mode in ["manual", "short_events", "ending_soonest"] {
            let updated = original
                .patched(&json!({"mining_priority_mode":mode}))
                .unwrap();
            let encoded = serde_json::to_value(&updated).unwrap();
            assert_eq!(encoded["mining_priority_mode"], mode);
            assert_eq!(updated.games_to_watch, original.games_to_watch);
            assert!(Settings::from_saved(encoded).unwrap() == updated);
            assert!(
                updated
                    .patched(&json!({"auto_mine_badges":true}))
                    .unwrap()
                    .games_to_watch
                    == original.games_to_watch
            );
        }
        for invalid in [json!("fastest"), json!(true), json!(1)] {
            assert!(
                original
                    .patched(&json!({"mining_priority_mode":invalid}))
                    .is_err()
            );
            assert!(Settings::from_saved(json!({"mining_priority_mode":invalid})).is_err());
        }
    }

    #[test]
    fn old_settings_default_automatic_types_off_and_patches_preserve_them() {
        let settings = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert!(!settings.auto_mine_badges && !settings.auto_mine_emotes);
        let settings = settings.patched(&json!({"auto_mine_emotes":true})).unwrap();
        let settings = settings.patched(&json!({"auto_mine_badges":true})).unwrap();
        assert!(settings.auto_mine_badges && settings.auto_mine_emotes);
        assert_eq!(settings.games_to_watch, ["Rust"]);
        let restored = Settings::from_saved(serde_json::to_value(&settings).unwrap()).unwrap();
        assert!(restored == settings);
        assert!(
            settings
                .patched(&json!({"auto_mine_badges":"true"}))
                .is_err()
        );
    }

    #[test]
    fn hide_selected_games_defaults_off_and_preserves_explicit_choices() {
        assert!(!Settings::default().inventory_filters.hide_selected_games);
        let old = Settings::from_saved(json!({"games_to_watch":["Rust"],
            "inventory_filters":{"show_benefit_emote":false}}))
        .unwrap();
        assert!(!old.inventory_filters.hide_selected_games);
        assert!(!old.inventory_filters.show_benefit_emote);
        for enabled in [true, false] {
            let saved = old
                .patched(&json!({"inventory_filters":{"hide_selected_games":enabled}}))
                .unwrap();
            let restored = Settings::from_saved(serde_json::to_value(&saved).unwrap()).unwrap();
            assert_eq!(restored.inventory_filters.hide_selected_games, enabled);
            assert!(!restored.inventory_filters.show_benefit_emote);
            assert_eq!(restored.games_to_watch, ["Rust"]);
        }
        assert!(
            old.patched(&json!({"inventory_filters":{"hide_selected_games":"true"}}))
                .is_err()
        );
    }

    #[test]
    fn progress_percentage_defaults_off_and_survives_settings_updates() {
        let old = Settings::from_saved(json!({"games_to_watch":["Rust"]})).unwrap();
        assert!(!old.show_progress_percentage);
        let enabled = old
            .patched(&json!({"show_progress_percentage":true}))
            .unwrap();
        let updated = enabled.patched(&json!({"mining_paused":true})).unwrap();
        let restored = Settings::from_saved(serde_json::to_value(&updated).unwrap()).unwrap();
        assert!(restored.show_progress_percentage);
        assert!(restored.mining_paused);
        assert_eq!(restored.games_to_watch, ["Rust"]);
        assert!(
            !restored
                .patched(&json!({"show_progress_percentage":false}))
                .unwrap()
                .show_progress_percentage
        );
        assert!(
            old.patched(&json!({"show_progress_percentage":"true"}))
                .is_err()
        );
    }

    #[test]
    fn defaults_and_partial_updates_preserve_selection() {
        let settings = Settings::default()
            .patched(
                &json!({"games_to_watch": [" Rust ", "rust", "Straße", "STRASSE"],
                "drop_name_blacklist": [" coat ", "", "COAT"],
                "inventory_filters": {"show_benefit_emote": false}}),
            )
            .unwrap();
        assert_eq!(settings.games_to_watch, ["Rust", "Straße"]);
        assert_eq!(settings.drop_name_blacklist, ["coat"]);
        assert!(settings.inventory_filters.show_benefit_badge);
        assert!(!settings.inventory_filters.show_benefit_emote);
        assert!(settings.selected("STRASSE"));
        assert!(!Settings::default().selected("Rust"));
    }

    #[test]
    fn retired_filters_are_ignored_without_resetting_display_or_mining_choices() {
        let saved = json!({
            "inventory_filters_version": 2,
            "games_to_watch": ["Rust"],
            "mining_benefits": {"EMOTE": false},
            "inventory_filters": {
                "game_name_search": [" Sea of Thieves "],
                "show_active": false, "show_upcoming": false, "show_expired": true,
                "show_finished": true, "show_only_not_linked": true, "show_benefit_other": false,
                "show_benefit_badge": false, "show_benefit_emote": false,
                "hide_selected_games": true
            }
        });
        let settings = Settings::from_saved(saved).unwrap();
        let filters = &settings.inventory_filters;
        assert_eq!(filters.game_name_search, ["Sea of Thieves"]);
        assert!(filters.hide_selected_games && filters.show_benefit_item);
        assert!(!filters.show_benefit_badge && !filters.show_benefit_emote);
        assert_eq!(settings.games_to_watch, ["Rust"]);
        assert!(!settings.mining_benefits["EMOTE"]);
        let encoded = serde_json::to_value(&settings).unwrap();
        assert!(encoded.get("inventory_filters_version").is_none());
        assert_eq!(encoded["inventory_filters"].as_object().unwrap().len(), 5);
        assert!(Settings::from_saved(encoded).unwrap() == settings);
        let patched = settings
            .patched(&json!({
                "inventory_filters_version": 999,
                "inventory_filters": {"show_only_not_linked": true, "show_benefit_other": true}
            }))
            .unwrap();
        assert!(patched == settings);
    }

    #[test]
    fn invalid_patches_are_rejected_without_echoing_input() {
        for patch in [
            json!({"connection_quality": 0}),
            json!({"connection_quality": 7}),
            json!({"minimum_refresh_interval_minutes": 0}),
            json!({"games_to_watch": "Rust"}),
            json!({"proxy": "http://secret@example.invalid/path"}),
            json!({"inventory_filters": {"show_benefit_badge": "yes"}}),
        ] {
            assert_eq!(
                Settings::default()
                    .patched(&patch)
                    .err()
                    .unwrap()
                    .to_string(),
                "invalid_settings"
            );
        }
    }

    #[test]
    fn unknown_saved_fields_never_reach_serialization() {
        let settings = Settings::from_saved(json!({
            "theme": "dark", "dark_mode": true,
            "telegram_bot_token": "secret", "telegram_chat_id": "123",
            "games_to_watch": ["Rust"]
        }))
        .unwrap();
        let encoded = serde_json::to_string(&settings).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("telegram"));
        assert!(!encoded.contains("theme"));
        assert!(settings.selected("Rust"));
    }

    #[test]
    fn partial_saved_benefit_maps_merge_defaults_and_remain_editable() {
        let settings =
            Settings::from_saved(json!({"mining_benefits":{"EMOTE":false,"retired":true}}))
                .unwrap();
        assert!(!settings.mining_benefits["EMOTE"]);
        assert!(settings.mining_benefits["DIRECT_ENTITLEMENT"]);
        assert_eq!(settings.mining_benefits.len(), 4);
        let updated = settings
            .patched(&json!({"mining_benefits":{"BADGE":false}}))
            .unwrap();
        assert!(!updated.mining_benefits["BADGE"]);
        assert!(!updated.mining_benefits["EMOTE"]);
        assert!(updated.mining_benefits["UNKNOWN"]);
    }
}
