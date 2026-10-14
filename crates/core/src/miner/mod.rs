mod claims;
mod inventory;
mod session;
#[path = "watch.rs"]
mod watching;
use claims::*;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use tokio::{
    sync::{Mutex, Notify, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    app::{Application as App, Command, CommandRequest, message_in},
    config::Settings,
    domain::{Campaign, Channel, Drop, MAX_ESTIMATED_MINUTES, wanted_items},
    dto::{InventoryStatus, Login, ManualMode, RefreshState},
    store::{ClaimJournal, PendingClaim},
    twitch::{
        Endpoints, TwitchClient, TwitchError, TwitchHttp,
        channels::select_channel,
        inventory::Inventory,
        oauth::{DeviceLogin, Session},
        playback::{POLL_INTERVAL, Playback},
        pubsub::{Event, PubSub},
    },
};

const WATCH_INTERVAL: Duration = Duration::from_secs(59);
const PROGRESS_DELAY: Duration = Duration::from_secs(20);
const CHANNEL_DELAY: Duration = Duration::from_secs(2);
const FAILED_CHANNEL_COOLDOWN: Duration = Duration::from_secs(600);

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
struct Intent {
    refresh: u64,
    clear: u64,
    settings: u64,
    manual_revision: u64,
    selected: Option<u64>,
    channel_login: Option<String>,
    manual_duration: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ManualSelection {
    channel: u64,
    expires_at: Option<Instant>,
}
impl ManualSelection {
    fn new(channel: u64, duration: Option<Duration>) -> Self {
        Self {
            channel,
            expires_at: duration.map(|duration| Instant::now() + duration),
        }
    }
}

struct Generation {
    cancel: CancellationToken,
    confirmed: Arc<Notify>,
    retry: Arc<Notify>,
    task: JoinHandle<Result<(), TwitchError>>,
}

#[derive(Default)]
struct Resume {
    manual: Option<ManualSelection>,
    channel: Option<Channel>,
    lookup: Option<(String, u64)>,
    seen: Intent,
    user_id: Option<u64>,
    disputed_campaigns: Vec<Campaign>,
    avoided: HashMap<u64, Instant>,
}

pub struct Miner {
    app: Arc<App>,
    commands: mpsc::Receiver<CommandRequest>,
    endpoints: Endpoints,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum JobKind {
    Manual,
    Inventory,
    Channels,
    Watch,
    Playback,
    Poll,
    Claim,
    Update,
    Notification,
}
enum Job {
    Manual {
        revision: u64,
        requested_at: Instant,
        result: Box<Result<Option<Channel>, TwitchError>>,
    },
    Inventory {
        result: Result<Inventory, TwitchError>,
        requested_at: chrono::DateTime<Utc>,
        refresh_sequence: u64,
    },
    Channels {
        result: Result<Vec<Channel>, TwitchError>,
        requested_at: Instant,
    },
    Watch {
        channel: Box<Channel>,
        result: Result<bool, TwitchError>,
        requested_at: Instant,
        at: Instant,
    },
    Playback {
        state: Box<Playback>,
        requested_at: Instant,
        cancel: CancellationToken,
        result: Result<(), TwitchError>,
    },
    Poll {
        channel: u64,
        requested_at: Instant,
        result: Result<Option<(String, u32)>, TwitchError>,
    },
    Claim {
        id: String,
        announce: bool,
        attempted_at: chrono::DateTime<Utc>,
        result: Result<ClaimOutcome, TwitchError>,
    },
    Update {
        result: Result<Vec<Channel>, TwitchError>,
        requested_at: Instant,
    },
    Notification(Result<(), TwitchError>),
}
struct CompletedJob {
    epoch: u64,
    kind: JobKind,
    job: Job,
}

struct Mining {
    app: Arc<App>,
    client: TwitchClient,
    journal: Arc<Mutex<ClaimJournal>>,
    intent: watch::Receiver<Intent>,
    seen: Intent,
    events: mpsc::Receiver<Event>,
    campaigns: Vec<Campaign>,
    channels: Vec<Channel>,
    channels_loaded: bool,
    status: InventoryStatus,
    watching: Option<u64>,
    paused: bool,
    watch_started: Instant,
    manual: Option<ManualSelection>,
    lookup: Option<(String, u64)>,
    manual_pending: Option<String>,
    manual_error: Option<String>,
    jobs: JoinSet<CompletedJob>,
    busy: HashSet<JobKind>,
    watch_abort: Option<tokio::task::AbortHandle>,
    playback_cancel: Option<CancellationToken>,
    playback: Option<Playback>,
    next_playback: Instant,
    watch_failures: u8,
    epoch: u64,
    refresh: bool,
    channels_dirty: bool,
    publish: bool,
    next_refresh: Instant,
    next_watch: Instant,
    poll_at: Option<Instant>,
    last_progress: Option<(String, Instant)>,
    next_retry: Instant,
    refresh_channels: HashMap<u64, Instant>,
    channel_events: HashMap<u64, Instant>,
    beacon_events: HashMap<u64, Instant>,
    viewer_events: HashMap<u64, Instant>,
    notifications: HashSet<String>,
    claim_retry: HashMap<String, Instant>,
    claim_wait: Option<(String, Instant, u8)>,
    last_inventory: Instant,
    next_progress_refresh: Instant,
    next_transition: Option<chrono::DateTime<Utc>>,
    pending_claims: Vec<PendingClaim>,
    rejected_account_ids: HashSet<String>,
    // Streams whose watches kept failing; automatic selection prefers any other until then.
    avoided: HashMap<u64, Instant>,
    unclaimed_inventory: HashMap<String, chrono::DateTime<Utc>>,
}
impl Mining {
    fn restore(&mut self, saved: &Resume) {
        if saved.user_id == Some(self.client.user_id) {
            self.avoided = saved.avoided.clone();
            for campaign in &saved.disputed_campaigns {
                if !self.campaigns.iter().any(|c| c.id == campaign.id) {
                    self.campaigns.push(campaign.clone());
                }
            }
        }
        self.manual = saved.manual;
        self.seen = saved.seen.clone();
        self.lookup = saved.lookup.clone();
        self.manual_pending = saved.lookup.as_ref().map(|(login, _)| login.clone());
        if let Some(extra) = &saved.channel {
            // Keep identity, but require fresh stream
            // eligibility in this network generation before watching again.
            self.channels
                .push(Channel::offline(extra.identity.clone(), extra.acl_based));
            self.refresh_channels
                .insert(extra.identity.id, Instant::now());
        }
    }

    fn resume(&self) -> Resume {
        let manual = self.manual;
        Resume {
            user_id: Some(self.client.user_id),
            disputed_campaigns: self
                .campaigns
                .iter()
                .filter(|c| {
                    c.needs_progress_refresh(Utc::now())
                        && c.drops.iter().any(|d| d.progress_disputed)
                })
                .cloned()
                .collect(),
            manual,
            channel: manual.and_then(|manual| {
                self.channels
                    .iter()
                    .find(|c| c.identity.id == manual.channel)
                    .cloned()
            }),
            lookup: self
                .manual_pending
                .as_ref()
                .map(|login| (login.clone(), self.seen.manual_revision)),
            seen: self.seen.clone(),
            avoided: self.avoided.clone(),
        }
    }
    fn new(
        app: Arc<App>,
        client: TwitchClient,
        journal: Arc<Mutex<ClaimJournal>>,
        intent: watch::Receiver<Intent>,
        events: mpsc::Receiver<Event>,
    ) -> Self {
        let now = Instant::now();
        Self {
            app,
            client,
            journal,
            intent,
            seen: Intent::default(),
            events,
            campaigns: vec![],
            channels: vec![],
            channels_loaded: false,
            status: InventoryStatus::default(),
            watching: None,
            paused: false,
            watch_started: now,
            manual: None,
            lookup: None,
            manual_pending: None,
            manual_error: None,
            jobs: JoinSet::new(),
            busy: HashSet::new(),
            watch_abort: None,
            playback_cancel: None,
            playback: None,
            next_playback: now,
            watch_failures: 0,
            epoch: 0,
            refresh: true,
            channels_dirty: false,
            publish: false,
            next_refresh: now,
            next_watch: now,
            poll_at: None,
            last_progress: None,
            next_retry: now,
            refresh_channels: HashMap::new(),
            channel_events: HashMap::new(),
            beacon_events: HashMap::new(),
            viewer_events: HashMap::new(),
            notifications: HashSet::new(),
            claim_retry: HashMap::new(),
            claim_wait: None,
            last_inventory: now,
            next_progress_refresh: now,
            next_transition: None,
            pending_claims: vec![],
            rejected_account_ids: HashSet::new(),
            avoided: HashMap::new(),
            unclaimed_inventory: HashMap::new(),
        }
    }
    fn spawn(&mut self, kind: JobKind, future: impl Future<Output = Job> + Send + 'static) {
        let epoch = self.epoch;
        self.busy.insert(kind);
        let abort = self.jobs.spawn(async move {
            CompletedJob {
                epoch,
                kind,
                job: future.await,
            }
        });
        if kind == JobKind::Watch {
            self.watch_abort = Some(abort);
        }
    }

    async fn run(&mut self, pool: &mut PubSub) -> Result<(), TwitchError> {
        self.apply_intent(pool).await;
        pool.set_channels(
            &self
                .channels
                .iter()
                .map(|c| c.identity.id)
                .collect::<Vec<_>>(),
        );
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {biased;
                _=self.client.http.cancel.cancelled()=>return Err(TwitchError::Cancelled),
                changed=self.intent.changed()=>{if changed.is_err(){return Ok(());}self.apply_intent(pool).await;},
                event=self.events.recv()=>{if let Some(event)=event{self.event(event).await?;}},
                result=self.jobs.join_next(),if !self.jobs.is_empty()=>{
                    match result {
                        Some(Ok(completed))=>{
                            self.busy.remove(&completed.kind);
                            if completed.kind==JobKind::Watch{self.watch_abort=None;}
                            if completed.kind==JobKind::Playback{self.playback_cancel=None;}
                            if completed.epoch==self.epoch || matches!(completed.job,Job::Claim{..}){self.complete(completed.job,pool).await?;}
                        },
                        Some(Err(error))=>{
                            if !error.is_cancelled(){return Err(TwitchError::InvalidResponse);}
                            self.busy.remove(&JobKind::Watch);self.watch_abort=None;
                        },
                        _=>{},
                    }
                },
                _=tick.tick()=>{},
            }
            if self.next_transition.is_some_and(|at| Utc::now() >= at) {
                self.channels_dirty = true;
                self.publish = true;
                self.set_transition();
            }
            let settings = self.app.settings.read().await.clone();
            self.reselect(&settings).await;
            if self.publish {
                self.publish(&settings).await?;
                self.publish = false;
            }
            if Instant::now() >= self.next_retry {
                self.schedule(&settings).await;
            }
            self.schedule_playback(&settings);
        }
    }

    async fn apply_intent(&mut self, pool: &PubSub) {
        let intent = self.intent.borrow_and_update().clone();
        if intent.clear != self.seen.clear {
            self.cancel_watch();
            self.playback = None;
            self.next_playback = Instant::now();
            self.app.snapshot.write().await.current_drop = None;
            self.epoch = self.epoch.wrapping_add(1);
            self.campaigns.clear();
            self.rejected_account_ids.clear();
            self.unclaimed_inventory.clear();
            self.channels.clear();
            self.channels_loaded = false;
            self.watching = None;
            self.watch_started = Instant::now();
            self.last_progress = None;
            self.watch_failures = 0;
            self.manual = None;
            self.avoided.clear();
            self.lookup = None;
            self.manual_pending = None;
            self.manual_error = None;
            self.poll_at = None;
            self.claim_wait = None;
            self.refresh_channels.clear();
            self.channel_events.clear();
            self.beacon_events.clear();
            self.viewer_events.clear();
            self.status = InventoryStatus::default();
            pool.set_channels(&[]);
            self.publish = true;
        }
        if intent.refresh != self.seen.refresh {
            self.refresh |=
                intent.clear != self.seen.clear || !self.busy.contains(&JobKind::Inventory);
            self.next_retry = Instant::now();
        }
        if intent.settings != self.seen.settings {
            self.channels_dirty = true;
            self.publish = true;
            let minutes = self
                .app
                .settings
                .read()
                .await
                .minimum_refresh_interval_minutes;
            self.next_refresh = self.last_inventory + Duration::from_secs(u64::from(minutes) * 60);
        }
        let language = self.app.settings.read().await.language;
        self.apply_manual(&intent, language);
        let manual_revision = self.seen.manual_revision;
        self.seen = intent;
        self.seen.manual_revision = manual_revision;
    }

    async fn schedule(&mut self, settings: &Settings) {
        let now = Instant::now();
        let wall = Utc::now();
        // Recheck disputed progress and missing claims even without a watchable channel.
        if now >= self.next_progress_refresh
            && self
                .campaigns
                .iter()
                .any(|c| c.needs_progress_refresh(wall))
        {
            self.request_progress_refresh();
        }
        if !self.busy.contains(&JobKind::Manual)
            && let Some((login, revision)) = self.lookup.take()
        {
            let client = self.client.clone();
            self.spawn(JobKind::Manual, async move {
                let requested_at = Instant::now();
                let result =
                    tokio::time::timeout(Duration::from_secs(30), client.resolve_channel(&login))
                        .await
                        .unwrap_or(Err(TwitchError::Network));
                Job::Manual {
                    revision,
                    requested_at,
                    result: Box::new(result),
                }
            });
        }
        if ![JobKind::Claim, JobKind::Watch, JobKind::Poll]
            .iter()
            .any(|kind| self.busy.contains(kind))
        {
            let journal = self.journal.lock().await;
            let candidate = self
                .campaigns
                .iter()
                .filter(|c| settings.auto_claim_drops && !c.upcoming(wall))
                .flat_map(|c| c.drops.iter().map(move |d| (c, d)))
                .find(|(c, d)| {
                    d.can_claim(c.ends_at, wall)
                        && !journal.acknowledged(self.client.user_id, d, c.linked)
                        && self.claim_retry.get(&d.id).is_none_or(|at| now >= *at)
                })
                .map(|(c, d)| PendingClaim::new(self.client.user_id, c, d, settings));
            drop(journal);
            if let Some(pending) = candidate {
                let id = pending.entry.id.clone();
                let announce = !self.journal.lock().await.settled(self.client.user_id, &id);
                let attempted_at = pending.entry.claimed_at;
                let client = self.client.clone();
                let app = self.app.clone();
                let journal = self.journal.clone();
                self.spawn(JobKind::Claim, async move {
                    let result = claim(&app, &client, &journal, pending).await;
                    Job::Claim {
                        id,
                        announce,
                        attempted_at,
                        result,
                    }
                });
                return;
            }
            if let Some(pending) = self
                .pending_claims
                .iter()
                .find(|p| {
                    settings.auto_claim_drops
                        && wall < p.retry_until
                        && self
                            .claim_retry
                            .get(&p.entry.id)
                            .is_none_or(|at| now >= *at)
                })
                .cloned()
            {
                let client = self.client.clone();
                let app = self.app.clone();
                let journal = self.journal.clone();
                let id = pending.entry.id.clone();
                let announce = !self.journal.lock().await.settled(self.client.user_id, &id);
                let attempted_at = pending.entry.claimed_at;
                self.spawn(JobKind::Claim, async move {
                    let result = claim(&app, &client, &journal, pending).await;
                    Job::Claim {
                        id,
                        announce,
                        attempted_at,
                        result,
                    }
                });
                return;
            }
            if !settings.mining_paused
                && let Some(channel) = self.watching
            {
                if self
                    .claim_wait
                    .as_ref()
                    .is_some_and(|(_, due, _)| now >= *due)
                    || self.poll_at.is_some_and(|at| now >= at)
                {
                    self.poll_at = None;
                    if self.claim_wait.is_some()
                        || self.last_progress.as_ref().is_none_or(|(id, at)| {
                            now.duration_since(*at) >= WATCH_INTERVAL
                                || !self.progress_eligible(id, channel, settings)
                        })
                    {
                        let client = self.client.clone();
                        self.spawn(JobKind::Poll, async move {
                            Job::Poll {
                                channel,
                                requested_at: Instant::now(),
                                result: client.current_drop(channel).await,
                            }
                        });
                        return;
                    }
                }
                if self.claim_wait.is_none()
                    && now >= self.next_watch
                    && let Some(mut channel) = self.watch_channel(settings).cloned()
                {
                    self.next_watch = now + WATCH_INTERVAL;
                    let client = self.client.clone();
                    let requested_at = now;
                    self.spawn(JobKind::Watch, async move {
                        let result = client.send_watch(&mut channel, Utc::now()).await;
                        Job::Watch {
                            channel: Box::new(channel),
                            result,
                            requested_at,
                            at: Instant::now(),
                        }
                    });
                    return;
                }
            }
        }
        let manual_update = self
            .manual
            .map(|manual| manual.channel)
            .filter(|id| self.refresh_channels.get(id).is_some_and(|at| now >= *at));
        let updates: Vec<_> = self
            .channels
            .iter()
            .filter(|c| {
                manual_update.is_none_or(|id| c.identity.id == id)
                    && self
                        .refresh_channels
                        .get(&c.identity.id)
                        .is_some_and(|at| now >= *at)
            })
            .cloned()
            .collect();
        if !updates.is_empty() && !self.busy.contains(&JobKind::Update) {
            let client = self.client.clone();
            self.spawn(JobKind::Update, async move {
                let mut updates = updates;
                let requested_at = Instant::now();
                let result = if manual_update.is_some() {
                    client.update_manual_channel(&mut updates[0]).await
                } else {
                    client.update_channels(&mut updates).await
                }
                .map(|()| updates);
                Job::Update {
                    result,
                    requested_at,
                }
            });
            return;
        }
        if [
            JobKind::Inventory,
            JobKind::Channels,
            JobKind::Update,
            JobKind::Notification,
        ]
        .iter()
        .any(|kind| self.busy.contains(kind))
        {
            return;
        }
        if self.refresh || now >= self.next_refresh {
            self.refresh = false;
            self.next_progress_refresh = now + Duration::from_secs(60);
            let (refresh_sequence, _) = self.app.begin_inventory_refresh().await;
            let client = self.client.clone();
            self.app
                .status(message_in(
                    settings.language,
                    "gui.status.fetching_inventory",
                    &[],
                ))
                .await;
            self.spawn(JobKind::Inventory, async move {
                let requested_at = Utc::now();
                Job::Inventory {
                    result: client.inventory().await,
                    requested_at,
                    refresh_sequence,
                }
            });
            return;
        }
        if self.channels_dirty {
            self.channels_dirty = false;
            let client = self.client.clone();
            let campaigns = self.campaigns.clone();
            let settings = settings.clone();
            let current = self
                .channels
                .iter()
                .find(|c| {
                    Some(c.identity.id)
                        == self.watching.or(self.manual.map(|manual| manual.channel))
                })
                .cloned();
            self.app
                .status(message_in(settings.language, "gui.status.gathering", &[]))
                .await;
            self.spawn(JobKind::Channels, async move {
                Job::Channels {
                    requested_at: Instant::now(),
                    result: client
                        .channels(&campaigns, &settings, current.as_ref())
                        .await,
                }
            });
            return;
        }
        let auto_claim_drops = self.app.settings.read().await.auto_claim_drops;
        if auto_claim_drops && let Some(id) = self.notifications.iter().next().cloned() {
            self.notifications.remove(&id);
            let client = self.client.clone();
            self.spawn(JobKind::Notification, async move {
                Job::Notification(client.delete_notification(&id).await)
            });
        }
    }

    async fn complete(&mut self, job: Job, pool: &PubSub) -> Result<(), TwitchError> {
        use crate::app::activity::{ActivityEvent, Category, Severity};
        let settings = self.app.settings.read().await.clone();
        let now = Instant::now();
        let inventory_failed = matches!(&job, Job::Inventory { result: Err(_), .. });
        let notification = matches!(&job, Job::Notification(_));
        let playback = matches!(&job, Job::Playback { .. });
        let (operation, category, channel_id, drop_id, succeeded) = match &job {
            Job::Manual { result, .. } => ("manual", Category::Mining, None, None, result.is_ok()),
            Job::Inventory { result, .. } => (
                "inventory",
                Category::Inventory,
                None,
                None,
                result
                    .as_ref()
                    .is_ok_and(|inventory| inventory.status.available),
            ),
            Job::Channels { result, .. } => {
                ("channels", Category::Mining, None, None, result.is_ok())
            }
            Job::Watch {
                channel, result, ..
            } => (
                "watch",
                Category::Mining,
                Some(channel.identity.id),
                None,
                *result == Ok(true),
            ),
            Job::Playback { state, result, .. } => (
                "playback",
                Category::Mining,
                Some(state.channel),
                None,
                result.is_ok(),
            ),
            Job::Poll {
                channel, result, ..
            } => (
                "progress",
                Category::Mining,
                Some(*channel),
                None,
                result.is_ok(),
            ),
            Job::Claim { id, result, .. } => (
                "claim",
                Category::Claims,
                None,
                Some(id.clone()),
                *result == Ok(ClaimOutcome::Claimed),
            ),
            Job::Update { result, .. } => ("streams", Category::Mining, None, None, result.is_ok()),
            Job::Notification(result) => (
                "notification",
                Category::Account,
                None,
                None,
                result.is_ok(),
            ),
        };
        let mut activity = ActivityEvent::new_in(
            settings.language,
            "gui.backend.twitch_error",
            category,
            Severity::Warning,
            &[("operation", operation)],
        );
        activity.channel_id = channel_id;
        activity.drop_id = drop_id;
        let error = match job {
            Job::Playback {
                state,
                requested_at,
                cancel,
                result,
            } => {
                if cancel.is_cancelled() {
                    // Keep acknowledged segments across pause, never revive a cancelled URL.
                    if let Some(current) = &mut self.playback {
                        current.retain_checks(&state);
                    }
                    return Ok(());
                }
                if result == Err(TwitchError::Unauthorized) {
                    return Err(TwitchError::Unauthorized);
                }
                if requested_at < self.watch_started
                    || settings.mining_paused
                    || self.watching != Some(state.channel)
                    || !self.channels.iter().any(|c| state.matches(c))
                {
                    return Ok(());
                }
                self.playback = Some(*state);
                // Playback acknowledgements never confirm, estimate or schedule reward minutes.
                result.err()
            }
            Job::Manual {
                revision,
                requested_at,
                result,
            } => {
                let result = *result;
                if matches!(
                    result,
                    Err(TwitchError::Unauthorized | TwitchError::Cancelled)
                ) {
                    return Err(result.err().unwrap());
                }
                if self.client.http.cancel.is_cancelled() {
                    return Err(TwitchError::Cancelled);
                }
                let receiver = self.intent.clone();
                let intent = receiver.borrow();
                if intent.manual_revision == revision {
                    let error = self.finish_manual(result, requested_at, intent.manual_duration);
                    self.lookup = None;
                    self.manual_pending = None;
                    self.manual_error = error.map(|key| message_in(settings.language, key, &[]));
                    self.publish = true;
                    pool.set_channels(
                        &self
                            .channels
                            .iter()
                            .map(|c| c.identity.id)
                            .collect::<Vec<_>>(),
                    );
                }
                None
            }
            Job::Inventory {
                result: Ok(mut inventory),
                requested_at,
                refresh_sequence,
            } => {
                for campaign in &mut inventory.campaigns {
                    for drop in &mut campaign.drops {
                        // Only fresh account records can contradict an acknowledged claim.
                        if drop.confirmed_at.is_some() {
                            if !drop.claimed {
                                self.unclaimed_inventory
                                    .entry(drop.id.clone())
                                    .and_modify(|at| *at = (*at).max(requested_at))
                                    .or_insert(requested_at);
                            } else if self
                                .unclaimed_inventory
                                .get(&drop.id)
                                .is_some_and(|at| *at <= requested_at)
                            {
                                self.unclaimed_inventory.remove(&drop.id);
                            }
                        }
                        if let Some(previous) = self
                            .campaigns
                            .iter()
                            .find(|c| c.id == campaign.id)
                            .and_then(|c| c.drops.iter().find(|d| d.id == drop.id))
                        {
                            drop.reconcile_inventory(previous, requested_at);
                        }
                    }
                }
                if !inventory.status.available {
                    for previous in &self.campaigns {
                        if (previous.active(Utc::now())
                            || previous.upcoming(Utc::now())
                            || previous.needs_progress_refresh(Utc::now()))
                            && !inventory.campaigns.iter().any(|c| c.id == previous.id)
                        {
                            // Retained records are not fresh inventory confirmations.
                            inventory.campaigns.push(previous.clone());
                        }
                    }
                }
                for campaign in &mut inventory.campaigns {
                    for drop in &mut campaign.drops {
                        // A completed refresh must release the estimate ceiling, including
                        // retained records, without overwriting newer account evidence.
                        if drop.estimated_minutes >= MAX_ESTIMATED_MINUTES
                            && drop.confirmed_at.is_none_or(|at| at <= requested_at)
                        {
                            drop.estimated_minutes = 0;
                        }
                    }
                }
                self.unclaimed_inventory.retain(|id, _| {
                    inventory
                        .campaigns
                        .iter()
                        .any(|c| c.drops.iter().any(|d| &d.id == id))
                });
                self.campaigns = inventory.campaigns;
                self.status = inventory.status;
                self.rejected_account_ids = inventory.rejected_account_ids;
                self.recover_claims(&inventory.awards).await?;
                let observed_at = Utc::now();
                let entries = self
                    .campaigns
                    .iter()
                    .flat_map(|campaign| {
                        campaign
                            .drops
                            .iter()
                            .filter(|drop| drop.claimed)
                            .map(move |drop| {
                                let mut entry = campaign
                                    .history_entry(drop, drop.claimed_at.unwrap_or(observed_at));
                                entry.claimed_at_is_observed = drop.claimed_at.is_none();
                                entry
                            })
                    })
                    .collect();
                let app = self.app.clone();
                tokio::task::spawn_blocking(move || app.import_history(entries))
                    .await
                    .map_err(|_| TwitchError::Storage)?
                    .map_err(|_| TwitchError::Storage)?;
                self.last_inventory = now;
                self.set_transition();
                self.next_refresh = now
                    + Duration::from_secs(
                        u64::from(settings.minimum_refresh_interval_minutes) * 60,
                    );
                self.channels_dirty = true;
                self.publish_snapshot(
                    &settings,
                    Some((
                        refresh_sequence,
                        (!self.status.available).then(|| {
                            message_in(settings.language, "gui.redesign.campaigns_unavailable", &[])
                        }),
                    )),
                )
                .await?;
                self.publish = false;
                None
            }
            Job::Channels {
                result: Ok(mut channels),
                requested_at,
            } => {
                self.preserve_channel_events(&mut channels, requested_at);
                if let Some(ManualSelection { channel: id, .. }) = self.manual
                    && !channels.iter().any(|c| c.identity.id == id)
                    && let Some(current) = self.channels.iter().find(|c| c.identity.id == id)
                {
                    channels.insert(0, current.clone());
                    channels.truncate(crate::twitch::channels::MAX_CHANNELS);
                }
                self.channels = channels;
                self.channels_loaded = true;
                let intent = self.intent.borrow().clone();
                self.apply_manual(&intent, settings.language);
                self.channel_events
                    .retain(|id, _| self.channels.iter().any(|c| c.identity.id == *id));
                self.beacon_events
                    .retain(|id, _| self.channels.iter().any(|c| c.identity.id == *id));
                self.viewer_events
                    .retain(|id, _| self.channels.iter().any(|c| c.identity.id == *id));
                self.refresh_channels
                    .retain(|id, _| self.channels.iter().any(|c| c.identity.id == *id));
                pool.set_channels(
                    &self
                        .channels
                        .iter()
                        .map(|c| c.identity.id)
                        .collect::<Vec<_>>(),
                );
                self.publish = true;
                if self.watching.is_none() {
                    self.idle_status(&settings).await;
                }
                None
            }
            Job::Watch {
                channel,
                result,
                requested_at,
                at,
            } => {
                if matches!(
                    result,
                    Err(TwitchError::Unauthorized | TwitchError::Cancelled)
                ) {
                    return Err(result.err().unwrap());
                }
                let current = self.channels.iter_mut().find(|c| {
                    !settings.mining_paused
                        && self.watching == Some(c.identity.id)
                        && c.identity.id == channel.identity.id
                        && c.broadcast_id == channel.broadcast_id
                        && self
                            .beacon_events
                            .get(&c.identity.id)
                            .is_none_or(|at| *at <= requested_at)
                });
                let Some(current) = current else {
                    return Ok(());
                };
                // Persist failure invalidation too, and prevent an older stream
                // refresh from restoring the stale beacon cached in its clone.
                if current.beacon_url != channel.beacon_url {
                    current.beacon_url = channel.beacon_url;
                    self.beacon_events.insert(current.identity.id, at);
                }
                if result == Ok(true) {
                    self.watch_failures = 0;
                    self.next_watch = at + WATCH_INTERVAL;
                    self.poll_at = Some(at + PROGRESS_DELAY);
                    None
                } else {
                    self.refresh_channels.insert(channel.identity.id, now);
                    self.watch_failures += 1;
                    let error = result.err().unwrap_or(TwitchError::Network);
                    if self.watch_failures >= 3 {
                        tracing::warn!(
                            failures = self.watch_failures,
                            "Repeated watch failures; renewing Twitch connections"
                        );
                        if self.manual.is_none() {
                            self.avoided
                                .insert(channel.identity.id, now + FAILED_CHANNEL_COOLDOWN);
                        }
                        return Err(error);
                    }
                    Some(error)
                }
            }
            Job::Poll {
                channel,
                result,
                requested_at,
            } => {
                if !settings.mining_paused
                    && self.watching == Some(channel)
                    && requested_at >= self.watch_started
                    && requested_at >= self.last_inventory
                    && self
                        .channel_events
                        .get(&channel)
                        .is_none_or(|at| *at <= requested_at)
                {
                    let current = result.as_ref().ok().and_then(|v| v.as_ref());
                    let newer_progress = self
                        .last_progress
                        .as_ref()
                        .is_some_and(|(_, at)| *at > requested_at);
                    let confirmed = newer_progress
                        || current.is_some_and(|(id, minutes)| {
                            let accepted = self.confirm(id, *minutes, &settings);
                            if accepted && self.reported_drop(id, channel, &settings).is_some() {
                                self.last_progress = Some((id.clone(), now));
                            }
                            accepted && self.progress_eligible(id, channel, &settings)
                        });
                    if let Some((claimed, _, attempts)) = self.claim_wait.take() {
                        if current.is_some_and(|(id, _)| id == &claimed) && attempts < 7 {
                            self.claim_wait =
                                Some((claimed, now + Duration::from_secs(2), attempts + 1));
                        } else {
                            self.next_watch = now;
                        }
                    } else if !confirmed && self.manual.is_none() {
                        let watching = self.channels.iter().find(|c| c.identity.id == channel);
                        for campaign in &mut self.campaigns {
                            if watching
                                .is_some_and(|c| campaign.can_watch(c, &settings, Utc::now()))
                                && campaign.bump_estimates(&settings, Utc::now())
                            {
                                self.refresh = true;
                                self.channels_dirty = true;
                            }
                        }
                        self.publish = true;
                    }
                }
                result.err()
            }
            Job::Claim {
                id,
                result: Ok(ClaimOutcome::Claimed),
                announce,
                attempted_at,
            } => {
                self.recover_claims(&HashMap::new()).await?;
                let campaign_id = self
                    .campaigns
                    .iter()
                    .find(|campaign| campaign.drops.iter().any(|drop| drop.id == id))
                    .map(|campaign| campaign.id.clone());
                if let Some(drop) = self
                    .campaigns
                    .iter_mut()
                    .flat_map(|c| &mut c.drops)
                    .find(|d| d.id == id)
                {
                    if self
                        .unclaimed_inventory
                        .get(&id)
                        .is_none_or(|at| *at <= attempted_at)
                    {
                        drop.mark_claimed(attempted_at);
                    }
                    let mut event = crate::app::activity::ActivityEvent::new(
                        "status.claimed_drop",
                        crate::app::activity::Category::Claims,
                        crate::app::activity::Severity::Info,
                        &[("drop", &drop.name)],
                    );
                    event.drop_id = Some(drop.id.clone());
                    event.campaign_id = campaign_id;
                    if announce {
                        self.app.record_activity(event).await;
                    }
                }
                if let Some(campaign) = self
                    .campaigns
                    .iter()
                    .find(|c| c.drops.iter().any(|d| d.id == id))
                    && let Some(drop) = campaign.drops.iter().find(|d| d.id == id)
                    && announce
                {
                    self.app
                        .notify_claimed(settings.language, self.client.user_id, campaign, drop);
                }
                self.claim_retry.remove(&id);
                self.claim_wait = Some((id, now + Duration::from_secs(4), 0));
                self.publish = true;
                None
            }
            Job::Claim {
                result: Ok(ClaimOutcome::Disabled),
                ..
            } => None,
            Job::Claim { id, result, .. } => {
                self.claim_retry.insert(id, now + Duration::from_secs(60));
                result.err()
            }
            Job::Update {
                result: Ok(mut updated),
                requested_at,
            } => {
                self.preserve_channel_events(&mut updated, requested_at);
                for channel in updated {
                    if self
                        .refresh_channels
                        .get(&channel.identity.id)
                        .is_some_and(|due| *due <= requested_at)
                    {
                        self.refresh_channels.remove(&channel.identity.id);
                    }
                    if let Some(current) = self
                        .channels
                        .iter_mut()
                        .find(|c| c.identity.id == channel.identity.id)
                    {
                        *current = channel;
                        self.channel_events
                            .entry(current.identity.id)
                            .and_modify(|at| *at = (*at).max(requested_at))
                            .or_insert(requested_at);
                    }
                }
                self.publish = true;
                None
            }
            Job::Notification(result) => result.err(),
            Job::Inventory {
                result: Err(error),
                refresh_sequence,
                ..
            } => {
                self.app
                    .finish_inventory_refresh(
                        refresh_sequence,
                        Some(message_in(
                            settings.language,
                            "gui.redesign.refresh_failed_detail",
                            &[],
                        )),
                    )
                    .await;
                self.refresh = false;
                self.next_refresh = now + Duration::from_secs(60);
                self.next_progress_refresh = self.next_refresh;
                Some(error)
            }
            Job::Channels {
                result: Err(error), ..
            } => {
                self.channels_dirty = true;
                Some(error)
            }
            Job::Update {
                result: Err(error), ..
            } => Some(error),
        };
        if let Some(error) = error {
            if matches!(error, TwitchError::Unauthorized | TwitchError::Cancelled) {
                return Err(error);
            }
            if !inventory_failed && !notification && !playback {
                self.next_retry = now + Duration::from_secs(10);
            }
            activity.message = message_in(
                settings.language,
                "gui.backend.twitch_error",
                &[("error", &error.to_string())],
            );
            activity.args.insert("error".into(), error.to_string());
            self.app.record_activity(activity).await;
        } else if succeeded {
            self.app.recover_activity(&activity).await;
        }
        Ok(())
    }
}
