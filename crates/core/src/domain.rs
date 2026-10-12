use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::{
    config::{MiningPriorityMode, Settings},
    dto::*,
    policy::DropPolicy,
};

pub const MAX_ESTIMATED_MINUTES: u32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MiningPriority {
    Deadline(DateTime<Utc>, usize),
    Selected(usize),
    Automatic(DateTime<Utc>),
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid Twitch campaign data")]
pub struct InvalidData;

pub fn text(value: &Value, key: &str) -> Result<String, InvalidData> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(InvalidData)
}

pub fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn nullable_array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], InvalidData> {
    match value.get(key) {
        Some(Value::Null) => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        _ => Err(InvalidData),
    }
}

fn timestamp(value: &Value, key: &str) -> Result<DateTime<Utc>, InvalidData> {
    let at: DateTime<Utc> = value[key]
        .as_str()
        .ok_or(InvalidData)?
        .parse()
        .map_err(|_| InvalidData)?;
    // Every campaign/drop date must support scheduling lead time and claim grace.
    at.checked_sub_signed(Duration::hours(1))
        .ok_or(InvalidData)?;
    at.checked_add_signed(Duration::hours(24))
        .ok_or(InvalidData)?;
    Ok(at)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub slug: String,
    pub image_url: String,
}

impl Game {
    pub fn parse(value: &Value) -> Result<Self, InvalidData> {
        let name = text(value, "displayName").or_else(|_| text(value, "name"))?;
        Ok(Self {
            id: number(&value["id"]).ok_or(InvalidData)?,
            slug: text(value, "slug").unwrap_or_else(|_| Self::slug(&name)),
            name,
            image_url: value["boxArtURL"].as_str().unwrap_or_default().to_owned(),
        })
    }

    pub fn slug(name: &str) -> String {
        let mut result = String::new();
        for c in name.to_lowercase().chars().filter(|c| *c != '\'') {
            if c.is_alphanumeric() || c == '_' {
                result.push(c);
            } else if !result.is_empty() && !result.ends_with('-') {
                result.push('-');
            }
        }
        result.trim_end_matches('-').to_owned()
    }

    pub fn special(&self) -> bool {
        matches!(self.id, 509663 | 509672)
    }
}

#[derive(Clone, Debug)]
pub struct Benefit {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub image_url: String,
}

