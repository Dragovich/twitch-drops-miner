use std::{cmp::Reverse, collections::BTreeMap, sync::LazyLock};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration, Utc};
use http::{Method, StatusCode};
use regex::Regex;
use serde_json::{Value, json};
use url::Url;

use super::{
    RetryPolicy, TwitchClient, TwitchError, diagnostics,
    inventory::values,
    operations::{Operation, directory},
    success,
};
use crate::{
    config::Settings,
    domain::{Campaign, Channel, ChannelIdentity, Game, MiningPriority, number},
};

pub const MAX_CHANNELS: usize = 199;
static BEACON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)"beacon_?url"\s*:\s*"([^"\s]+)""#).unwrap());
static SETTINGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)src="(https?://[^"\s]+/config/settings\.[0-9a-f]{32}\.js)""#).unwrap()
});

/// Accept only a Twitch login or a channel root URL, never an arbitrary fetch target.
pub fn channel_login(input: &str) -> Option<String> {
    if input.len() > 256 {
        return None;
    }
    let input = input.trim();
    let login = if input.contains('/') || input.contains(':') {
        let url = Url::parse(input).ok()?;
        if !matches!(url.scheme(), "https" | "http")
            || !matches!(
                url.host_str(),
                Some("twitch.tv" | "www.twitch.tv" | "m.twitch.tv")
            )
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        url.path().trim_matches('/').to_owned()
    } else {
        input.trim_start_matches('@').to_owned()
    };
    (1..=25).contains(&login.len()).then_some(())?;
    login
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then(|| login.to_ascii_lowercase())
}

impl TwitchClient {
    pub async fn resolve_channel(&self, login: &str) -> Result<Option<Channel>, TwitchError> {
        let login = channel_login(login).ok_or(TwitchError::InvalidResponse)?;
        let response = self
            .gql(Operation::StreamInfo.request(json!({"channel":login})))
            .await?;
        let user = &response["data"]["user"];
        if user.is_null() {
            return Ok(None);
        }
        let identity = ChannelIdentity {
            id: number(&user["id"]).ok_or_else(|| {
                diagnostics::invalid(
                    "StreamInfo",
                    "user.id must be an unsigned integer",
                    user.get("id"),
                )
            })?,
            name: user["displayName"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(&login)
                .to_owned(),
            login,
        };
        let mut channel = Channel::offline(identity, false);
        update_stream(&mut channel, user);
        let drops_enabled = if channel.online() {
            // Reward metadata is optional for an explicit watch request. Keep it
            // off the critical path after this short attempt; auth still fails closed.
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                self.gql(
                    Operation::AvailableDrops
                        .request(json!({"channelID":channel.identity.id.to_string()})),
                ),
            )
            .await
            {
                Ok(Ok(response)) => values(&response["data"]["channel"]["viewerDropCampaigns"])
                    .any(|v| v["id"].as_str().is_some_and(|id| !id.is_empty())),
                Ok(Err(error @ (TwitchError::Unauthorized | TwitchError::Cancelled))) => {
                    return Err(error);
                }
                _ if self.http.cancel.is_cancelled() => return Err(TwitchError::Cancelled),
                _ => false,
            }
        } else {
            false
        };
        channel.drops_enabled = drops_enabled;
        Ok(Some(channel))
    }

    pub async fn update_manual_channel(&self, channel: &mut Channel) -> Result<(), TwitchError> {
        let resolved = self.resolve_channel(&channel.identity.login).await?;
        let mut fresh = resolved
            .filter(|r| r.identity.id == channel.identity.id)
            .unwrap_or_else(|| Channel::offline(channel.identity.clone(), channel.acl_based));
        fresh.acl_based = channel.acl_based;
        if fresh.broadcast_id == channel.broadcast_id {
            fresh.beacon_url = channel.beacon_url.clone();
        }
        *channel = fresh;
        Ok(())
    }

    pub async fn channels(
        &self,
        campaigns: &[Campaign],
        settings: &Settings,
        current: Option<&Channel>,
    ) -> Result<Vec<Channel>, TwitchError> {
        let now = Utc::now();
        let mut campaigns: Vec<_> = campaigns
            .iter()
            .filter(|c| c.can_earn_within(settings, now, now + Duration::hours(1)))
            .collect();
        campaigns.sort_by_cached_key(|c| c.mining_priority(settings, now));
        let mut channels = BTreeMap::new();
        let mut directories = BTreeMap::new();
        for campaign in &campaigns {
            if !campaign.allowed_channels.is_empty() {
                for identity in &campaign.allowed_channels {
                    channels
                        .entry(identity.id)
                        .or_insert_with(|| Channel::offline(identity.clone(), true));
                }
            } else {
                directories
                    .entry(campaign.game.id)
                    .or_insert(&campaign.game);
            }
        }
        if let Some(current) = current {
            channels
                .entry(current.identity.id)
                .or_insert_with(|| Channel::offline(current.identity.clone(), current.acl_based));
        }
        let mut restricted: Vec<_> = channels.into_values().collect();
        // Check the participating lists before trimming: an offline popular game
        // must not hide a live participant farther down its campaign ACL.
        self.update_channels(&mut restricted).await?;
        let mut channels: BTreeMap<_, _> =
            restricted.into_iter().map(|c| (c.identity.id, c)).collect();
        let games: Vec<_> = directories.into_values().collect();
        for games in games.chunks(20) {
            let responses = self
                .batch(games.iter().map(|g| directory(&g.slug, 20)).collect())
                .await?;
            for (game, response) in games.iter().zip(responses) {
                for edge in values(&response["data"]["game"]["streams"]["edges"]).take(20) {
                    if let Some(channel) = directory_channel(&edge["node"], game) {
                        channels
                            .entry(channel.identity.id)
                            .and_modify(|old| {
                                let acl = old.acl_based;
                                *old = channel.clone();
                                old.acl_based = acl;
                            })
                            .or_insert(channel);
                    }
                }
            }
        }
        if let Some(current) = current
            && let Some(updated) = channels.get_mut(&current.identity.id)
            && updated.broadcast_id == current.broadcast_id
        {
            updated.beacon_url = current.beacon_url.clone();
        }
        let mut channels: Vec<_> = channels.into_values().collect();
        let mineable: Vec<_> = campaigns
            .iter()
            .filter(|c| c.can_mine(settings, now))
            .map(|c| (*c, c.mining_priority(settings, now)))
            .collect();
        channels.sort_by_key(|c| {
            (
                channel_priority(c, &mineable),
                Reverse(c.acl_based),
                Reverse(c.viewers),
                c.identity.id,
            )
        });
        // Preserve the watched row through a settings rebuild. Eligibility is still
        // checked against the new settings before another beacon can be sent.
        if let Some(current) = current
            && let Some(index) = channels
                .iter()
                .position(|c| c.identity.id == current.identity.id)
        {
            let current = channels.remove(index);
            channels.insert(0, current);
        }
        channels.truncate(MAX_CHANNELS);
        Ok(channels)
    }

