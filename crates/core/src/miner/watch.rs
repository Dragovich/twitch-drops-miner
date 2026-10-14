use super::*;

impl Mining {
    pub(super) fn watch_channel(&self, settings: &Settings) -> Option<&Channel> {
        if settings.mining_paused || self.claim_wait.is_some() {
            return None;
        }
        self.channels.iter().find(|c| {
            Some(c.identity.id) == self.watching
                && c.online()
                && (self.manual.is_some_and(|manual| {
                    manual.channel == c.identity.id
                        && manual.expires_at.is_none_or(|at| Instant::now() < at)
                }) || self
                    .campaigns
                    .iter()
                    .any(|campaign| campaign.can_watch(c, settings, Utc::now())))
        })
    }

    pub(super) fn schedule_playback(&mut self, settings: &Settings) {
        if self.busy.contains(&JobKind::Playback) || Instant::now() < self.next_playback {
            return;
        }
        let Some(channel) = self.watch_channel(settings) else {
            return;
        };
        let Some(mut state) = self
            .playback
            .as_ref()
            .filter(|p| p.matches(channel))
            .cloned()
            .or_else(|| Playback::new(channel))
        else {
            return;
        };
        self.playback = Some(state.clone());
        let cancel = self.client.http.cancel.child_token();
        self.playback_cancel = Some(cancel.clone());
        let client = self.client.clone();
        let requested_at = Instant::now();
        self.next_playback = requested_at + POLL_INTERVAL;
        self.spawn(JobKind::Playback, async move {
            let result = tokio::select! {biased;
                _ = cancel.cancelled() => Err(TwitchError::Cancelled),
                result = state.poll(&client) => result,
            };
            Job::Playback {
                state: Box::new(state),
                requested_at,
                cancel,
                result,
            }
        });
    }

    pub(super) fn apply_manual(&mut self, intent: &Intent, language: crate::config::Language) {
        if intent.manual_revision != self.seen.manual_revision
            && let Some(login) = &intent.channel_login
        {
            self.lookup = Some((login.clone(), intent.manual_revision));
            self.manual_pending = Some(login.clone());
            self.manual_error = None;
            self.seen.manual_revision = intent.manual_revision;
            self.next_retry = Instant::now();
            self.publish = true;
            return;
        }
        if intent.manual_revision != self.seen.manual_revision
            && (intent.selected.is_none() || self.channels_loaded)
        {
            self.lookup = None;
            self.manual_pending = None;
            self.manual_error = None;
            self.manual = intent.selected.and_then(|id| {
                self.channels.iter().find(|c| c.identity.id == id)?;
                Some(ManualSelection::new(id, intent.manual_duration))
            });
            self.seen.manual_revision = intent.manual_revision;
            if intent.selected.is_some() && self.manual.is_none() {
                self.manual_error = Some(crate::app::message_in(
                    language,
                    "gui.channels.not_found",
                    &[],
                ));
            }
            self.publish = true;
        }
    }

