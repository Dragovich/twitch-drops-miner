use super::*;

impl Mining {
    pub(super) fn preserve_channel_events(
        &mut self,
        channels: &mut [Channel],
        requested_at: Instant,
    ) {
        for channel in channels {
            if channel.online()
                && self
                    .viewer_events
                    .get(&channel.identity.id)
                    .is_some_and(|at| *at > requested_at)
                && let Some(current) = self
                    .channels
                    .iter()
                    .find(|c| c.identity.id == channel.identity.id)
            {
                channel.viewers = current.viewers;
            }
            if self
                .channel_events
                .get(&channel.identity.id)
                .is_some_and(|at| *at > requested_at)
                && let Some(current) = self
                    .channels
                    .iter()
                    .find(|c| c.identity.id == channel.identity.id)
            {
                *channel = current.clone();
            }
            // Stream metadata never discovers beacon addresses. For the same
            // broadcast, the owner has the latest acknowledgement/invalidation.
            if let Some(current) = self.channels.iter().find(|c| {
                c.identity.id == channel.identity.id && c.broadcast_id == channel.broadcast_id
            }) {
                channel.beacon_url = current.beacon_url.clone();
            }
            if self.watching == Some(channel.identity.id)
                && self.channels.iter().any(|c| {
                    c.identity.id == channel.identity.id && c.broadcast_id != channel.broadcast_id
                })
            {
                // A new broadcast on the same channel also starts a new watch context.
                self.watch_started = Instant::now();
                self.last_progress = None;
                self.poll_at = None;
            }
        }
    }

    pub(super) fn set_transition(&mut self) {
        let now = Utc::now();
        self.next_transition = self
            .campaigns
            .iter()
            .flat_map(|c| {
                [c.starts_at, c.ends_at]
                    .into_iter()
                    .chain(c.drops.iter().flat_map(|d| [d.starts_at, d.ends_at]))
            })
            .flat_map(|at| [at - chrono::Duration::hours(1), at])
            .filter(|at| *at > now)
            .min();
    }

    pub(super) async fn idle_status(&self, settings: &Settings) {
        if settings.mining_paused {
            self.app
                .status(crate::app::message_in(
                    settings.language,
                    "status.paused",
                    &[],
                ))
                .await;
            return;
        }
        let key = if settings.games_to_watch.is_empty()
            && !settings.auto_mine_badges
            && !settings.auto_mine_emotes
        {
            "status.no_selection"
        } else if !self.campaigns.iter().any(|c| {
            c.can_earn_within(
                settings,
                Utc::now(),
                Utc::now() + chrono::Duration::hours(1),
            )
        }) {
            if self.status.available {
                "status.no_campaign"
            } else {
                "status.catalog_unavailable"
            }
        } else {
            "status.no_channel"
        };
        let text = crate::app::message_in(settings.language, key, &[]);
        self.app.status(text.clone()).await;
        self.app.activity(key, &[]).await;
    }

    pub(super) async fn publish(&self, settings: &Settings) -> Result<(), TwitchError> {
        self.publish_snapshot(settings, None).await
    }