impl Benefit {
    fn parse(value: &Value) -> Result<Self, InvalidData> {
        let kind = value["distributionType"]
            .as_str()
            .filter(|kind| matches!(*kind, "BADGE" | "EMOTE" | "DIRECT_ENTITLEMENT"))
            .unwrap_or("UNKNOWN");
        Ok(Self {
            id: text(value, "id")?,
            name: text(value, "name")?,
            kind: kind.to_owned(),
            image_url: value["imageAssetURL"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        })
    }

    pub fn wanted(&self, settings: &Settings) -> bool {
        settings
            .mining_benefits
            .get(&self.kind)
            .copied()
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
pub struct Drop {
    pub id: String,
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub required_minutes: u32,
    pub confirmed_minutes: u32,
    pub estimated_minutes: u32,
    pub confirmed_at: Option<DateTime<Utc>>,
    // Runtime evidence only; never included in saved history or completion records.
    pub reported_minutes: Option<u32>,
    pub progress_disputed: bool,
    pub claimed: bool,
    pub claimed_at: Option<DateTime<Utc>>,
    pub claim_id: Option<String>,
    pub prerequisites: Vec<String>,
    pub benefits: Vec<Benefit>,
}

impl Drop {
    fn parse(
        value: &Value,
        awards: &HashMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Self, InvalidData> {
        let starts_at = timestamp(value, "startAt")?;
        let ends_at = timestamp(value, "endAt")?;
        let benefits = value["benefitEdges"]
            .as_array()
            .ok_or(InvalidData)?
            .iter()
            .map(|edge| Benefit::parse(&edge["benefit"]))
            .collect::<Result<Vec<_>, _>>()?;
        let account = value.get("self").filter(|v| !v.is_null());
        if let Some(account) = account {
            let claimed = account["isClaimed"].as_bool().ok_or(InvalidData)?;
            if !claimed && account["currentMinutesWatched"].as_u64().is_none() {
                return Err(InvalidData);
            }
        }
        let awarded_at = benefits
            .iter()
            .try_fold(starts_at, |latest, benefit| {
                awards
                    .get(&benefit.id)
                    .filter(|at| starts_at <= **at && **at < ends_at)
                    .map(|at| latest.max(*at))
            })
            .filter(|_| !benefits.is_empty());
        let claimed = account
            .map(|v| v["isClaimed"].as_bool().unwrap_or(false))
            .unwrap_or(awarded_at.is_some());
        let required_minutes = value["requiredMinutesWatched"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(InvalidData)?;
        let confirmed_minutes = if claimed {
            required_minutes
        } else {
            account
                .and_then(|v| v["currentMinutesWatched"].as_u64())
                .unwrap_or(0)
                .min(u64::from(required_minutes)) as u32
        };
        Ok(Self {
            id: text(value, "id")?,
            name: text(value, "name")?,
            starts_at,
            ends_at,
            required_minutes,
            confirmed_minutes,
            estimated_minutes: 0,
            confirmed_at: account.map(|_| now),
            reported_minutes: None,
            progress_disputed: false,
            claimed,
            claimed_at: awarded_at.filter(|_| claimed),
            claim_id: account
                .and_then(|v| v["dropInstanceID"].as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            prerequisites: nullable_array(value, "preconditionDrops")?
                .iter()
                .map(|v| text(v, "id"))
                .collect::<Result<_, _>>()?,
            benefits,
        })
    }

    pub fn watch_reward(&self) -> bool {
        self.required_minutes > 0
    }
    pub fn current_minutes(&self) -> u32 {
        self.confirmed_minutes
            .saturating_add(self.estimated_minutes)
    }
    pub fn remaining_minutes(&self) -> u32 {
        self.required_minutes.saturating_sub(self.current_minutes())
    }
    pub fn progress(&self) -> f64 {
        if self.required_minutes == 0 {
            0.0
        } else {
            (f64::from(self.current_minutes()) / f64::from(self.required_minutes)).min(1.0)
        }
    }

    pub fn confirm(&mut self, minutes: u32, now: DateTime<Utc>) {
        self.confirmed_minutes = if self.claimed {
            self.required_minutes
        } else {
            minutes.min(self.required_minutes)
        };
        self.estimated_minutes = 0;
        self.confirmed_at = Some(now);
        self.reported_minutes = None;
        self.progress_disputed = false;
    }

    pub fn reconcile_inventory(&mut self, previous: &Self, requested_at: DateTime<Utc>) {
        if self.id != previous.id {
            return;
        }
        if !self.claimed && self.claim_id.is_none() {
            self.claim_id = previous.claim_id.clone();
        }
        if self.claimed || self.required_minutes != previous.required_minutes {
            return;
        }
        if previous
            .confirmed_at
            .is_some_and(|at| at > requested_at || self.confirmed_at.is_none())
        {
            // An older request or public metadata cannot replace account evidence.
            self.confirmed_minutes = previous.confirmed_minutes;
            self.confirmed_at = previous.confirmed_at;
            self.estimated_minutes = previous.estimated_minutes;
            self.claimed = previous.claimed;
            self.claimed_at = previous.claimed_at;
            if previous.claim_id.is_some() {
                self.claim_id = previous.claim_id.clone();
            }
            self.reported_minutes = previous.reported_minutes;
            self.progress_disputed = previous.progress_disputed;
        } else if !previous.claimed
            && previous.claim_id.is_some()
            && previous.confirmed_minutes >= previous.required_minutes
        {
            // Keep completion backed by an issued claim instance while claiming it.
            self.confirmed_minutes = previous.confirmed_minutes;
            self.confirmed_at = previous.confirmed_at;
        } else {
            // Fresh account inventory can contradict a live counter. Keep that counter
            // separately until inventory catches up, rather than retaining its maximum.
            self.reported_minutes = previous
                .reported_minutes
                .filter(|minutes| *minutes > self.confirmed_minutes);
            self.progress_disputed = self.reported_minutes.is_some();
        }
    }

    pub fn mark_claimed(&mut self, now: DateTime<Utc>) {
        self.claimed = true;
        self.claimed_at = Some(now);
        self.confirm(self.required_minutes, now);
    }

    pub fn can_claim(&self, campaign_end: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        self.claim_id.is_some() && !self.claimed && now < campaign_end + Duration::hours(24)
    }

    pub fn image_url(&self) -> String {
        self.benefits
            .iter()
            .find(|b| !b.image_url.is_empty())
            .map(|b| b.image_url.clone())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelIdentity {
    pub id: u64,
    pub login: String,
    pub name: String,
}

impl ChannelIdentity {
    pub fn parse(value: &Value) -> Result<Self, InvalidData> {
        let login = text(value, "login").or_else(|_| text(value, "name"))?;
        Ok(Self {
            id: number(&value["id"]).ok_or(InvalidData)?,
            name: text(value, "displayName").unwrap_or_else(|_| login.clone()),
            login,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub identity: ChannelIdentity,
    pub game: Option<Game>,
    pub broadcast_id: Option<String>,
    pub viewers: Option<u64>,
    pub drops_enabled: bool,
    pub acl_based: bool,
    pub beacon_url: Option<url::Url>,
}

impl Channel {
    pub fn offline(identity: ChannelIdentity, acl_based: bool) -> Self {
        Self {
            identity,
            game: None,
            broadcast_id: None,
            viewers: None,
            drops_enabled: false,
            acl_based,
            beacon_url: None,
        }
    }

    pub fn online(&self) -> bool {
        self.broadcast_id.is_some()
    }

    pub fn view(&self, watching: Option<u64>, campaigns: &[Campaign]) -> ChannelView {
        ChannelView {
            id: self.identity.id,
            login: self.identity.login.clone(),
            name: self.identity.name.clone(),
            game: self.game.as_ref().map(|g| g.name.clone()),
            game_id: self.game.as_ref().map(|g| g.id),
            game_icon: self.game.as_ref().and_then(|game| {
                std::iter::once(game)
                    .chain(campaigns.iter().map(|campaign| &campaign.game))
                    .find(|candidate| candidate.id == game.id && !candidate.image_url.is_empty())
                    .map(|game| game.image_url.clone())
            }),
            viewers: self.viewers,
            online: self.online(),
            drops_enabled: self.drops_enabled,
            acl_based: self.acl_based,
            watching: watching == Some(self.identity.id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Campaign {
    pub id: String,
    pub name: String,
    pub game: Game,
    pub linked: Option<bool>,
    pub link_url: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub valid: bool,
    pub allowed_channels: Vec<ChannelIdentity>,
    pub drops: Vec<Drop>,
}

impl Campaign {
    pub fn parse(
        value: &Value,
        awards: &HashMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Self, InvalidData> {
        let channels = nullable_array(&value["allow"], "channels")?;
        let enabled = match value["allow"].get("isEnabled") {
            None | Some(Value::Bool(true)) => true,
            Some(Value::Null | Value::Bool(false)) => false,
            _ => return Err(InvalidData),
        };
        let allowed_channels = if enabled {
            channels
                .iter()
                .map(ChannelIdentity::parse)
                .collect::<Result<_, _>>()?
        } else {
            vec![]
        };
        let raw_drops = value["timeBasedDrops"].as_array().ok_or(InvalidData)?;
        if raw_drops.len() > 5000 {
            return Err(InvalidData);
        }
        let drops = raw_drops
            .iter()
            .map(|d| Drop::parse(d, awards, now))
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen = HashSet::new();
        if drops.iter().any(|d| !seen.insert(&d.id)) {
            return Err(InvalidData);
        }
        Ok(Self {
            id: text(value, "id")?,
            name: text(value, "name")?,
            game: Game::parse(&value["game"])?,
            linked: value["self"]["isAccountConnected"].as_bool(),
            link_url: value["accountLinkURL"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            starts_at: timestamp(value, "startAt")?,
            ends_at: timestamp(value, "endAt")?,
            valid: value["status"].as_str() != Some("EXPIRED"),
            allowed_channels,
            drops,
        })
    }

    pub fn url(&self) -> String {
        let mut url =
            url::Url::parse("https://www.twitch.tv/drops/campaigns").expect("constant URL");
        url.query_pairs_mut().append_pair("dropID", &self.id);
        url.into()
    }

    pub fn active(&self, now: DateTime<Utc>) -> bool {
        self.valid && self.starts_at <= now && now < self.ends_at
    }
    pub fn upcoming(&self, now: DateTime<Utc>) -> bool {
        self.valid && now < self.starts_at
    }
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        !self.valid || self.ends_at <= now
    }
    pub fn policy(&self, settings: &Settings) -> DropPolicy {
        DropPolicy::evaluate(&self.drops, &settings.drop_name_blacklist)
    }

    pub fn mining_policy(&self, settings: &Settings, now: DateTime<Utc>) -> DropPolicy {
        DropPolicy::for_targets(&self.drops, &settings.drop_name_blacklist, |drop| {
            now < drop.ends_at && self.wanted_drop(drop, settings)
        })
    }

    fn wanted_drop(&self, drop: &Drop, settings: &Settings) -> bool {
        drop.benefits.iter().any(|benefit| {
            benefit.wanted(settings)
                && (settings.selected(&self.game.name)
                    || benefit.kind == "BADGE" && settings.auto_mine_badges
                    || benefit.kind == "EMOTE" && settings.auto_mine_emotes)
        })
    }

    fn priority_deadlines(
        &self,
        settings: &Settings,
        now: DateTime<Utc>,
        policy: &DropPolicy,
    ) -> HashMap<String, crate::policy::WatchPriority> {
        if settings.mining_priority_mode == MiningPriorityMode::Manual
            || !settings.selected(&self.game.name)
            || !self.active(now)
        {
            return HashMap::new();
        }
        policy.watch_deadlines(&self.drops, now, |drop| {
            let start = self.starts_at.max(drop.starts_at);
            let end = self.ends_at.min(drop.ends_at);
            (start <= now
                && now < end
                && self.wanted_drop(drop, settings)
                && (settings.mining_priority_mode == MiningPriorityMode::EndingSoonest
                    || end - start <= Duration::hours(24)))
            .then_some(end)
        })
    }

    pub fn mining_priority(&self, settings: &Settings, now: DateTime<Utc>) -> MiningPriority {
        let priority = settings.game_priority(Some(&self.game.name));
        if priority == usize::MAX {
            return MiningPriority::Automatic(self.ends_at);
        }
        if settings.mining_priority_mode != MiningPriorityMode::Manual {
            let policy = self.mining_policy(settings, now);
            let deadlines = self.priority_deadlines(settings, now, &policy);
            if let Some(deadline) = self
                .drops
                .iter()
                .filter(|drop| {
                    self.drop_eligible(drop, &policy, now, now + Duration::nanoseconds(1))
                })
                .filter_map(|drop| deadlines.get(&drop.id).map(|priority| priority.deadline))
                .min()
            {
                return MiningPriority::Deadline(deadline, priority);
            }
        }
        MiningPriority::Selected(priority)
    }

    pub fn prerequisites_met(&self, drop: &Drop) -> bool {
        drop.prerequisites
            .iter()
            .all(|id| self.drops.iter().any(|d| &d.id == id && d.claimed))
    }

    pub fn needs_claim_refresh(&self, now: DateTime<Utc>) -> bool {
        !self.upcoming(now)
            && now < self.ends_at + Duration::hours(24)
            && self.drops.iter().any(|d| {
                !d.claimed
                    && d.watch_reward()
                    && (d.confirmed_minutes >= d.required_minutes
                        || d.confirmed_minutes > 0 && !self.prerequisites_met(d))
            })
    }

    pub fn needs_progress_refresh(&self, now: DateTime<Utc>) -> bool {
        self.needs_claim_refresh(now)
            || !self.upcoming(now)
                && now < self.ends_at + Duration::hours(24)
                && self.drops.iter().any(|d| !d.claimed && d.progress_disputed)
    }

    pub fn is_prerequisite(&self, id: &str, drop: &Drop) -> bool {
        let by_id: HashMap<_, _> = self.drops.iter().map(|d| (d.id.as_str(), d)).collect();
        let mut pending: Vec<_> = drop.prerequisites.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        while let Some(parent) = pending.pop() {
            if parent == id {
                return true;
            }
            if seen.insert(parent)
                && let Some(parent) = by_id.get(parent)
            {
                pending.extend(parent.prerequisites.iter().map(String::as_str));
            }
        }
        false
    }

    pub fn drop_eligible(
        &self,
        drop: &Drop,
        policy: &DropPolicy,
        now: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> bool {
        policy.mineable.contains(&drop.id)
            && self.prerequisites_met(drop)
            && drop.confirmed_minutes < drop.required_minutes
            && drop.estimated_minutes < MAX_ESTIMATED_MINUTES
            && now < drop.ends_at
            && drop.starts_at < before
    }

    pub fn can_earn_within(
        &self,
        settings: &Settings,
        now: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> bool {
        let policy = self.mining_policy(settings, now);
        self.valid
            && now < self.ends_at
            && self.starts_at < before
            && self
                .drops
                .iter()
                .any(|d| self.drop_eligible(d, &policy, now, before))
    }

    pub fn channel_eligible(&self, channel: &Channel, ignore_channel_status: bool) -> bool {
        (self.allowed_channels.is_empty()
            || self
                .allowed_channels
                .iter()
                .any(|c| c.id == channel.identity.id))
            && (ignore_channel_status
                || channel.game.as_ref().is_some_and(|g| g.id == self.game.id)
                || channel.online() && self.game.special() && !self.allowed_channels.is_empty())
    }

    pub fn can_watch(&self, channel: &Channel, settings: &Settings, now: DateTime<Utc>) -> bool {
        self.can_mine(settings, now) && self.matches_channel(channel)
    }

    pub fn matches_channel(&self, channel: &Channel) -> bool {
        channel.online()
            && (self.game.special() || channel.drops_enabled)
            && self.channel_eligible(channel, false)
    }

    pub fn can_mine(&self, settings: &Settings, now: DateTime<Utc>) -> bool {
        let policy = self.mining_policy(settings, now);
        self.active(now)
            && self.valid
            && self
                .drops
                .iter()
                .any(|d| self.drop_eligible(d, &policy, now, now + Duration::nanoseconds(1)))
            && self.drops.iter().any(|d| {
                policy.mineable.contains(&d.id) && d.benefits.iter().any(|b| b.wanted(settings))
            })
    }

    pub fn first_drop(&self, settings: &Settings, now: DateTime<Utc>) -> Option<&Drop> {
        let policy = self.mining_policy(settings, now);
        let deadlines = self.priority_deadlines(settings, now, &policy);
        self.drops
            .iter()
            .filter(|d| self.drop_eligible(d, &policy, now, now + Duration::nanoseconds(1)))
            .min_by_key(|d| {
                (
                    deadlines
                        .get(&d.id)
                        .map(|priority| priority.deadline)
                        .unwrap_or(DateTime::<Utc>::MAX_UTC),
                    d.remaining_minutes(),
                )
            })
    }

    pub fn bump_estimates(&mut self, settings: &Settings, now: DateTime<Utc>) -> bool {
        let policy = self.mining_policy(settings, now);
        let eligible: HashSet<_> = self
            .drops
            .iter()
            .filter(|d| self.drop_eligible(d, &policy, now, now + Duration::nanoseconds(1)))
            .map(|d| d.id.clone())
            .collect();
        let mut stalled = false;
        for drop in &mut self.drops {
            if eligible.contains(&drop.id) && !drop.progress_disputed {
                drop.estimated_minutes += 1;
                stalled |= drop.estimated_minutes >= MAX_ESTIMATED_MINUTES;
            }
        }
        stalled
    }

    fn eligibility(
        &self,
        drop: &Drop,
        settings: &Settings,
        now: DateTime<Utc>,
        policy: &DropPolicy,
    ) -> Eligibility {
        if drop.claimed {
            Eligibility::Claimed
        } else if self.expired(now) || drop.ends_at <= now {
            Eligibility::Expired
        } else if self.upcoming(now) || now < drop.starts_at {
            Eligibility::Upcoming
        } else if policy.reasons.contains_key(&drop.id) {
            Eligibility::Ignored
        } else if drop.watch_reward() && drop.confirmed_minutes >= drop.required_minutes {
            Eligibility::AwaitingClaim
        } else if !self.prerequisites_met(drop) {
            Eligibility::Prerequisite
        } else if !policy.mineable.contains(&drop.id) {
            if !settings.selected(&self.game.name)
                && !settings.auto_mine_badges
                && !settings.auto_mine_emotes
            {
                Eligibility::Unselected
            } else {
                Eligibility::Filtered
            }
        } else if drop.estimated_minutes >= MAX_ESTIMATED_MINUTES {
            Eligibility::AwaitingConfirmation
        } else {
            Eligibility::Ready
        }
    }

    pub fn priority_context(&self, settings: &Settings, now: DateTime<Utc>) -> PriorityContext {
        match self.mining_priority(settings, now) {
            MiningPriority::Deadline(deadline, _) => {
                let policy = self.mining_policy(settings, now);
                let deadlines = self.priority_deadlines(settings, now, &policy);
                PriorityContext {
                    reason: if settings.mining_priority_mode == MiningPriorityMode::ShortEvents {
                        PriorityReason::ShortEvent
                    } else {
                        PriorityReason::EndingSoonest
                    },
                    deadline: Some(deadline),
                    target_ids: {
                        let mut targets: Vec<_> = self
                            .drops
                            .iter()
                            .filter(|drop| {
                                self.drop_eligible(
                                    drop,
                                    &policy,
                                    now,
                                    now + Duration::nanoseconds(1),
                                )
                            })
                            .filter_map(|drop| deadlines.get(&drop.id))
                            .filter(|priority| priority.deadline == deadline)
                            .flat_map(|priority| priority.targets.clone())
                            .collect();
                        targets.sort();
                        targets.dedup();
                        targets
                    },
                }
            }
            MiningPriority::Automatic(_) => PriorityContext {
                reason: if self.mining_policy(settings, now).mineable.is_empty() {
                    PriorityReason::NotSelected
                } else {
                    PriorityReason::AutomaticReward
                },
                ..Default::default()
            },
            _ => PriorityContext::default(),
        }
    }

    pub fn view(&self, settings: &Settings, now: DateTime<Utc>) -> CampaignView {
        let policy = self.policy(settings);
        let mining_policy = self.mining_policy(settings, now);
        let deadlines = self.priority_deadlines(settings, now, &mining_policy);
        let drops: Vec<_> = self
            .drops
            .iter()
            .filter(|d| d.watch_reward())
            .map(|drop| {
                let reason = policy.reasons.get(&drop.id);
                let mineable = policy.mineable.contains(&drop.id);
                DropView {
                    eligibility: self.eligibility(drop, settings, now, &mining_policy),
                    prerequisites: drop.prerequisites.clone(),
                    effective_starts_at: Some(self.starts_at.max(drop.starts_at)),
                    effective_ends_at: Some(self.ends_at.min(drop.ends_at)),
                    priority_deadline: deadlines.get(&drop.id).map(|priority| priority.deadline),
                    id: drop.id.clone(),
                    name: drop.name.clone(),
                    current_minutes: drop.current_minutes(),
                    confirmed_minutes: drop.confirmed_minutes,
                    confirmed_at: drop.confirmed_at,
                    required_minutes: drop.required_minutes,
                    progress: drop.progress(),
                    is_claimed: drop.claimed,
                    can_claim: drop.can_claim(self.ends_at, now),
                    is_ignored: reason.is_some(),
                    is_mineable: mineable,
                    is_skipped: !drop.claimed && reason.is_none() && !mineable,
                    ignored_reason: reason.map(|r| r.kind().to_owned()),
                    ignored_keyword: reason.and_then(|r| r.keyword()).map(str::to_owned),
                    ignored_precondition: reason.and_then(|r| r.precondition()).map(str::to_owned),
                    benefits: drop
                        .benefits
                        .iter()
                        .map(|b| BenefitView {
                            name: b.name.clone(),
                            kind: b.kind.clone(),
                            image_url: b.image_url.clone(),
                        })
                        .collect(),
                    starts_at: drop.starts_at,
                    ends_at: drop.ends_at,
                }
            })
            .collect();
        CampaignView {
            game_key: crate::config::fold(&self.game.name),
            selected: settings.selected(&self.game.name),
            saved_rank: settings
                .games_to_watch
                .iter()
                .position(|game| crate::config::fold(game) == crate::config::fold(&self.game.name))
                .map(|rank| rank + 1),
            priority: self.priority_context(settings, now),
            allowed_channels: self
                .allowed_channels
                .iter()
                .map(|channel| AllowedChannel {
                    login: channel.login.clone(),
                    name: channel.name.clone(),
                })
                .collect(),
            id: self.id.clone(),
            name: self.name.clone(),
            game_name: self.game.name.clone(),
            game_box_art_url: self.game.image_url.clone(),
            campaign_url: self.url(),
            link_url: self.link_url.clone(),
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            linked: self.linked,
            active: self.active(now),
            upcoming: self.upcoming(now),
            expired: self.expired(now),
            finished: !drops.is_empty() && drops.iter().all(|d| d.is_claimed),
            mining_finished: policy.mineable.is_empty(),
            claimed_drops: drops.iter().filter(|d| d.is_claimed).count(),
            total_drops: drops.len(),
            ignored_drops: drops.iter().filter(|d| d.is_ignored).count(),
            skipped_drops: drops.iter().filter(|d| d.is_skipped).count(),
            drops,
        }
    }

    pub fn progress(&self, drop: &Drop) -> Progress {
        Progress {
            drop_id: drop.id.clone(),
            drop_name: drop.name.clone(),
            campaign_id: self.id.clone(),
            campaign_name: self.name.clone(),
            game_name: self.game.name.clone(),
            current_minutes: drop.current_minutes(),
            confirmed_minutes: drop.confirmed_minutes,
            confirmed_at: drop.confirmed_at,
            required_minutes: drop.required_minutes,
            progress: drop.progress(),
            remaining_seconds: drop.remaining_minutes().saturating_mul(60),
        }
    }

    pub fn history_entry(&self, drop: &Drop, now: DateTime<Utc>) -> HistoryEntry {
        HistoryEntry {
            id: drop.id.clone(),
            claimed_at: now,
            claimed_at_is_observed: false,
            game: self.game.name.clone(),
            campaign: self.name.clone(),
            drop_name: drop.name.clone(),
            benefits: drop.benefits.iter().map(|b| b.name.clone()).collect(),
            required_minutes: drop.required_minutes,
            campaign_id: self.id.clone(),
            image_url: drop.image_url(),
        }
    }
}

pub fn wanted_items(
    campaigns: &[Campaign],
    settings: &Settings,
    now: DateTime<Utc>,
) -> Vec<WantedGame> {
    let mut result: Vec<WantedGame> = vec![];
    let mut campaigns: Vec<_> = campaigns
        .iter()
        .filter(|c| c.can_earn_within(settings, now, now + Duration::hours(1)))
        .collect();
    campaigns.sort_by_cached_key(|c| c.mining_priority(settings, now));
    for campaign in campaigns {
        let policy = campaign.mining_policy(settings, now);
        let deadlines = campaign.priority_deadlines(settings, now, &policy);
        let mut ordered: Vec<_> = campaign
            .drops
            .iter()
            .filter(|d| d.watch_reward() && now < d.ends_at && policy.mineable.contains(&d.id))
            .collect();
        ordered.sort_by_key(|d| {
            let deadline = deadlines.get(&d.id).map(|priority| priority.deadline);
            (
                deadline.unwrap_or(DateTime::<Utc>::MAX_UTC),
                deadline.is_some()
                    && !campaign.drop_eligible(d, &policy, now, now + Duration::nanoseconds(1)),
            )
        });
        let drops: Vec<_> = ordered
            .into_iter()
            .filter_map(|d| {
                // The mining policy already selects targets and required prerequisites.
                // A prerequisite must stay visible even when its own benefit type is off.
                let benefits = &d.benefits;
                (!benefits.is_empty()).then(|| WantedDrop {
                    id: d.id.clone(),
                    eligibility: campaign.eligibility(d, settings, now, &policy),
                    starts_at: Some(campaign.starts_at.max(d.starts_at)),
                    ends_at: Some(campaign.ends_at.min(d.ends_at)),
                    prerequisites: d.prerequisites.clone(),
                    priority_deadline: deadlines.get(&d.id).map(|priority| priority.deadline),
                    name: d.name.clone(),
                    benefits: benefits.iter().map(|b| b.name.clone()).collect(),
                    image_url: benefits
                        .iter()
                        .find(|b| !b.image_url.is_empty())
                        .map(|b| b.image_url.clone())
                        .unwrap_or_default(),
                })
            })
            .collect();
        if drops.is_empty() {
            continue;
        }
        let entry = WantedCampaign {
            priority: campaign.priority_context(settings, now),
            id: campaign.id.clone(),
            name: campaign.name.clone(),
            url: campaign.url(),
            drops,
        };
        if let Some(game) = result
            .iter_mut()
            .find(|g| g.game_id == Some(campaign.game.id))
        {
            game.campaigns.push(entry);
        } else {
            result.push(WantedGame {
                game_key: crate::config::fold(&campaign.game.name),
                saved_rank: settings
                    .games_to_watch
                    .iter()
                    .position(|game| {
                        crate::config::fold(game) == crate::config::fold(&campaign.game.name)
                    })
                    .map(|rank| rank + 1),
                game_name: campaign.game.name.clone(),
                game_icon: Some(campaign.game.image_url.clone()),
                game_id: Some(campaign.game.id),
                campaigns: vec![entry],
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CampaignArchive;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-09-26T12:00:00Z".parse().unwrap()
    }
    fn raw_drop(id: &str, prerequisites: &[&str]) -> Value {
        json!({"id":id,"name":id,"startAt":"2026-09-25T12:00:00Z","endAt":"2026-09-28T12:00:00Z",
            "requiredMinutesWatched":60,"preconditionDrops":prerequisites.iter().map(|id| json!({"id":id})).collect::<Vec<_>>(),
            "benefitEdges":[{"benefit":{"id":format!("b-{id}"),"name":id,"distributionType":"DIRECT_ENTITLEMENT","imageAssetURL":"https://static-cdn.jtvnw.net/reward.png"}}]})
    }
    fn raw_campaign(drops: Vec<Value>) -> Value {
        json!({"id":"c","name":"Campaign","game":{"id":"1","name":"Rust"},"startAt":"2026-09-25T12:00:00Z",
            "endAt":"2026-09-28T12:00:00Z","status":"ACTIVE","allow":{"channels":[],"isEnabled":true},"timeBasedDrops":drops})
    }
    fn campaign(drops: Vec<Value>) -> Campaign {
        Campaign::parse(&raw_campaign(drops), &HashMap::new(), now()).unwrap()
    }

    #[test]
    fn inventory_disputes_respect_request_order_identity_and_claim_evidence() {
        let mut previous = campaign(vec![raw_drop("reward", &[])]).drops.remove(0);
        previous.confirm(42, now());
        previous.reported_minutes = Some(42);
        let mut account = previous.clone();
        account.confirm(23, now() + Duration::seconds(2));

        let mut old_request = account.clone();
        old_request.reconcile_inventory(&previous, now() - Duration::seconds(1));
        assert_eq!(old_request.confirmed_minutes, 42);
        assert!(!old_request.progress_disputed);

        account.reconcile_inventory(&previous, now() + Duration::seconds(1));
        assert_eq!(account.confirmed_minutes, 23);
        assert!(account.progress_disputed);
        assert_eq!(account.reported_minutes, Some(42));
        assert_eq!(account.confirmed_at, Some(now() + Duration::seconds(2)));

        let mut public = campaign(vec![raw_drop("reward", &[])]).drops.remove(0);
        public.reconcile_inventory(&account, now() + Duration::seconds(3));
        assert_eq!(public.confirmed_minutes, 23);
        assert!(public.progress_disputed);
        assert_eq!(public.reported_minutes, Some(42));

        let mut other = campaign(vec![raw_drop("different", &[])]).drops.remove(0);
        other.reconcile_inventory(&account, now() + Duration::seconds(3));
        assert_eq!(other.confirmed_minutes, 0);
        assert!(!other.progress_disputed);
        let mut changed = account.clone();
        changed.confirm(10, now() + Duration::seconds(4));
        changed.required_minutes = 120;
        changed.reconcile_inventory(&account, now() + Duration::seconds(3));
        assert_eq!(changed.confirmed_minutes, 10);
        assert!(!changed.progress_disputed);

        let mut caught_up = account.clone();
        caught_up.confirm(42, now() + Duration::seconds(4));
        caught_up.reconcile_inventory(&account, now() + Duration::seconds(3));
        assert!(!caught_up.progress_disputed);
        assert_eq!(caught_up.reported_minutes, None);
        account.mark_claimed(now() + Duration::seconds(4));
        assert_eq!(account.confirmed_minutes, 60);
        assert!(!account.progress_disputed);
        assert_eq!(account.reported_minutes, None);
    }

    #[test]
    fn priority_explanations_match_opt_in_and_inherited_deadlines() {
        let mut c = campaign(vec![
            raw_drop("parent", &[]),
            raw_drop("reward", &["parent"]),
        ]);
        assert_eq!(
            c.priority_context(&Settings::default(), now()).reason,
            PriorityReason::NotSelected
        );
        c.drops[1].benefits[0].kind = "BADGE".into();
        let automatic = Settings {
            auto_mine_badges: true,
            ..Settings::default()
        };
        assert_eq!(
            c.priority_context(&automatic, now()).reason,
            PriorityReason::AutomaticReward
        );
        let settings = selected().patched(&json!({"mining_priority_mode":"ending_soonest", "mining_benefits":{"DIRECT_ENTITLEMENT":false}})).unwrap();
        c.drops[0].ends_at = now() + Duration::hours(1);
        let view = c.view(&settings, now());
        assert_eq!(view.priority.deadline, Some(c.drops[0].ends_at));
        assert_eq!(view.priority.target_ids, vec!["reward"]);
        assert_eq!(view.drops[0].eligibility, Eligibility::Ready);
        assert_eq!(view.drops[1].eligibility, Eligibility::Prerequisite);
        assert!(view.drops[0].confirmed_at.is_none());
    }

    #[test]
    fn malformed_eligibility_fields_cannot_become_unrestricted() {
        for mode in [
            "missing_allow",
            "null_allow",
            "missing_channels",
            "wrong_channels",
            "wrong_enabled",
            "null_channel",
            "missing_dependencies",
            "wrong_dependencies",
            "null_dependency",
            "missing_benefits",
            "wrong_benefits",
        ] {
            let mut raw = raw_campaign(vec![raw_drop("successor", &["parent"])]);
            match mode {
                "missing_allow" => {
                    raw.as_object_mut().unwrap().remove("allow");
                }
                "null_allow" => raw["allow"] = Value::Null,
                "missing_channels" => {
                    raw["allow"].as_object_mut().unwrap().remove("channels");
                }
                "wrong_channels" => raw["allow"]["channels"] = json!({}),
                "wrong_enabled" => raw["allow"]["isEnabled"] = json!("true"),
                "null_channel" => {
                    raw["allow"]["channels"] = json!([null, {"id":"10","login":"streamer"}])
                }
                "missing_dependencies" => {
                    raw["timeBasedDrops"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("preconditionDrops");
                }
                "wrong_dependencies" => {
                    raw["timeBasedDrops"][0]["preconditionDrops"] = json!({"id":"parent"})
                }
                "null_dependency" => raw["timeBasedDrops"][0]["preconditionDrops"] = json!([null]),
                "missing_benefits" => {
                    raw["timeBasedDrops"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("benefitEdges");
                }
                _ => raw["timeBasedDrops"][0]["benefitEdges"] = Value::Null,
            }
            assert!(
                Campaign::parse(&raw, &HashMap::new(), now()).is_err(),
                "{mode}"
            );
        }
    }

    #[test]
    fn explicit_empty_eligibility_and_nullable_acl_flags_remain_compatible() {
        for empty in [Value::Null, json!([])] {
            let mut raw = raw_campaign(vec![raw_drop("reward", &[])]);
            raw["allow"]["channels"] = empty.clone();
            raw["timeBasedDrops"][0]["preconditionDrops"] = empty;
            let parsed = Campaign::parse(&raw, &HashMap::new(), now()).unwrap();
            assert!(parsed.can_watch(&channel(), &selected(), now()));
        }
        for enabled in [
            None,
            Some(json!(true)),
            Some(json!(false)),
            Some(Value::Null),
        ] {
            let mut raw = raw_campaign(vec![raw_drop("reward", &[])]);
            raw["allow"]["channels"] = json!([{"id":"11","login":"restricted"}]);
            if let Some(enabled) = &enabled {
                raw["allow"]["isEnabled"] = enabled.clone();
            } else {
                raw["allow"].as_object_mut().unwrap().remove("isEnabled");
            }
            let parsed = Campaign::parse(&raw, &HashMap::new(), now()).unwrap();
            assert_eq!(
                parsed.can_watch(&channel(), &selected(), now()),
                matches!(enabled, Some(Value::Bool(false) | Value::Null))
            );
        }
        let parsed = campaign(vec![
            raw_drop("parent", &[]),
            raw_drop("successor", &["parent"]),
        ]);
        assert!(!parsed.prerequisites_met(&parsed.drops[1]));
    }
    fn selected() -> Settings {
        Settings::default()
            .patched(&json!({"games_to_watch":["Rust"]}))
            .unwrap()
    }
    fn channel() -> Channel {
        Channel {
            identity: ChannelIdentity {
                id: 10,
                login: "streamer".into(),
                name: "Streamer".into(),
            },
            game: Some(Game {
                id: 1,
                name: "Rust".into(),
                slug: "rust".into(),
                image_url: String::new(),
            }),
            broadcast_id: Some("b".into()),
            viewers: None,
            drops_enabled: true,
            acl_based: false,
            beacon_url: None,
        }
    }

    #[test]
    fn channel_artwork_uses_matching_catalog_game_without_changing_stream_identity() {
        let mut stream = channel();
        let mut catalog = vec![campaign(vec![raw_drop("coat", &[])])];
        assert_eq!(stream.view(Some(10), &catalog).game_icon, None);
        catalog[0].game.image_url = "https://static-cdn.jtvnw.net/catalog.jpg".into();
        let view = stream.view(Some(10), &catalog);
        assert_eq!(
            view.game_icon.as_deref(),
            Some(catalog[0].game.image_url.as_str())
        );
        assert_eq!(view.game_id, Some(1));
        assert_eq!(view.game.as_deref(), Some("Rust"));
        assert!(view.watching && view.online && view.drops_enabled);
        assert!(stream.game.as_ref().unwrap().image_url.is_empty());

        stream.game.as_mut().unwrap().image_url = "https://static-cdn.jtvnw.net/stream.jpg".into();
        assert_eq!(
            stream.view(None, &catalog).game_icon.as_deref(),
            Some("https://static-cdn.jtvnw.net/stream.jpg")
        );
        stream.game.as_mut().unwrap().image_url.clear();
        stream.game.as_mut().unwrap().id = 2;
        assert_eq!(stream.view(None, &catalog).game_icon, None);
        stream.game = None;
        assert_eq!(stream.view(None, &catalog).game_icon, None);
    }

    #[test]
    fn discovery_cannot_select_games_or_invent_account_progress() {
        let c = campaign(vec![raw_drop("coat", &[])]);
        assert!(!c.can_watch(&channel(), &Settings::default(), now()));
        assert!(wanted_items(std::slice::from_ref(&c), &Settings::default(), now()).is_empty());
        assert!(c.can_watch(&channel(), &selected(), now()));
        let view = c.view(&selected(), now());
        assert_eq!(view.linked, None);
        assert_eq!(view.drops[0].confirmed_at, None);
        assert_eq!(view.drops[0].confirmed_minutes, 0);
        assert!(!view.finished);
    }

    #[test]
    fn mining_priority_follows_the_event_prerequisite_not_unrelated_rewards() {
        let mut c = campaign(vec![
            raw_drop("ordinary", &[]),
            raw_drop("parent", &[]),
            raw_drop("event", &["parent"]),
        ]);
        c.drops[0].required_minutes = 5;
        c.drops[2].starts_at = now();
        c.drops[2].ends_at = now() + Duration::hours(3);
        let settings = selected()
            .patched(&json!({"mining_priority_mode":"short_events"}))
            .unwrap();
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "parent");
        assert_eq!(c.first_drop(&selected(), now()).unwrap().id, "ordinary");
        let mut hidden_parent = c.clone();
        hidden_parent.drops[1].benefits.clear();
        assert_eq!(
            hidden_parent.first_drop(&settings, now()).unwrap().id,
            "parent"
        );
        let queue = wanted_items(&[hidden_parent], &settings, now());
        let names: Vec<_> = queue[0].campaigns[0]
            .drops
            .iter()
            .map(|drop| drop.name.as_str())
            .collect();
        assert_eq!(names, ["event", "ordinary"]);
        c.drops[1].confirm(60, now());
        assert!(!c.drops[1].claimed);
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "ordinary");
        c.drops[1].mark_claimed(now());
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "event");
        c.drops[2].confirm(60, now());
        assert!(!c.drops[2].claimed);
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "ordinary");
    }

    #[test]
    fn mining_priority_uses_effective_drop_windows_and_confirmed_completion() {
        let mut c = campaign(vec![raw_drop("reward", &[])]);
        let short = selected()
            .patched(&json!({"mining_priority_mode":"short_events"}))
            .unwrap();
        let ending = selected()
            .patched(&json!({"mining_priority_mode":"ending_soonest"}))
            .unwrap();
        c.drops[0].ends_at = now() + Duration::hours(1);
        let end = c.drops[0].ends_at;
        // A long-running reward ending tonight is not a short event.
        assert_eq!(
            c.mining_priority(&short, now()),
            MiningPriority::Selected(0)
        );
        assert_eq!(
            c.mining_priority(&ending, now()),
            MiningPriority::Deadline(end, 0)
        );
        c.drops[0].starts_at = end - Duration::hours(24);
        assert_eq!(
            c.mining_priority(&short, now()),
            MiningPriority::Deadline(end, 0)
        );
        c.drops[0].starts_at -= Duration::nanoseconds(1);
        assert_eq!(
            c.mining_priority(&short, now()),
            MiningPriority::Selected(0)
        );
        c.starts_at = now();
        c.ends_at = now() + Duration::minutes(30);
        let deadline = MiningPriority::Deadline(c.ends_at, 0);
        assert_eq!(c.mining_priority(&short, now()), deadline);
        c.drops[0].confirm(59, now());
        c.drops[0].estimated_minutes = 1;
        assert_eq!(c.drops[0].remaining_minutes(), 0);
        assert_eq!(c.mining_priority(&short, now()), deadline);
        assert!(!c.drops[0].claimed);
        for patch in [
            json!({"drop_name_blacklist":["reward"]}),
            json!({"mining_benefits":{"DIRECT_ENTITLEMENT":false}}),
        ] {
            assert_eq!(
                c.mining_priority(&short.patched(&patch).unwrap(), now()),
                MiningPriority::Selected(0)
            );
        }
        assert_eq!(
            c.mining_priority(&short, c.ends_at),
            MiningPriority::Selected(0)
        );
        c.drops[0].confirm(60, now());
        assert_eq!(
            c.mining_priority(&short, now()),
            MiningPriority::Selected(0)
        );
        assert!(!c.drops[0].claimed);
    }

    #[test]
    fn mining_priority_does_not_boost_unreachable_or_filtered_event_branches() {
        // Deliberately put dependents before their shared prerequisites.
        let mut c = campaign(vec![
            raw_drop("event", &["left", "right"]),
            raw_drop("ordinary", &[]),
            raw_drop("left", &[]),
            raw_drop("right", &[]),
        ]);
        c.drops[0].starts_at = now();
        c.drops[0].ends_at = now() + Duration::hours(3);
        c.drops[0].benefits[0].kind = "EMOTE".into();
        c.drops[1].benefits[0].kind = "EMOTE".into();
        c.drops[1].required_minutes = 5;
        let settings = selected().patched(&json!({"mining_priority_mode":"short_events", "mining_benefits":{"DIRECT_ENTITLEMENT":false}})).unwrap();
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "left");
        c.drops[3].ends_at = now();
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Selected(0)
        );
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "ordinary");
        c.drops[3].ends_at = c.ends_at;
        c.drops[3].starts_at = c.drops[0].ends_at;
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Selected(0)
        );
        c.drops[3].starts_at = c.starts_at;
        c.drops[3].confirm(60, now());
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "left");
        assert!(!c.prerequisites_met(&c.drops[0]));
        let ignored = settings
            .patched(&json!({"drop_name_blacklist":["event"]}))
            .unwrap();
        assert_eq!(
            c.mining_priority(&ignored, now()),
            MiningPriority::Selected(0)
        );
        c.drops[0].prerequisites.push("missing".into());
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Selected(0)
        );
        c.drops[0].prerequisites.pop();
        c.drops[2].prerequisites.push("event".into());
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Selected(0)
        );
    }

    #[test]
    fn mining_priority_shares_the_earliest_deadline_across_dependency_branches() {
        let mut c = campaign(vec![
            raw_drop("ordinary", &[]),
            raw_drop("late", &["shared"]),
            raw_drop("early", &["shared"]),
            raw_drop("shared", &[]),
        ]);
        for index in [1, 2] {
            c.drops[index].starts_at = now();
            c.drops[index].ends_at = now() + Duration::hours(4 - index as i64);
        }
        let settings = selected()
            .patched(&json!({"mining_priority_mode":"short_events"}))
            .unwrap();
        let early_end = c.drops[2].ends_at;
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Deadline(early_end, 0)
        );
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "shared");
        assert_eq!(
            wanted_items(&[c.clone()], &settings, now())[0].campaigns[0].drops[0].name,
            "shared"
        );
        c.drops[2].mark_claimed(now());
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Deadline(c.drops[1].ends_at, 0)
        );
        c.drops[3].ends_at = now() + Duration::minutes(30);
        assert_eq!(
            c.mining_priority(&settings, now()),
            MiningPriority::Deadline(c.drops[3].ends_at, 0)
        );
    }

    #[test]
    fn timing_prerequisites_and_estimate_ceiling_control_eligibility() {
        let mut c = campaign(vec![raw_drop("first", &[]), raw_drop("second", &["first"])]);
        assert!(c.can_watch(&channel(), &selected(), c.starts_at));
        assert!(!c.can_watch(
            &channel(),
            &selected(),
            c.starts_at - Duration::nanoseconds(1)
        ));
        assert!(!c.can_watch(&channel(), &selected(), c.ends_at));
        assert_eq!(c.first_drop(&selected(), now()).unwrap().id, "first");
        for _ in 0..MAX_ESTIMATED_MINUTES - 1 {
            assert!(!c.bump_estimates(&selected(), now()));
        }
        assert!(c.bump_estimates(&selected(), now()));
        assert!(!c.can_watch(&channel(), &selected(), now()));
        assert_eq!(c.drops[0].confirmed_minutes, 0);
        assert_eq!(c.drops[1].estimated_minutes, 0);
        c.drops[0].confirm(0, now());
        assert!(c.can_watch(&channel(), &selected(), now()));
        c.drops[0].mark_claimed(now());
        assert_eq!(c.first_drop(&selected(), now()).unwrap().id, "second");
        assert_eq!(c.policy(&selected()).remaining_minutes(), 60);
    }

    #[test]
    fn campaign_and_drop_dates_leave_room_for_scheduling_and_claim_deadlines() {
        for path in [
            "/startAt",
            "/endAt",
            "/timeBasedDrops/0/startAt",
            "/timeBasedDrops/0/endAt",
        ] {
            for at in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
                let mut raw = raw_campaign(vec![raw_drop("coat", &[])]);
                *raw.pointer_mut(path).unwrap() = json!(at.to_rfc3339());
                assert!(
                    Campaign::parse(&raw, &HashMap::new(), now()).is_err(),
                    "{path}: {at}"
                );
            }
        }
        let mut raw = raw_campaign(vec![raw_drop("coat", &[])]);
        raw["startAt"] = json!((DateTime::<Utc>::MIN_UTC + Duration::hours(1)).to_rfc3339());
        raw["endAt"] = json!((DateTime::<Utc>::MAX_UTC - Duration::hours(24)).to_rfc3339());
        let mut campaign = Campaign::parse(&raw, &HashMap::new(), now()).unwrap();
        campaign.drops[0].claim_id = Some("instance".into());
        assert!(campaign.drops[0].can_claim(campaign.ends_at, now()));
    }

    #[test]
    fn claims_are_independent_of_selection_ignore_and_expiry_until_strict_grace_end() {
        let mut c = campaign(vec![raw_drop("coat", &[])]);
        c.drops[0].claim_id = Some("instance".into());
        let settings = Settings::default()
            .patched(&json!({"drop_name_blacklist":["coat"]}))
            .unwrap();
        assert!(c.drops[0].can_claim(c.ends_at, c.ends_at + Duration::hours(23)));
        assert!(!c.drops[0].can_claim(c.ends_at, c.ends_at + Duration::hours(24)));
        assert!(c.view(&settings, now()).drops[0].can_claim);
        c.drops[0].mark_claimed(now());
        assert!(!c.drops[0].can_claim(c.ends_at, now()));
        assert!(c.view(&settings, now()).finished);
    }

    #[test]
    fn every_benefit_needs_account_evidence_within_the_reward_window() {
        let mut drop = raw_drop("coat", &[]);
        drop["benefitEdges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"benefit":{"id":"second","name":"Extra"}}));
        let raw = raw_campaign(vec![drop]);
        let mut awards = HashMap::from([("b-coat".into(), now())]);
        assert!(!Campaign::parse(&raw, &awards, now()).unwrap().drops[0].claimed);
        awards.insert("second".into(), "2026-09-28T12:00:00Z".parse().unwrap());
        assert!(!Campaign::parse(&raw, &awards, now()).unwrap().drops[0].claimed);
        awards.insert("second".into(), now());
        let claimed = Campaign::parse(&raw, &awards, now()).unwrap();
        assert!(claimed.drops[0].claimed);
        assert_eq!(claimed.drops[0].confirmed_minutes, 60);
        let mut explicit = raw;
        explicit["timeBasedDrops"][0]["self"] =
            json!({"isClaimed":false,"currentMinutesWatched":7});
        assert!(!Campaign::parse(&explicit, &awards, now()).unwrap().drops[0].claimed);
    }

    #[test]
    fn incomplete_metadata_cannot_create_or_erase_confirmed_completion() {
        let drop = raw_drop("coat", &[]);
        let awards = HashMap::from([("b-coat".into(), now())]);
        let raw = raw_campaign(vec![drop]);
        let mut missing_benefit = raw.clone();
        missing_benefit["timeBasedDrops"][0]["benefitEdges"]
            .as_array_mut()
            .unwrap()
            .push(Value::Null);
        assert!(Campaign::parse(&missing_benefit, &awards, now()).is_err());
        let mut missing_drop = raw.clone();
        missing_drop["timeBasedDrops"]
            .as_array_mut()
            .unwrap()
            .push(Value::Null);
        assert!(Campaign::parse(&missing_drop, &awards, now()).is_err());
        for incomplete in [
            json!({}),
            json!({"isClaimed":false}),
            json!({"currentMinutesWatched":4}),
        ] {
            let mut unknown = raw.clone();
            unknown["timeBasedDrops"][0]["self"] = incomplete;
            assert!(Campaign::parse(&unknown, &awards, now()).is_err());
        }
        let c = Campaign::parse(&raw, &awards, now()).unwrap();
        assert!(c.view(&selected(), now()).finished);
    }

    #[test]
    fn ignored_dependencies_prune_only_unused_branches() {
        let mut starter = raw_drop("starter", &[]);
        starter["benefitEdges"] = json!([]);
        let mut c = campaign(vec![
            starter,
            raw_drop("coat", &["starter"]),
            raw_drop("boots", &["starter"]),
            raw_drop("dependent", &["coat"]),
        ]);
        let settings = selected()
            .patched(&json!({"drop_name_blacklist":["coat"]}))
            .unwrap();
        let policy = c.policy(&settings);
        assert_eq!(
            policy.mineable,
            HashSet::from(["starter".into(), "boots".into()])
        );
        assert_eq!(policy.reasons["dependent"].precondition(), Some("coat"));
        let view = c.view(&settings, now());
        assert_eq!(
            (view.claimed_drops, view.ignored_drops, view.skipped_drops),
            (0, 2, 0)
        );
        c.drops[2].mark_claimed(now());
        assert!(!c.policy(&settings).mineable.contains("starter"));
        assert_eq!(c.view(&settings, now()).skipped_drops, 1);
        c.drops[1].mark_claimed(now());
        assert!(c.policy(&settings).mineable.contains("dependent"));
        assert!(c.prerequisites_met(&c.drops[3]));
    }

    #[test]
    fn missing_prerequisites_and_cycles_are_blocked_but_claimed_cycles_are_safe() {
        let mut c = campaign(vec![
            raw_drop("missing", &["absent"]),
            raw_drop("a", &["b"]),
            raw_drop("b", &["a"]),
        ]);
        assert!(c.policy(&selected()).mineable.is_empty());
        assert!(!c.can_watch(&channel(), &selected(), now()));
        c.drops[1].mark_claimed(now());
        let policy = c.policy(&selected());
        assert_eq!(policy.mineable, HashSet::from(["b".into()]));
        assert_eq!(policy.remaining_minutes(), 60);
    }

    #[test]
    fn special_categories_require_real_enabled_acl_for_cross_category_streams() {
        let mut raw = raw_campaign(vec![raw_drop("reward", &[])]);
        raw["game"]["id"] = "509663".into();
        raw["game"]["name"] = "Special Events".into();
        raw["allow"]["channels"] = json!([{"id":"10","name":"streamer"}]);
        let mut c = Campaign::parse(&raw, &HashMap::new(), now()).unwrap();
        let settings = selected()
            .patched(&json!({"games_to_watch":["Special Events"]}))
            .unwrap();
        let mut stream = channel();
        stream.drops_enabled = false;
        stream.game = None;
        assert!(c.can_watch(&stream, &settings, now()));
        assert!(!c.can_watch(&stream, &selected(), now()));
        stream.broadcast_id = None;
        assert!(!c.can_watch(&stream, &settings, now()));
        assert!(c.channel_eligible(&stream, true));
        stream = channel();
        c.allowed_channels.clear();
        assert!(!c.can_watch(&stream, &settings, now()));
        raw["allow"]["isEnabled"] = false.into();
        assert!(
            !Campaign::parse(&raw, &HashMap::new(), now())
                .unwrap()
                .can_watch(&stream, &settings, now())
        );
        raw["allow"]["isEnabled"] = Value::Null;
        assert!(
            !Campaign::parse(&raw, &HashMap::new(), now())
                .unwrap()
                .can_watch(&stream, &settings, now())
        );
        c.game.id = 1;
        c.game.name = "Rust".into();
        assert!(c.can_watch(&stream, &selected(), now()));
        stream.drops_enabled = false;
        assert!(!c.can_watch(&stream, &selected(), now()));
    }

    #[test]
    fn automatic_types_target_rewards_and_dependencies_without_selecting_games() {
        let mut emote = raw_drop("emote", &["starter"]);
        emote["benefitEdges"][0]["benefit"]["distributionType"] = "EMOTE".into();
        let mut badge = raw_drop("badge", &[]);
        badge["benefitEdges"][0]["benefit"]["distributionType"] = "BADGE".into();
        let mut c = campaign(vec![
            raw_drop("starter", &[]),
            emote,
            badge,
            raw_drop("item", &[]),
        ]);
        let defaults = Settings::default();
        assert!(!c.can_mine(&defaults, now()));
        let settings = defaults.patched(&json!({"auto_mine_emotes":true})).unwrap();
        assert!(settings.games_to_watch.is_empty());
        assert_eq!(
            c.mining_policy(&settings, now()).mineable,
            HashSet::from(["starter".into(), "emote".into()])
        );
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "starter");
        assert!(c.can_watch(&channel(), &settings, now()));
        let queue = wanted_items(&[c.clone()], &settings, now());
        assert_eq!(
            queue[0].campaigns[0]
                .drops
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            ["starter", "emote"]
        );
        c.bump_estimates(&settings, now());
        assert_eq!(
            c.drops
                .iter()
                .map(|d| d.estimated_minutes)
                .collect::<Vec<_>>(),
            [1, 0, 0, 0]
        );
        c.drops[0].mark_claimed(now());
        assert_eq!(c.first_drop(&settings, now()).unwrap().id, "emote");
        c.drops[1].mark_claimed(now());
        assert!(!c.can_mine(&settings, now()));
        let badges = settings.patched(&json!({"auto_mine_badges":true})).unwrap();
        assert_eq!(c.first_drop(&badges, now()).unwrap().id, "badge");
        assert!(
            !c.can_mine(
                &badges
                    .patched(&json!({"mining_benefits":{"BADGE":false}}))
                    .unwrap(),
                now()
            )
        );
        assert!(
            !c.can_mine(
                &badges
                    .patched(&json!({"drop_name_blacklist":["badge"]}))
                    .unwrap(),
                now()
            )
        );
        c.drops[2].required_minutes = 0;
        assert!(!c.can_mine(&badges, now()));
    }

    #[test]
    fn automatic_targets_respect_ignored_dependencies_expiry_and_display_filters() {
        let mut target = raw_drop("emote", &["starter"]);
        target["benefitEdges"][0]["benefit"]["distributionType"] = "EMOTE".into();
        let mut c = campaign(vec![raw_drop("starter", &[]), target]);
        let settings = Settings::default()
            .patched(&json!({"auto_mine_emotes":true,
            "inventory_filters":{"show_benefit_emote":false},
            "mining_benefits":{"DIRECT_ENTITLEMENT":false}}))
            .unwrap();
        assert!(
            c.can_mine(&settings, now()),
            "required item remains eligible despite item filter"
        );
        assert_eq!(
            wanted_items(&[c.clone()], &settings, now())[0].campaigns[0]
                .drops
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            ["starter", "emote"]
        );
        assert!(
            !c.can_mine(
                &settings
                    .patched(&json!({"drop_name_blacklist":["starter"]}))
                    .unwrap(),
                now()
            )
        );
        c.drops[1].ends_at = now();
        assert!(!c.can_mine(&settings, now()));
        assert!(wanted_items(&[c], &settings, now()).is_empty());
    }

    #[test]
    fn queue_omits_subscription_expired_and_ignored_rewards_but_keeps_sequences_and_art() {
        let mut subscription = raw_drop("subscription", &[]);
        subscription["requiredMinutesWatched"] = 0.into();
        let mut expired = raw_drop("expired", &[]);
        expired["endAt"] = "2026-09-26T11:00:00Z".into();
        let mut upcoming = raw_drop("upcoming", &[]);
        upcoming["startAt"] = "2026-09-26T12:30:00Z".into();
        let c = campaign(vec![
            subscription,
            expired,
            upcoming,
            raw_drop("coat", &[]),
            raw_drop("boots", &["coat"]),
            raw_drop("ignored", &[]),
        ]);
        let settings = selected()
            .patched(&json!({"drop_name_blacklist":["ignored"]}))
            .unwrap();
        let queue = wanted_items(&[c], &settings, now());
        let drops = &queue[0].campaigns[0].drops;
        assert_eq!(
            drops.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["upcoming", "coat", "boots"]
        );
        assert!(drops.iter().all(|d| d.image_url.ends_with("reward.png")));
        let empty = campaign(vec![]);
        assert!(!empty.view(&settings, now()).finished);
    }

    #[test]
    fn completed_archive_survives_restart_refresh_and_metadata_only_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let mut archive = CampaignArchive::load(dir.path());
        let mut c = campaign(vec![raw_drop("coat", &[])]);
        c.drops[0].mark_claimed(now());
        archive.update(&[c.view(&selected(), now())]).unwrap();
        let mut archive = CampaignArchive::load(dir.path());
        let historical = archive.merge(vec![], now() + Duration::days(90));
        assert!(historical[0].finished && historical[0].expired);
        assert!(!historical[0].active);
        c.drops[0].claimed = false;
        c.drops[0].confirmed_at = None;
        archive.update(&[c.view(&selected(), now())]).unwrap();
        assert!(archive.merge(vec![c.view(&selected(), now())], now())[0].finished);
        c.drops[0].confirm(3, now());
        archive.update(&[c.view(&selected(), now())]).unwrap();
        assert!(!archive.merge(vec![c.view(&selected(), now())], now())[0].finished);
        assert!(
            CampaignArchive::load(dir.path())
                .merge(vec![], now())
                .is_empty()
        );
        c.drops[0].mark_claimed(now());
        archive.update(&[c.view(&selected(), now())]).unwrap();
        c.drops
            .push(campaign(vec![raw_drop("new", &[])]).drops.remove(0));
        archive.update(&[c.view(&selected(), now())]).unwrap();
        assert!(
            CampaignArchive::load(dir.path())
                .merge(vec![], now())
                .is_empty()
        );
    }
}