    pub(super) async fn event(&mut self, event: Event) -> Result<(), TwitchError> {
        match event {
            Event::Unauthorized => return Err(TwitchError::Unauthorized),
            Event::Progress { id, minutes } => {
                let settings = self.app.settings.read().await.clone();
                self.confirm(&id, minutes, &settings);
            }
            Event::Claim { id, instance } => {
                if let Some(drop) = self
                    .campaigns
                    .iter_mut()
                    .flat_map(|c| &mut c.drops)
                    .find(|d| d.id == id)
                {
                    drop.claim_id = Some(instance);
                    self.claim_retry.remove(&id);
                } else {
                    self.refresh = true;
                }
            }
            Event::Notification(id) => {
                self.refresh = true;
                if self.notifications.len() < 256 {
                    self.notifications.insert(id);
                }
            }
            Event::Offline(id) => {
                self.channel_events.insert(id, Instant::now());
                self.beacon_events.insert(id, Instant::now());
                if let Some(channel) = self.channels.iter_mut().find(|c| c.identity.id == id) {
                    channel.broadcast_id = None;
                    channel.game = None;
                    channel.viewers = None;
                    channel.drops_enabled = false;
                    channel.beacon_url = None;
                    self.publish = true;
                }
                self.refresh_channels.remove(&id);
            }
            Event::Changed(id) => {
                self.channel_events.insert(id, Instant::now());
                self.beacon_events.insert(id, Instant::now());
                if let Some(channel) = self.channels.iter_mut().find(|c| c.identity.id == id) {
                    // The old category is no longer evidence of eligibility.
                    channel.broadcast_id = None;
                    channel.game = None;
                    channel.drops_enabled = false;
                    channel.beacon_url = None;
                    self.publish = true;
                    self.refresh_channels
                        .insert(id, Instant::now() + CHANNEL_DELAY);
                }
            }
            Event::Viewers { id, count } => {
                self.viewer_events.insert(id, Instant::now());
                if let Some(channel) = self.channels.iter_mut().find(|c| c.identity.id == id) {
                    if channel.online() {
                        channel.viewers = Some(count);
                        if let Some(channel) = self
                            .app
                            .snapshot
                            .write()
                            .await
                            .channels
                            .iter_mut()
                            .find(|c| c.id == id)
                        {
                            channel.viewers = Some(count);
                        }
                    } else {
                        self.refresh_channels
                            .entry(id)
                            .or_insert(Instant::now() + CHANNEL_DELAY);
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn confirm(&mut self, id: &str, minutes: u32, settings: &Settings) -> bool {
        let eligible = self
            .watching
            .is_some_and(|channel| self.progress_eligible(id, channel, settings));
        if let Some(drop) = self
            .campaigns
            .iter_mut()
            .flat_map(|c| &mut c.drops)
            .find(|d| d.id == id)
        {
            // Delayed progress cannot undo confirmed completion or restore an old card.
            let previous_report = drop.reported_minutes.unwrap_or(drop.confirmed_minutes);
            if minutes < previous_report.max(drop.confirmed_minutes) {
                return false;
            }
            let advanced = minutes > previous_report;
            if !drop.progress_disputed {
                drop.confirm(minutes, Utc::now());
            }
            drop.reported_minutes = (!drop.claimed).then_some(minutes.min(drop.required_minutes));
            drop.estimated_minutes = 0;
            let disputed = drop.progress_disputed;
            let completed = !drop.claimed
                && drop.watch_reward()
                && drop.confirmed_minutes >= drop.required_minutes;
            let reported = self
                .watching
                .and_then(|channel| self.reported_drop(id, channel, settings));
            let blocked = reported.is_some_and(|(c, d)| !c.prerequisites_met(d));
            let predecessor = self.last_progress.as_ref().is_some_and(|(current, _)| {
                self.watching
                    .and_then(|channel| self.reported_drop(current, channel, settings))
                    .is_some_and(|(c, d)| c.is_prerequisite(id, d))
            });
            // PubSub has no channel/request ordering; a predecessor cannot displace
            // the successor already reported for this watch. Still retain its minutes.
            if !predecessor
                && (reported.is_some() && (advanced || self.last_progress.is_none())
                    || completed
                        && eligible
                        && self
                            .last_progress
                            .as_ref()
                            .is_none_or(|(previous, _)| previous == id))
            {
                self.last_progress = Some((id.to_owned(), Instant::now()));
            }
            if completed || blocked || disputed {
                self.request_progress_refresh();
            }
            self.publish = true;
            true
        } else {
            self.request_progress_refresh();
            false
        }
    }

    pub(super) fn request_progress_refresh(&mut self) {
        // Unknown rewards, disputed progress and completion need account inventory evidence.
        if Instant::now() >= self.next_progress_refresh {
            self.next_progress_refresh = Instant::now() + Duration::from_secs(60);
            self.refresh = true;
        }
    }

    pub(super) fn progress_eligible(&self, id: &str, channel: u64, settings: &Settings) -> bool {
        let now = Utc::now();
        self.reported_drop(id, channel, settings)
            .is_some_and(|(c, d)| {
                self.manual.is_some()
                    || c.drop_eligible(
                        d,
                        &c.mining_policy(settings, now),
                        now,
                        now + chrono::Duration::nanoseconds(1),
                    )
            })
    }

    pub(super) fn reported_drop(
        &self,
        id: &str,
        channel: u64,
        settings: &Settings,
    ) -> Option<(&Campaign, &Drop)> {
        let now = Utc::now();
        let channel = self.channels.iter().find(|c| c.identity.id == channel)?;
        self.campaigns
            .iter()
            .filter(|c| c.active(now) && c.matches_channel(channel))
            .find_map(|c| {
                c.drops
                    .iter()
                    .find(|d| {
                        d.id == id
                            && !d.claimed
                            && d.confirmed_minutes < d.required_minutes
                            && d.starts_at <= now
                            && now < d.ends_at
                            && if self.manual.is_some() {
                                c.prerequisites_met(d)
                            } else {
                                c.mining_policy(settings, now).mineable.contains(id)
                            // Actual successor progress can precede local prerequisite claim evidence.
                            && (c.prerequisites_met(d) || d.confirmed_minutes > 0)
                            }
                    })
                    .map(|d| (c, d))
            })
    }

    pub(super) async fn reselect(&mut self, settings: &Settings) {
        let now = Utc::now();
        if self.playback.as_ref().is_some_and(|state| {
            self.channels
                .iter()
                .find(|c| Some(c.identity.id) == self.watching)
                .is_none_or(|c| !state.matches(c))
        }) {
            if let Some(cancel) = &self.playback_cancel {
                cancel.cancel();
            }
            self.playback = None;
            self.next_playback = Instant::now();
        }
        if self.paused != settings.mining_paused {
            self.paused = settings.mining_paused;
            self.cancel_watch();
            // Fence completed requests from before this pause/resume transition.
            self.watch_started = Instant::now();
            if let Some(channel) = self.watching {
                self.beacon_events.insert(channel, self.watch_started);
            }
            self.poll_at = None;
            self.claim_wait = None;
            self.watch_failures = 0;
            self.next_watch = Instant::now();
            self.next_playback = Instant::now();
            self.publish = true;
            let key = if self.paused {
                "status.paused"
            } else {
                "status.resumed"
            };
            self.app
                .status(crate::app::message_in(settings.language, key, &[]))
                .await;
            self.app.activity(key, &[]).await;
        }
        if self
            .manual
            .is_some_and(|manual| manual.expires_at.is_some_and(|at| Instant::now() >= at))
        {
            self.manual = None;
            self.publish = true;
        }
        let mut next = select_channel(
            &self.channels,
            &self.campaigns,
            settings,
            now,
            self.watching,
            self.manual.map(|manual| manual.channel),
        );
        self.avoided.retain(|_, until| Instant::now() < *until);
        if self.manual.is_none() && next.is_some_and(|id| self.avoided.contains_key(&id)) {
            // Keep a failing stream only when no other eligible stream exists.
            let others: Vec<_> = self
                .channels
                .iter()
                .filter(|c| !self.avoided.contains_key(&c.identity.id))
                .cloned()
                .collect();
            let current = self.watching.filter(|id| !self.avoided.contains_key(id));
            next = select_channel(&others, &self.campaigns, settings, now, current, None).or(next);
        }
        if next != self.watching {
            self.cancel_watch();
            self.playback = None;
            self.next_playback = Instant::now();
            self.watching = next;
            self.watch_started = Instant::now();
            self.watch_failures = 0;
            self.next_watch = Instant::now();
            self.poll_at = None;
            self.last_progress = None;
            self.claim_wait = None;
            self.publish = true;
            if !settings.mining_paused
                && let Some(channel) = self.channels.iter().find(|c| Some(c.identity.id) == next)
            {
                let status = crate::app::message_in(
                    settings.language,
                    "status.watching",
                    &[("channel", &channel.identity.name)],
                );
                self.app.status(status.clone()).await;
                let mut event = crate::app::activity::ActivityEvent::new_in(
                    settings.language,
                    "status.watching",
                    crate::app::activity::Category::Mining,
                    crate::app::activity::Severity::Info,
                    &[("channel", &channel.identity.name)],
                );
                event.channel_id = Some(channel.identity.id);
                self.app.record_activity(event).await;
            }
        }
    }

    pub(super) fn cancel_watch(&self) {
        if let Some(cancel) = &self.playback_cancel {
            cancel.cancel();
        }
        if let Some(watch) = &self.watch_abort {
            watch.abort();
        }
    }

    pub(super) fn finish_manual(
        &mut self,
        result: Result<Option<Channel>, TwitchError>,
        requested_at: Instant,
        duration: Option<Duration>,
    ) -> Option<&'static str> {
        let mut resolved = match result {
            Ok(Some(resolved)) => resolved,
            Ok(None) => return Some("gui.channels.not_found"),
            Err(_) => return Some("gui.channels.lookup_failed"),
        };
        self.preserve_channel_events(std::slice::from_mut(&mut resolved), requested_at);
        let id = resolved.identity.id;
        self.channels.retain(|c| c.identity.id != id);
        self.channels.insert(0, resolved.clone());
        self.channels
            .truncate(crate::twitch::channels::MAX_CHANNELS);
        self.channel_events.insert(id, Instant::now());
        self.beacon_events.insert(id, Instant::now());
        self.manual = Some(ManualSelection::new(id, duration));
        self.channels_dirty = true;
        None
    }
}