    pub async fn update_channels(&self, channels: &mut [Channel]) -> Result<(), TwitchError> {
        for channels in channels.chunks_mut(10) {
            let queries = channels
                .iter()
                .flat_map(|c| {
                    [
                        Operation::StreamInfo.request(json!({"channel":c.identity.login})),
                        Operation::AvailableDrops
                            .request(json!({"channelID":c.identity.id.to_string()})),
                    ]
                })
                .collect();
            let responses = self.batch(queries).await?;
            for (channel, responses) in channels.iter_mut().zip(responses.chunks_exact(2)) {
                let user = &responses[0]["data"]["user"];
                update_stream(channel, user);
                channel.drops_enabled = channel.online()
                    && values(&responses[1]["data"]["channel"]["viewerDropCampaigns"])
                        .any(|v| v["id"].is_string());
            }
        }
        Ok(())
    }

    async fn page(&self, url: Url) -> Result<String, TwitchError> {
        let response = self
            .http
            .execute(self.http.request(Method::GET, url), RetryPolicy::Replay)
            .await?;
        success(response.status())?;
        String::from_utf8(response.into_body()).map_err(|error| {
            tracing::warn!(
                valid_up_to = error.utf8_error().valid_up_to(),
                "Twitch page is not UTF-8"
            );
            TwitchError::InvalidResponse
        })
    }

    fn trusted_url(&self, raw: &str) -> Result<Url, TwitchError> {
        let url =
            Url::parse(raw).map_err(|_| diagnostics::invalid("Beacon", "malformed URL", None))?;
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(diagnostics::invalid(
                "Beacon",
                "URL contains credentials or fragment",
                None,
            ));
        }
        #[cfg(test)]
        if url.origin() == self.http.endpoints.web.origin() {
            return Ok(url);
        }
        let trusted = url.scheme() == "https"
            && url.port().is_none()
            && url.host_str().is_some_and(|host| {
                ["twitch.tv", "jtvnw.net", "ttvnw.net"]
                    .iter()
                    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            });
        if trusted {
            Ok(url)
        } else {
            Err(diagnostics::invalid(
                "Beacon",
                "URL origin is not trusted",
                None,
            ))
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn beacon(&self, channel: &Channel) -> Result<Url, TwitchError> {
        if channel.identity.login.is_empty()
            || !channel
                .identity
                .login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(TwitchError::InvalidResponse);
        }
        let url = self
            .http
            .endpoints
            .web
            .join(&channel.identity.login)
            .map_err(|_| TwitchError::InvalidResponse)?;
        let page = self.page(url).await?;
        if let Some(beacon) = BEACON.captures(&page) {
            return self.trusted_url(&beacon[1]);
        }
        let settings = SETTINGS.captures(&page).ok_or_else(|| {
            diagnostics::invalid(
                "Beacon",
                "page has neither beacon URL nor settings script",
                None,
            )
        })?;
        let script = self.page(self.trusted_url(&settings[1])?).await?;
        let beacon = BEACON.captures(&script).ok_or_else(|| {
            diagnostics::invalid("Beacon", "settings script has no beacon URL", None)
        })?;
        self.trusted_url(&beacon[1])
    }

    #[tracing::instrument(skip_all)]
    pub async fn send_watch(
        &self,
        channel: &mut Channel,
        now: DateTime<Utc>,
    ) -> Result<bool, TwitchError> {
        let Some(broadcast) = &channel.broadcast_id else {
            return Ok(false);
        };
        let payload = json!([{"event":"minute-watched","properties":{
            "broadcast_id":broadcast,"channel_id":channel.identity.id.to_string(),"channel":channel.identity.login,
            "client_time":now.to_rfc3339_opts(chrono::SecondsFormat::Micros,true),
            "game":channel.game.as_ref().map_or("",|g|g.name.as_str()),"game_id":channel.game.as_ref().map(|g|g.id.to_string()).unwrap_or_default(),
            "hidden":false,"is_live":true,"live":true,"location":"channel","logged_in":true,"minutes_logged":1,
            "muted":false,"player":"site","user_id":self.user_id,
        }}]);
        let url = match channel.beacon_url.take() {
            Some(url) => url,
            None => self.beacon(channel).await?,
        };
        let response = self
            .http
            .execute(
                self.http
                    .request(Method::POST, url.clone())
                    .form(&[("data", STANDARD.encode(payload.to_string()))]),
                RetryPolicy::Transport,
            )
            .await?;
        let acknowledged = response.status() == StatusCode::NO_CONTENT;
        if acknowledged {
            channel.beacon_url = Some(url);
        } else {
            tracing::warn!(
                status = response.status().as_u16(),
                expected_status = 204,
                "Watch beacon was not acknowledged"
            );
        }
        Ok(acknowledged)
    }

