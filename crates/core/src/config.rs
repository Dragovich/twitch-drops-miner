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
    pub show_active: bool,
    pub show_upcoming: bool,
    pub show_expired: bool,
    pub show_finished: bool,
    pub show_only_not_linked: bool,
    pub show_benefit_badge: bool,
    pub show_benefit_emote: bool,
    pub show_benefit_item: bool,
    pub show_benefit_other: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            game_name_search: vec![],
            show_active: true,
            show_upcoming: true,
            show_expired: false,
            show_finished: false,
            show_only_not_linked: false,
            show_benefit_badge: true,
            show_benefit_emote: true,
            show_benefit_item: true,
            show_benefit_other: true,
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

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub games_to_watch: Vec<String>,
    pub game_metadata: Vec<GameMetadata>,
    pub mining_paused: bool,
    pub auto_claim_drops: bool,
    pub show_progress_percentage: bool,
    pub mining_priority_mode: MiningPriorityMode,
    pub auto_mine_badges: bool,
    pub auto_mine_emotes: bool,
    pub drop_name_blacklist: Vec<String>,
    pub inventory_filters: Filters,
    pub inventory_filters_version: u32,
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
            auto_claim_drops: true,
            show_progress_percentage: false,
            mining_priority_mode: MiningPriorityMode::Manual,
            auto_mine_badges: false,
            auto_mine_emotes: false,
            drop_name_blacklist: vec![],
            inventory_filters: Filters::default(),
            inventory_filters_version: 2,
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
    pub fn from_saved(mut value: Value) -> Result<Self, InvalidSettings> {
        let mut legacy = serde_json::to_value(Filters::default()).map_err(|_| InvalidSettings)?;
        legacy["show_active"] = false.into();
        if value.get("inventory_filters_version").is_none()
            && value.get("inventory_filters") == Some(&legacy)
        {
            value["inventory_filters"] =
                serde_json::to_value(Filters::default()).map_err(|_| InvalidSettings)?;
        }
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
        self.inventory_filters_version = 2;
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
            if value.is_null() || current.get(key).is_none() || key == "inventory_filters_version" {
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
                "inventory_filters": {"show_upcoming": false}}),
            )
            .unwrap();
        assert_eq!(settings.games_to_watch, ["Rust", "Straße"]);
        assert_eq!(settings.drop_name_blacklist, ["coat"]);
        assert!(settings.inventory_filters.show_active);
        assert!(!settings.inventory_filters.show_upcoming);
        assert!(settings.selected("STRASSE"));
        assert!(!Settings::default().selected("Rust"));
    }

    #[test]
    fn only_old_default_filters_migrate() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        old.as_object_mut()
            .unwrap()
            .remove("inventory_filters_version");
        old["inventory_filters"]["show_active"] = false.into();
        assert!(
            Settings::from_saved(old.clone())
                .unwrap()
                .inventory_filters
                .show_active
        );
        old["inventory_filters"]["show_expired"] = true.into();
        assert!(
            !Settings::from_saved(old)
                .unwrap()
                .inventory_filters
                .show_active
        );
    }

    #[test]
    fn invalid_patches_are_rejected_without_echoing_input() {
        for patch in [
            json!({"connection_quality": 0}),
            json!({"connection_quality": 7}),
            json!({"minimum_refresh_interval_minutes": 0}),
            json!({"games_to_watch": "Rust"}),
            json!({"proxy": "http://secret@example.invalid/path"}),
            json!({"inventory_filters": {"show_active": "yes"}}),
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
            "language": "old", "dark_mode": true,
            "telegram_bot_token": "secret", "telegram_chat_id": "123",
            "games_to_watch": ["Rust"]
        }))
        .unwrap();
        let encoded = serde_json::to_string(&settings).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("telegram"));
        assert!(!encoded.contains("language"));
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