    pub(super) async fn publish_snapshot(
        &self,
        settings: &Settings,
        refresh: Option<(u64, Option<String>)>,
    ) -> Result<(), TwitchError> {
        let now = Utc::now();
        let live: Vec<_> = self
            .campaigns
            .iter()
            .filter(|c| c.drops.iter().any(|d| d.watch_reward()))
            .map(|c| c.view(settings, now))
            .collect();
        let app = self.app.clone();
        let campaigns = tokio::task::spawn_blocking(move || {
            let mut archive = app.archive.blocking_lock();
            archive.update(&live)?;
            Ok::<_, anyhow::Error>(archive.merge(live, now))
        })
        .await
        .map_err(|_| TwitchError::Storage)?
        .map_err(|_| TwitchError::Storage)?;
        let mineable: Vec<_> = self
            .campaigns
            .iter()
            .filter(|c| c.can_mine(settings, now))
            .collect();
        let channels: Vec<_> = self
            .channels
            .iter()
            .filter(|channel| {
                self.manual
                    .is_some_and(|manual| manual.channel == channel.identity.id)
                    || mineable.iter().any(|c| c.matches_channel(channel))
            })
            .map(|c| {
                c.view(
                    self.watching.filter(|_| !settings.mining_paused),
                    &self.campaigns,
                )
            })
            .collect();
        let wanted = wanted_items(&self.campaigns, settings, now);
        let paused_progress = if settings.mining_paused {
            self.app.snapshot.read().await.current_drop.clone()
        } else {
            None
        };
        let retained = paused_progress.as_ref().and_then(|progress| {
            self.campaigns
                .iter()
                .find(|c| c.id == progress.campaign_id)
                .and_then(|c| {
                    c.drops
                        .iter()
                        .find(|d| d.id == progress.drop_id)
                        .map(|d| (c, d))
                })
        });
        let active = if paused_progress.is_some() {
            retained
        } else {
            self.channels
                .iter()
                .find(|c| Some(c.identity.id) == self.watching)
                .and_then(|channel| {
                    let reported = self
                        .last_progress
                        .as_ref()
                        .and_then(|(id, _)| self.reported_drop(id, channel.identity.id, settings));
                    if self.manual.is_some() {
                        return reported;
                    }
                    reported.or_else(|| {
                        self.campaigns
                            .iter()
                            .filter(|c| c.can_watch(channel, settings, now))
                            .filter_map(|c| c.first_drop(settings, now).map(|d| (c, d)))
                            .min_by_key(|(c, d)| {
                                (c.mining_priority(settings, now), d.remaining_minutes())
                            })
                    })
                })
        };
        let progress = active.map(|(c, d)| c.progress(d)).or(paused_progress);
        use crate::dto::{MiningState, MiningStatus};
        let mining = MiningStatus {
            state: if settings.mining_paused {
                MiningState::Paused
            } else if self.manual.is_some() {
                if self.watching.is_some() {
                    MiningState::ManualWatching
                } else {
                    MiningState::ManualOffline
                }
            } else if let Some((_, drop)) = active {
                if drop.confirmed_minutes >= drop.required_minutes {
                    MiningState::AwaitingClaim
                } else if drop.confirmed_at.is_none() {
                    MiningState::AwaitingProgress
                } else {
                    MiningState::Watching
                }
            } else if self
                .campaigns
                .iter()
                .any(|campaign| campaign.needs_claim_refresh(now))
            {
                MiningState::AwaitingClaim
            } else if settings.games_to_watch.is_empty()
                && !settings.auto_mine_badges
                && !settings.auto_mine_emotes
            {
                MiningState::NoSelection
            } else if self.watching.is_some() {
                MiningState::AwaitingProgress
            } else if self.status.checked_at.is_none() {
                MiningState::Discovering
            } else if !wanted.is_empty() {
                MiningState::WaitingChannel
            } else {
                MiningState::NoRewards
            },
            channel_id: self.watching.filter(|_| !settings.mining_paused),
            campaign_id: active.map(|(campaign, _)| campaign.id.clone()),
            drop_id: active.map(|(_, drop)| drop.id.clone()),
            priority: active
                .map(|(campaign, _)| campaign.priority_context(settings, now))
                .unwrap_or_default(),
        };
        let mut manual = self
            .manual
            .map(|manual| {
                let channel = self
                    .channels
                    .iter()
                    .find(|c| c.identity.id == manual.channel);
                ManualMode {
                    active: true,
                    game_name: channel
                        .and_then(|c| c.game.as_ref())
                        .map(|g| g.name.clone()),
                    channel_name: channel.map(|c| c.identity.name.clone()),
                    channel_id: channel.map(|c| c.identity.id),
                    expires_at: manual.expires_at.map(|at| {
                        now + chrono::Duration::from_std(
                            at.saturating_duration_since(Instant::now()),
                        )
                        .unwrap()
                    }),
                    ..ManualMode::default()
                }
            })
            .unwrap_or_default();
        manual.pending_channel = self.manual_pending.clone();
        manual.error = self.manual_error.clone();
        let games: Vec<_> = self
            .campaigns
            .iter()
            .map(|c| (c.game.name.clone(), ()))
            .collect::<BTreeMap<_, _>>()
            .into_keys()
            .collect();
        {
            let mut state = self.app.snapshot.write().await;
            state.campaigns = campaigns.clone();
            state.channels = channels.clone();
            state.current_drop = progress.clone();
            state.wanted_items = wanted.clone();
            state.manual_mode = manual.clone();
            state.inventory_status = self.status.clone();
            state.settings.games_available = games.clone();
            state.settings.refresh_game_keys();
            state.mining = mining;
            if let Some((sequence, error)) = refresh
                && state.inventory_refresh.sequence == sequence
                && state.inventory_refresh.state == RefreshState::Refreshing
            {
                state.inventory_refresh.sequence += 1;
                state.inventory_refresh.state = if error.is_some() {
                    RefreshState::Failed
                } else {
                    RefreshState::Refreshed
                };
                state.inventory_refresh.error = error;
            }
        }
        Ok(())
    }
}