    pub async fn current_drop(
        &self,
        channel_id: u64,
    ) -> Result<Option<(String, u32)>, TwitchError> {
        let response = self
            .gql(Operation::CurrentDrop.request(json!({"channelID":channel_id.to_string()})))
            .await?;
        let Some(drop) = response
            .pointer("/data/currentUser/dropCurrentSession")
            .filter(|v| !v.is_null())
        else {
            return Ok(None);
        };
        // Twitch also reports an empty session object when no current drop is available.
        if drop.get("channel") == Some(&Value::Null)
            && drop.get("dropID").and_then(Value::as_str) == Some("")
            && drop.get("currentMinutesWatched").and_then(Value::as_u64) == Some(0)
            && drop.get("game") == Some(&Value::Null)
            && drop.get("requiredMinutesWatched").and_then(Value::as_u64) == Some(0)
        {
            return Ok(None);
        }
        let reported_channel = number(&drop["channel"]["id"]).ok_or_else(|| {
            diagnostics::invalid(
                "CurrentDrop",
                "dropCurrentSession.channel.id must be a channel ID",
                drop.pointer("/channel/id"),
            )
        })?;
        if reported_channel != channel_id {
            return Ok(None);
        }
        let id = drop["dropID"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                diagnostics::invalid(
                    "CurrentDrop",
                    "dropCurrentSession.dropID must be a nonempty string",
                    drop.get("dropID"),
                )
            })?;
        let minutes = number(&drop["currentMinutesWatched"])
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                diagnostics::invalid(
                    "CurrentDrop",
                    "dropCurrentSession.currentMinutesWatched must fit u32",
                    drop.get("currentMinutesWatched"),
                )
            })?;
        Ok(Some((id.to_owned(), minutes)))
    }

    pub async fn claim(&self, claim_id: &str) -> Result<bool, TwitchError> {
        let response = self
            .gql(Operation::ClaimDrop.request(json!({"input":{"dropInstanceID":claim_id}})))
            .await?;
        Ok(matches!(
            response["data"]["claimDropRewards"]["status"].as_str(),
            Some("ELIGIBLE_FOR_ALL" | "DROP_INSTANCE_ALREADY_CLAIMED")
        ))
    }

    pub async fn delete_notification(&self, id: &str) -> Result<(), TwitchError> {
        self.gql(Operation::DeleteNotification.request(json!({"input":{"id":id}})))
            .await?;
        Ok(())
    }
}

fn update_stream(channel: &mut Channel, user: &Value) {
    let stream = &user["stream"];
    let broadcast_id = stream["id"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    if channel.broadcast_id != broadcast_id {
        channel.beacon_url = None;
    }
    channel.broadcast_id = broadcast_id;
    channel.game = channel
        .online()
        .then(|| Game::parse(&user["broadcastSettings"]["game"]).ok())
        .flatten();
    channel.viewers = channel
        .online()
        .then(|| number(&stream["viewersCount"]))
        .flatten();
    if let Some(name) = user["displayName"].as_str().filter(|n| !n.is_empty()) {
        channel.identity.name = name.to_owned();
    }
}

fn directory_channel(raw: &Value, game: &Game) -> Option<Channel> {
    Some(Channel {
        identity: ChannelIdentity::parse(&raw["broadcaster"]).ok()?,
        game: Some(Game::parse(&raw["game"]).unwrap_or_else(|_| game.clone())),
        broadcast_id: Some(raw["id"].as_str().filter(|s| !s.is_empty())?.to_owned()),
        viewers: number(&raw["viewersCount"]),
        drops_enabled: true,
        acl_based: false,
        beacon_url: None,
    })
}
fn channel_priority(
    channel: &Channel,
    campaigns: &[(&Campaign, MiningPriority)],
) -> MiningPriority {
    campaigns
        .iter()
        .filter(|(c, _)| c.matches_channel(channel))
        .map(|(_, priority)| *priority)
        .min()
        .unwrap_or(MiningPriority::Unavailable)
}

pub fn select_channel(
    channels: &[Channel],
    campaigns: &[Campaign],
    settings: &Settings,
    now: DateTime<Utc>,
    current: Option<u64>,
    manual: Option<u64>,
) -> Option<u64> {
    if let Some(id) = manual {
        return channels
            .iter()
            .find(|c| c.identity.id == id && c.online())
            .map(|c| c.identity.id);
    }
    let campaigns: Vec<_> = campaigns
        .iter()
        .filter(|c| c.can_mine(settings, now))
        .map(|c| (c, c.mining_priority(settings, now)))
        .collect();
    let eligible = |channel: &&Channel| campaigns.iter().any(|(c, _)| c.matches_channel(channel));
    let best = channels.iter().filter(eligible).min_by_key(|c| {
        (
            channel_priority(c, &campaigns),
            Reverse(c.acl_based),
            Reverse(c.viewers),
            c.identity.id,
        )
    })?;
    if let Some(current) = channels
        .iter()
        .filter(eligible)
        .find(|c| Some(c.identity.id) == current)
        && (
            channel_priority(current, &campaigns),
            Reverse(current.acl_based),
        ) <= (channel_priority(best, &campaigns), Reverse(best.acl_based))
    {
        return Some(current.identity.id);
    }
    Some(best.identity.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::tests::{campaign_json, gql_mock, http, session};
    use std::{collections::HashMap, sync::Arc};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[test]
    fn manual_input_is_a_twitch_login_not_a_network_target() {
        for input in [
            "  Extra_Streamer  ",
            "@Extra_Streamer",
            "https://www.twitch.tv/Extra_Streamer/",
        ] {
            assert_eq!(channel_login(input).as_deref(), Some("extra_streamer"));
        }
        for input in [
            "",
            "https://evil.test/a",
            "https://twitch.tv.evil.test/a",
            "https://user:secret@twitch.tv/a",
            "https://twitch.tv:8443/a",
            "https://twitch.tv/a/videos",
            "https://twitch.tv/a?token=secret",
            "https://twitch.tv/a#secret",
            "http://127.0.0.1/a",
            "a b",
            "a\\b",
            "a".repeat(26).as_str(),
        ] {
            assert!(
                channel_login(input).is_none(),
                "accepted invalid channel input"
            );
        }
    }

    #[test]
    fn cross_category_acl_streams_use_the_selected_campaign_priority() {
        let now = Utc::now();
        let rust = Campaign::parse(&campaign_json("rust"), &HashMap::new(), now).unwrap();
        let mut event = rust.clone();
        event.id = "event".into();
        event.game.id = 509663;
        event.game.name = "Special Events".into();
        let regular = channel(10);
        let mut host = channel(11);
        host.game.as_mut().unwrap().id = 509658;
        host.game.as_mut().unwrap().name = "Just Chatting".into();
        event.allowed_channels = vec![host.identity.clone()];
        let settings = Settings {
            games_to_watch: vec!["Special Events".into(), "Rust".into()],
            ..Settings::default()
        };
        assert_eq!(
            select_channel(
                &[regular, host],
                &[rust, event],
                &settings,
                now,
                Some(10),
                None
            ),
            Some(11)
        );
    }

    #[tokio::test]
    async fn automatic_types_discover_each_unselected_game_directory() {
        let server = MockServer::start().await;
        gql_mock(&server, |q| {
            assert!(q["variables"]["slug"] == "rust" || q["variables"]["slug"] == "other");
            json!({"data":{"game":{"streams":{"edges":[]}}}})
        })
        .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let now = Utc::now();
        let mut first = Campaign::parse(&campaign_json("one"), &HashMap::new(), now).unwrap();
        first.allowed_channels.clear();
        first.drops[0].benefits[0].kind = "EMOTE".into();
        let mut second = first.clone();
        second.id = "two".into();
        second.game.id = 2;
        second.game.name = "Other".into();
        second.game.slug = "other".into();
        let settings = Settings {
            auto_mine_emotes: true,
            ..Settings::default()
        };
        client
            .channels(&[first, second], &settings, None)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let batch: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(batch.as_array().unwrap().len(), 2);
        assert_ne!(batch[0]["variables"]["slug"], batch[1]["variables"]["slug"]);
    }

    #[test]
    fn selected_games_precede_automatic_types_then_expiry_with_manual_override() {
        let now = Utc::now();
        let mut first = Campaign::parse(&campaign_json("one"), &HashMap::new(), now).unwrap();
        first.allowed_channels.clear();
        first.drops[0].benefits[0].kind = "BADGE".into();
        let mut second = first.clone();
        second.game.id = 2;
        second.game.name = "Other".into();
        second.ends_at = first.ends_at - Duration::minutes(1);
        let a = channel(10);
        let mut b = channel(11);
        b.game = Some(second.game.clone());
        let channels = [a, b];
        let campaigns = [first, second];
        let mut settings = Settings {
            auto_mine_badges: true,
            ..Settings::default()
        };
        assert_eq!(
            select_channel(&channels, &campaigns, &settings, now, Some(10), None),
            Some(11)
        );
        settings.games_to_watch = vec!["Rust".into()];
        assert_eq!(
            select_channel(&channels, &campaigns, &settings, now, Some(11), None),
            Some(10)
        );
        assert_eq!(
            select_channel(&channels, &campaigns, &settings, now, None, Some(11)),
            Some(11)
        );
        settings.games_to_watch.clear();
        settings.auto_mine_badges = false;
        assert_eq!(
            select_channel(&channels, &campaigns, &settings, now, Some(10), None),
            None
        );
    }

    #[tokio::test]
    async fn eligible_streams_survive_the_limit_ahead_of_unrelated_acl_channels() {
        let server = MockServer::start().await;
        gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
            "VideoPlayerStreamInfoOverlayChannel" => {
                let live = q["variables"]["channel"] == "wanted";
                json!({"data":{"user":{"stream":{"id":"stream","viewersCount":if live {1} else {10000}},"broadcastSettings":{"game":{"id":if live {"1"} else {"2"},"name":if live {"Rust"} else {"Other"}}}}}})
            },
            "DropsHighlightService_AvailableDrops" => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"one"}]}}}),
            other => panic!("unexpected operation {other}"),
        }).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let mut campaign =
            Campaign::parse(&campaign_json("one"), &HashMap::new(), Utc::now()).unwrap();
        let mut identities: Vec<_> = (1..=MAX_CHANNELS as u64)
            .map(|id| ChannelIdentity {
                id,
                login: format!("unrelated{id}"),
                name: format!("Unrelated {id}"),
            })
            .collect();
        identities.push(ChannelIdentity {
            id: 999,
            login: "wanted".into(),
            name: "Wanted".into(),
        });
        campaign.allowed_channels = identities;
        let settings = Settings {
            games_to_watch: vec!["Rust".into()],
            ..Settings::default()
        };
        let channels = client.channels(&[campaign], &settings, None).await.unwrap();
        assert_eq!(channels.len(), MAX_CHANNELS);
        assert_eq!(channels[0].identity.id, 999);
    }

    #[tokio::test]
    async fn manual_lookup_resolves_identity_and_available_campaigns_without_scanning_directories()
    {
        let server = MockServer::start().await;
        gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
            "VideoPlayerStreamInfoOverlayChannel" => {
                assert_eq!(q["variables"]["channel"], "extra_streamer");
                json!({"data":{"user":{"id":"999","displayName":"Extra Streamer","stream":{"id":"live","viewersCount":null},"broadcastSettings":{"game":{"id":"1","name":"Rust","slug":"rust"}}}}})
            },
            "DropsHighlightService_AvailableDrops" => {
                assert_eq!(q["variables"]["channelID"], "999");
                json!({"data":{"channel":{"viewerDropCampaigns":[null,{"id":"one"}]}}})
            },
            other => panic!("unexpected operation {other}"),
        }).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let resolved = client
            .resolve_channel("extra_streamer")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.identity.id, 999);
        assert_eq!(resolved.viewers, None);
        assert_eq!(resolved.game.unwrap().name, "Rust");
        assert!(resolved.drops_enabled);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        server.reset().await;
        gql_mock(&server, |_| json!({"data":{"user":null}})).await;
        assert!(client.resolve_channel("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn manual_lookup_and_refresh_tolerate_failed_or_slow_metadata_but_propagate_auth() {
        for error in ["catalog unavailable", "service unavailable", "Unauthorized"] {
            let server = MockServer::start().await;
            gql_mock(&server, move |q| match q["operationName"].as_str().unwrap() {
                "VideoPlayerStreamInfoOverlayChannel" => json!({"data":{"user":{"id":"10","displayName":"Streamer","stream":{"id":"live"}}}}),
                "DropsHighlightService_AvailableDrops" => json!({"errors":[{"message":error}]}),
                other => panic!("unexpected operation {other}"),
            }).await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let lookup = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                client.resolve_channel("streamer"),
            )
            .await
            .unwrap();
            let mut channel = Channel::offline(channel(10).identity, false);
            let refresh = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                client.update_manual_channel(&mut channel),
            )
            .await
            .unwrap();
            if error == "Unauthorized" {
                assert!(matches!(lookup, Err(TwitchError::Unauthorized)));
                assert_eq!(refresh, Err(TwitchError::Unauthorized));
                assert!(!channel.online());
            } else {
                let resolved = lookup.unwrap().unwrap();
                assert!(resolved.online());
                assert!(!resolved.drops_enabled);
                refresh.unwrap();
                assert!(channel.online());
                assert!(!channel.drops_enabled);
                if error == "catalog unavailable" {
                    assert!(
                        client.update_channels(&mut [channel]).await.is_err(),
                        "automatic discovery must still require reward evidence"
                    );
                }
            }
        }
    }

    fn channel(id: u64) -> Channel {
        Channel {
            identity: ChannelIdentity {
                id,
                login: "streamer".into(),
                name: "Streamer".into(),
            },
            game: Some(Game {
                id: 1,
                name: "Rust".into(),
                slug: "rust".into(),
                image_url: String::new(),
            }),
            broadcast_id: Some("123".into()),
            viewers: Some(50),
            drops_enabled: true,
            acl_based: false,
            beacon_url: None,
        }
    }

    #[test]
    fn mining_priority_switches_to_the_twentieth_game_and_returns_to_manual_order() {
        let now = Utc::now();
        let mut first = Campaign::parse(&campaign_json("first"), &HashMap::new(), now).unwrap();
        first.allowed_channels.clear();
        first.starts_at = now - Duration::days(3);
        first.ends_at = now + Duration::days(3);
        first.drops[0].starts_at = first.starts_at;
        first.drops[0].ends_at = first.ends_at;
        let mut event = first.clone();
        event.id = "event".into();
        event.game.id = 20;
        event.game.name = "Game 20".into();
        event.drops[0].starts_at = now;
        event.drops[0].ends_at = now + Duration::hours(3);
        let mut streams = vec![channel(10), channel(20), channel(21)];
        streams[1].game = Some(event.game.clone());
        streams[2].game = Some(event.game.clone());
        streams[2].viewers = Some(1000);
        let mut games = vec!["Rust".to_owned()];
        games.extend((2..=20).map(|i| format!("Game {i}")));
        let manual = Settings {
            games_to_watch: games,
            ..Settings::default()
        };
        let mut campaigns = vec![first, event];
        assert_eq!(
            select_channel(&streams, &campaigns, &manual, now, Some(10), None),
            Some(10)
        );
        for mode in ["short_events", "ending_soonest"] {
            let settings = manual
                .patched(&json!({"mining_priority_mode":mode}))
                .unwrap();
            assert_eq!(
                select_channel(&streams, &campaigns, &settings, now, Some(10), None),
                Some(21)
            );
            assert_eq!(
                select_channel(&streams, &campaigns, &settings, now, Some(20), None),
                Some(20)
            );
            assert_eq!(
                select_channel(&streams, &campaigns, &settings, now, Some(10), Some(10)),
                Some(10)
            );
            assert_eq!(
                select_channel(
                    &streams,
                    &campaigns,
                    &settings,
                    now - Duration::seconds(1),
                    Some(10),
                    None
                ),
                Some(10)
            );
            assert_eq!(
                select_channel(
                    &streams,
                    &campaigns,
                    &settings,
                    now + Duration::hours(3),
                    Some(20),
                    None
                ),
                Some(10)
            );
            let queue = crate::domain::wanted_items(&campaigns, &settings, now);
            assert_eq!(queue[0].game_name, "Game 20");
            let mut unavailable = streams.clone();
            unavailable[1].broadcast_id = None;
            unavailable[2].drops_enabled = false;
            assert_eq!(
                select_channel(&unavailable, &campaigns, &settings, now, Some(20), None),
                Some(10)
            );
            let mut restricted = campaigns.clone();
            restricted[1].allowed_channels = vec![channel(999).identity];
            assert_eq!(
                select_channel(&streams, &restricted, &settings, now, Some(20), None),
                Some(10)
            );
            let optional = settings
                .patched(&json!({"games_to_watch":["Rust"], "auto_mine_badges":true}))
                .unwrap();
            restricted[1].allowed_channels.clear();
            restricted[1].drops[0].benefits[0].kind = "BADGE".into();
            assert_eq!(
                select_channel(&streams, &restricted, &optional, now, Some(20), None),
                Some(10)
            );
        }
        assert_eq!(
            select_channel(&streams, &campaigns, &manual, now, Some(20), None),
            Some(10)
        );
        // Ending soonest also considers a long campaign nearing its deadline;
        // Short events first intentionally preserves the short-window exception.
        campaigns[0].drops[0].ends_at = now + Duration::hours(2);
        let ending = manual
            .patched(&json!({"mining_priority_mode":"ending_soonest"}))
            .unwrap();
        let short = manual
            .patched(&json!({"mining_priority_mode":"short_events"}))
            .unwrap();
        assert_eq!(
            select_channel(&streams, &campaigns, &ending, now, Some(20), None),
            Some(10)
        );
        assert_eq!(
            select_channel(&streams, &campaigns, &short, now, Some(10), None),
            Some(21)
        );
        campaigns[0].drops[0].ends_at = campaigns[1].drops[0].ends_at;
        assert_eq!(
            select_channel(&streams, &campaigns, &ending, now, Some(20), None),
            Some(10)
        );
        let required = campaigns[1].drops[0].required_minutes;
        campaigns[1].drops[0].confirm(required, now);
        let settings = manual
            .patched(&json!({"mining_priority_mode":"short_events"}))
            .unwrap();
        assert_eq!(
            select_channel(&streams, &campaigns, &settings, now, Some(20), None),
            Some(10)
        );
        assert!(!campaigns[1].drops[0].claimed);
        assert_eq!(settings.games_to_watch, manual.games_to_watch);
    }

    #[tokio::test]
    async fn mining_priority_ranks_event_streams_before_the_channel_limit() {
        let server = MockServer::start().await;
        gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
            "VideoPlayerStreamInfoOverlayChannel" => {
                let event = q["variables"]["channel"] == "event";
                json!({"data":{"user":{"stream":{"id":"live","viewersCount":if event {1} else {10000}},"broadcastSettings":{"game":{"id":if event {"2"} else {"1"},"name":if event {"Event game"} else {"Rust"}}}}}})
            },
            "DropsHighlightService_AvailableDrops" => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"ordinary"},{"id":"event"}]}}}),
            other => panic!("unexpected operation {other}"),
        }).await;
        let now = Utc::now();
        let mut ordinary =
            Campaign::parse(&campaign_json("ordinary"), &HashMap::new(), now).unwrap();
        ordinary.starts_at = now - Duration::days(3);
        ordinary.ends_at = now + Duration::days(3);
        ordinary.drops[0].starts_at = ordinary.starts_at;
        ordinary.drops[0].ends_at = ordinary.ends_at;
        ordinary.allowed_channels = (1..=MAX_CHANNELS as u64)
            .map(|id| ChannelIdentity {
                id,
                login: format!("ordinary{id}"),
                name: format!("Ordinary {id}"),
            })
            .collect();
        let mut event = ordinary.clone();
        event.id = "event".into();
        event.game.id = 2;
        event.game.name = "Event game".into();
        event.drops[0].starts_at = now;
        event.drops[0].ends_at = now + Duration::hours(3);
        event.allowed_channels = vec![ChannelIdentity {
            id: 999,
            login: "event".into(),
            name: "Event".into(),
        }];
        let campaigns = [ordinary, event];
        let settings = Settings::default().patched(&json!({"games_to_watch":["Rust", "Event game"], "mining_priority_mode":"short_events"})).unwrap();
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let channels = client
            .channels(&campaigns, &settings, Some(&channel(10)))
            .await
            .unwrap();
        assert_eq!(channels.len(), MAX_CHANNELS);
        assert_eq!(channels[0].identity.id, 10);
        assert_eq!(channels[1].identity.id, 999);
        assert_eq!(
            select_channel(&channels, &campaigns, &settings, now, Some(10), None),
            Some(999)
        );
    }

    #[tokio::test]
    async fn public_beacon_page_and_script_rejections_preserve_authentication() {
        for status in [401, 403] {
            for script in [false, true] {
                let server = MockServer::start().await;
                let script_path = "/config/settings.0123456789abcdef0123456789abcdef.js";
                Mock::given(method("GET"))
                    .and(path("/streamer"))
                    .respond_with(if script {
                        ResponseTemplate::new(200).set_body_string(format!(
                            r#"<script src="{}{script_path}"></script>"#,
                            server.uri()
                        ))
                    } else {
                        ResponseTemplate::new(status)
                    })
                    .expect(1)
                    .mount(&server)
                    .await;
                Mock::given(method("GET"))
                    .and(path(script_path))
                    .respond_with(ResponseTemplate::new(status))
                    .expect(u64::from(script))
                    .mount(&server)
                    .await;
                let client = TwitchClient::new(Arc::new(http(&server)), &session());
                let mut channel = channel(10);
                assert_eq!(
                    client.send_watch(&mut channel, Utc::now()).await,
                    Err(TwitchError::Status(status))
                );
                assert!(channel.beacon_url.is_none());
                for request in server.received_requests().await.unwrap() {
                    assert!(!request.headers.contains_key("Authorization"));
                }
            }
        }
    }

    #[tokio::test]
    async fn watch_retries_a_closed_connection_promptly_with_the_same_payload() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url: Url = format!("http://{}/track", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let peer = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() < 16384);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        assert!(headers.starts_with("POST /track HTTP/1.1"));
                        let length: usize = headers
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .unwrap()
                            .1
                            .trim()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            bodies.push(bytes[end + 4..end + 4 + length].to_vec());
                            break;
                        }
                    }
                }
                // First request is fully received, then disconnected before response headers.
                if attempt == 1 {
                    socket
                        .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                        .await
                        .unwrap();
                }
            }
            bodies
        });
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let mut channel = channel(10);
        channel.beacon_url = Some(url.clone());
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.send_watch(&mut channel, Utc::now()),
        )
        .await;
        if !matches!(result, Ok(Ok(true))) {
            peer.abort();
            let _ = peer.await;
            panic!("watch must recover within five seconds: {result:?}");
        }
        let bodies = peer.await.unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_secs(1));
        assert_eq!(bodies.len(), 2);
        assert_eq!(
            bodies[0], bodies[1],
            "retry must not invent another watch minute"
        );
        assert_eq!(channel.beacon_url, Some(url));
    }

    #[tokio::test]
    async fn watch_retries_transient_http_responses_but_cancels_backoff() {
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            time::Duration,
        };

        for status in [429, 503] {
            let server = MockServer::start().await;
            let attempts = AtomicUsize::new(0);
            Mock::given(method("POST"))
                .and(path("/track"))
                .respond_with(move |_: &wiremock::Request| {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        ResponseTemplate::new(status).insert_header("Retry-After", "1")
                    } else {
                        ResponseTemplate::new(204)
                    }
                })
                .expect(2)
                .mount(&server)
                .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let mut channel = channel(10);
            channel.beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
            let start = tokio::time::Instant::now();
            assert!(
                tokio::time::timeout(
                    Duration::from_secs(5),
                    client.send_watch(&mut channel, Utc::now())
                )
                .await
                .unwrap()
                .unwrap()
            );
            assert!(start.elapsed() >= Duration::from_secs(1));
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].body, requests[1].body);
        }

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "60"))
            .expect(1)
            .mount(&server)
            .await;
        let http = Arc::new(http(&server));
        let client = TwitchClient::new(http.clone(), &session());
        let mut channel = channel(10);
        channel.beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
        let work = tokio::spawn(async move {
            let result = client.send_watch(&mut channel, Utc::now()).await;
            (result, channel.beacon_url)
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        http.cancel.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), work)
                .await
                .unwrap()
                .unwrap(),
            (Err(TwitchError::Cancelled), None)
        );
    }

    #[tokio::test]
    async fn watch_transport_errors_and_unacknowledged_responses_discard_the_cached_beacon() {
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unreachable = format!("http://{}/track", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        drop(listener);
        let mut channel = channel(10);
        channel.beacon_url = Some(unreachable);
        assert_eq!(
            client.send_watch(&mut channel, Utc::now()).await,
            Err(TwitchError::Network)
        );
        assert!(channel.beacon_url.is_none());
        for status in [200, 302, 401, 403, 500] {
            server.reset().await;
            Mock::given(method("POST"))
                .and(path("/track"))
                .respond_with(ResponseTemplate::new(status).insert_header("Retry-After", "1"))
                .expect(if status == 500 { 5 } else { 1 })
                .mount(&server)
                .await;
            channel.beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
            assert!(
                !tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    client.send_watch(&mut channel, Utc::now()),
                )
                .await
                .expect("retry exhaustion must return to the session owner")
                .unwrap()
            );
            assert!(channel.beacon_url.is_none());
        }
    }

    #[tokio::test]
    async fn both_beacon_formats_send_exact_watch_events_without_playlists_or_credentials() {
        for script in [false, true] {
            let server = MockServer::start().await;
            let url = format!("{}/track", server.uri());
            let body = format!(r#"{{"beacon_url":"{url}"}}"#);
            let script_path = "/config/settings.0123456789abcdef0123456789abcdef.js";
            let html = if script {
                format!(r#"<script src="{}{script_path}"></script>"#, server.uri())
            } else {
                body.clone()
            };
            Mock::given(method("GET"))
                .and(path("/streamer"))
                .respond_with(ResponseTemplate::new(200).set_body_string(html))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(script_path))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/track"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let mut channel = channel(10);
            let now = Utc::now();
            assert!(client.send_watch(&mut channel, now).await.unwrap());
            assert!(client.send_watch(&mut channel, now).await.unwrap());
            let requests = server.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.url.path() == "/streamer")
                    .count(),
                1
            );
            assert_eq!(requests.len(), if script { 4 } else { 3 });
            let watch = requests.iter().find(|r| r.method == "POST").unwrap();
            assert!(!watch.headers.contains_key("Authorization"));
            let form: HashMap<_, _> = url::form_urlencoded::parse(&watch.body).collect();
            let payload: Value =
                serde_json::from_slice(&STANDARD.decode(form["data"].as_bytes()).unwrap()).unwrap();
            assert_eq!(payload[0]["event"], "minute-watched");
            let fields = &payload[0]["properties"];
            assert_eq!(fields["channel_id"], "10");
            assert_eq!(fields["broadcast_id"], "123");
            assert_eq!(fields["game_id"], "1");
            assert_eq!(fields["user_id"], 42);
            assert_eq!(fields["minutes_logged"], 1);
            assert_eq!(
                fields["client_time"]
                    .as_str()
                    .unwrap()
                    .parse::<DateTime<Utc>>()
                    .unwrap()
                    .timestamp(),
                now.timestamp()
            );
            assert_eq!(fields["hidden"], false);
            assert_eq!(fields["is_live"], true);
            assert_eq!(fields["player"], "site");
            for hostile in [
                "https://twitch.tv.evil.example/track",
                "https://evil.example/track",
                "http://spade.twitch.tv/track",
                "https://user:pass@spade.twitch.tv/track",
                "https://spade.twitch.tv:8443/track",
            ] {
                assert!(client.trusted_url(hostile).is_err());
            }
            assert!(client.trusted_url("https://spade.twitch.tv/track").is_ok());
            assert!(
                client
                    .trusted_url("https://assets.twitch.tv/config/file.js")
                    .is_ok()
            );
        }
    }

    #[tokio::test]
    async fn stream_updates_preserve_nullable_viewers_and_clear_stale_stream_artifacts() {
        let server = MockServer::start().await;
        gql_mock(&server,|query|match query["operationName"].as_str().unwrap() {
            "VideoPlayerStreamInfoOverlayChannel"=>json!({"data":{"user":{"id":"10","displayName":"New name","stream":{"id":"456","viewersCount":null},"broadcastSettings":{"game":{"id":"1","name":"Rust"}}}}}),
            "DropsHighlightService_AvailableDrops"=>json!({"data":{"channel":{"viewerDropCampaigns":[null,{"id":"campaign"}]}}}),
            _=>panic!("unexpected query"),
        }).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let mut channels = vec![channel(10)];
        channels[0].beacon_url = Some(Url::parse("https://spade.twitch.tv/old").unwrap());
        client.update_channels(&mut channels).await.unwrap();
        assert_eq!(channels[0].broadcast_id.as_deref(), Some("456"));
        assert!(channels[0].beacon_url.is_none());
        assert!(channels[0].viewers.is_none());
        assert!(channels[0].drops_enabled);
        server.reset().await;
        gql_mock(&server, |_| json!({"data":{"user":null,"channel":null}})).await;
        client.update_channels(&mut channels).await.unwrap();
        assert!(!channels[0].online());
        assert!(channels[0].game.is_none());
        assert!(!channels[0].drops_enabled);
    }

    #[test]
    fn selection_preserves_healthy_streams_and_special_category_failover_without_bypassing_opt_in()
    {
        let now = Utc::now();
        let mut campaign =
            Campaign::parse(&campaign_json("campaign"), &HashMap::new(), now).unwrap();
        let mut settings = Settings::default();
        let mut channels = vec![channel(10), channel(11)];
        channels[1].viewers = Some(100);
        assert_eq!(
            select_channel(&channels, &[campaign.clone()], &settings, now, None, None),
            None
        );
        settings.games_to_watch = vec!["Rust".into()];
        assert_eq!(
            select_channel(&channels, &[campaign.clone()], &settings, now, None, None),
            Some(11)
        );
        assert_eq!(
            select_channel(
                &channels,
                &[campaign.clone()],
                &settings,
                now,
                Some(10),
                None
            ),
            Some(10)
        );
        channels[1].acl_based = true;
        assert_eq!(
            select_channel(
                &channels,
                &[campaign.clone()],
                &settings,
                now,
                Some(10),
                None
            ),
            Some(11)
        );
        assert_eq!(
            select_channel(
                &channels,
                &[campaign.clone()],
                &settings,
                now,
                None,
                Some(10)
            ),
            Some(10)
        );
        campaign.game.id = 509663;
        campaign.game.name = "Special Events".into();
        campaign.allowed_channels = channels.iter().map(|c| c.identity.clone()).collect();
        settings.games_to_watch = vec!["Special Events".into()];
        channels[0].broadcast_id = None;
        channels[1].drops_enabled = false;
        assert_eq!(
            select_channel(
                &channels,
                &[campaign.clone()],
                &settings,
                now,
                Some(10),
                None
            ),
            Some(11)
        );
        campaign.allowed_channels.clear();
        assert_eq!(
            select_channel(&channels, &[campaign], &settings, now, None, None),
            None
        );
    }

    #[tokio::test]
    async fn claims_use_account_ids_and_only_accept_twitch_success_states() {
        let server = MockServer::start().await;
        gql_mock(&server, |query| {
            assert_eq!(
                query["variables"]["input"]["dropInstanceID"],
                "earned-instance"
            );
            json!({"data":{"claimDropRewards":{"status":"DROP_INSTANCE_ALREADY_CLAIMED"}}})
        })
        .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        assert!(client.claim("earned-instance").await.unwrap());
        server.reset().await;
        gql_mock(
            &server,
            |_| json!({"data":{"claimDropRewards":{"status":"NOT_ELIGIBLE"}}}),
        )
        .await;
        assert!(!client.claim("earned-instance").await.unwrap());
    }
}
