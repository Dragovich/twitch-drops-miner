use super::*;

const RESTART_DELAY: Duration = Duration::from_secs(5);
const STABLE_GENERATION: Duration = Duration::from_secs(60);

// A generation that keeps failing soon after it starts is replaced with a growing
// delay, so a persistent fault cannot repeat authenticated requests every few seconds.
#[derive(Default)]
pub(super) struct Restarts {
    failures: u32,
}
impl Restarts {
    pub(super) fn reset(&mut self) {
        self.failures = 0;
    }
    pub(super) fn delay(&mut self, lived: Duration) -> Duration {
        if lived >= STABLE_GENERATION {
            self.failures = 0;
        }
        let delay = RESTART_DELAY * (1 << self.failures.min(6));
        self.failures = self.failures.saturating_add(1);
        delay
    }
}

impl Miner {
    pub fn new(app: Arc<App>, commands: mpsc::Receiver<CommandRequest>) -> Self {
        Self {
            app,
            commands,
            endpoints: Endpoints::default(),
        }
    }

    fn start(
        &self,
        settings: Settings,
        resume: Arc<Mutex<Resume>>,
        receiver: watch::Receiver<Intent>,
        delay: Duration,
    ) -> Generation {
        let cancel = CancellationToken::new();
        let confirmed = Arc::new(Notify::new());
        let retry = Arc::new(Notify::new());
        let woken = retry.clone();
        let app = self.app.clone();
        let endpoints = self.endpoints.clone();
        let stopped = cancel.clone();
        let confirmation = confirmed.clone();
        let task = tokio::spawn(async move {
            // The delay belongs to the replacement, so its owner keeps serving commands.
            tokio::select! {biased;
                _=stopped.cancelled()=>return Err(TwitchError::Cancelled),
                _=woken.notified()=>{},
                _=tokio::time::sleep(delay)=>{},
            }
            run_generation(
                app,
                settings,
                endpoints,
                stopped,
                confirmation,
                receiver,
                resume,
            )
            .await
        });
        Generation {
            cancel,
            confirmed,
            retry,
            task,
        }
    }

    pub async fn run(mut self) -> Result<(), TwitchError> {
        let resume = Arc::new(Mutex::new(Resume::default()));
        // Commands outlive a network generation, including commands accepted
        // after its last select cycle but before the supervisor observes its exit.
        let (intent, _) = watch::channel(Intent::default());
        let mut restarts = Restarts::default();
        let mut delay = Duration::ZERO;
        while !self.app.shutdown.is_cancelled() {
            let settings = self.app.settings.read().await.clone();
            let mut generation =
                self.start(settings.clone(), resume.clone(), intent.subscribe(), delay);
            let mut started = Instant::now() + std::mem::take(&mut delay);
            loop {
                tokio::select! {biased;
                    _=self.app.shutdown.cancelled()=>{
                        while let Ok(request)=self.commands.try_recv(){
                            if matches!(request.command,Command::Logout){self.logout(generation,request.complete).await;return Ok(());}
                            let _=request.complete.send(Err("Miner is shutting down".into()));
                        }
                        generation.cancel.cancel();let _=generation.task.await;return Ok(());
                    },
                    request=self.commands.recv()=>{
                        let Some(request)=request else {generation.cancel.cancel();let _=generation.task.await;return Ok(())};
                        match request.command {
                            Command::Logout=>{self.logout(generation,request.complete).await;*resume.lock().await=Resume::default();intent.send_replace(Intent::default());restarts.reset();break;},
                            Command::Shutdown=>{
                                self.app.shutdown.cancel();
                                let _=request.complete.send(Ok(()));
                            },
                            Command::ConfirmOAuth=>{generation.confirmed.notify_one();let _=request.complete.send(Ok(()));},
                            Command::SettingsChanged=>{
                                let current=self.app.settings.read().await.clone();
                                if current.proxy!=settings.proxy || current.connection_quality!=settings.connection_quality {
                                    generation.cancel.cancel();let _=generation.task.await;
                                    restarts.reset();
                                    let _=request.complete.send(Ok(()));break;
                                }
                                intent.send_modify(|intent|intent.settings=intent.settings.wrapping_add(1));
                                let _=request.complete.send(Ok(()));
                            },
                            command=>{
                                // An explicit refresh retries a generation still waiting to start
                                // and restarts the delay; its lifetime counts from now.
                                if matches!(command,Command::Refresh{..}) && Instant::now()<started {
                                    started=Instant::now();restarts.reset();generation.retry.notify_one();
                                }
                                intent.send_modify(|intent|match command {
                                    Command::Refresh{clear_cache}=>{intent.refresh=intent.refresh.wrapping_add(1);if clear_cache{intent.clear=intent.clear.wrapping_add(1);}},
                                    Command::SelectChannel(id,duration)=>{intent.channel_login=None;intent.selected=Some(id);intent.manual_duration=duration;intent.manual_revision=intent.manual_revision.wrapping_add(1);},
                                    Command::MineChannel(login,duration)=>{intent.channel_login=Some(login);intent.selected=None;intent.manual_duration=duration;intent.manual_revision=intent.manual_revision.wrapping_add(1);},
                                    Command::ExitManual=>{intent.channel_login=None;intent.selected=None;intent.manual_revision=intent.manual_revision.wrapping_add(1);},
                                    _=>{},
                                });
                                let _=request.complete.send(Ok(()));
                            },
                        }
                    },
                    result=&mut generation.task=>{
                        match result {
                            Ok(Err(TwitchError::Unauthorized))=>{
                                *resume.lock().await=Resume::default();
                                intent.send_replace(Intent::default());
                                if remove_session(&self.app).await.is_err(){self.app.activity("gui.backend.session_storage",&[]).await;}
                                reset_session(&self.app).await;
                                restarts.reset();
                            },
                            Ok(Err(TwitchError::Cancelled))|Ok(Ok(()))=>restarts.reset(),
                            Ok(Err(error))=>{
                                let sequence=self.app.snapshot.read().await.inventory_refresh.sequence;
                                self.app.finish_inventory_refresh(sequence,Some(self.app.text("gui.redesign.refresh_failed_detail",&[]).await)).await;
                                self.app.activity("gui.backend.twitch_error",&[("error",&error.to_string())]).await;
                                delay=restarts.delay(Instant::now().saturating_duration_since(started));
                            },
                            Err(_)=>{
                                self.app.activity("gui.backend.worker_failed",&[]).await;
                                self.app.shutdown.cancel();return Err(TwitchError::InvalidResponse);
                            },
                        }
                        break;
                    },
                }
            }
        }
        Ok(())
    }

    pub(super) async fn logout(
        &mut self,
        mut generation: Generation,
        first: oneshot::Sender<Result<(), String>>,
    ) {
        generation.cancel.cancel();
        let mut requests = vec![first];
        // The owner keeps draining even if the requesting browser disconnects or
        // process shutdown starts. No new login can overlap credential removal.
        loop {
            tokio::select! {
                _=&mut generation.task=>break,
                request=self.commands.recv()=>{
                    let Some(request)=request else {let _=generation.task.await;break};
                    self.during_logout(request,&mut requests);
                },
            }
        }
        while let Ok(request) = self.commands.try_recv() {
            self.during_logout(request, &mut requests);
        }
        let result = remove_session(&self.app)
            .await
            .map_err(|error| error.to_string());
        reset_session(&self.app).await;
        for request in requests {
            let _ = request.send(result.clone());
        }
    }
    fn during_logout(
        &self,
        request: CommandRequest,
        requests: &mut Vec<oneshot::Sender<Result<(), String>>>,
    ) {
        match request.command {
            Command::Logout => requests.push(request.complete),
            Command::Shutdown => {
                self.app.shutdown.cancel();
                requests.push(request.complete);
            }
            Command::SelectChannel(..) | Command::MineChannel(..) => {
                let _ = request
                    .complete
                    .send(Err("Twitch login is required".into()));
            }
            _ => {
                let _ = request.complete.send(Ok(()));
            }
        }
    }
}

pub(super) async fn remove_session(app: &Arc<App>) -> Result<(), TwitchError> {
    let directory = app.data.path.clone();
    tokio::task::spawn_blocking(move || Session::remove(&directory))
        .await
        .map_err(|_| TwitchError::Storage)?
}
pub(super) async fn publish_login(app: &App, mut login: Login) {
    let mut state = app.snapshot.write().await;
    if login.user_id.is_some() && login.user_id == state.login.user_id && login.profile.is_none() {
        login.profile = state.login.profile.clone();
    }
    if login.user_id.is_some() && state.login.user_id.is_none() {
        state.mining.state = crate::dto::MiningState::Discovering;
    }
    if login.user_id.is_some() {
        use crate::app::activity::{self, ActivityEvent, Category, Severity};
        let mut recovery = ActivityEvent::new_in(
            app.settings.read().await.language,
            "gui.backend.twitch_error",
            Category::Connection,
            Severity::Info,
            &[],
        );
        activity::recover(&mut state, &recovery);
        recovery
            .args
            .insert("operation".into(), "validation".into());
        activity::recover(&mut state, &recovery);
    }
    state.login = login;
}
pub(super) async fn reset_session(app: &App) {
    app.telegram.cancel_pending();
    let archived = app.archive.lock().await.merge(vec![], Utc::now());
    {
        let mut state = app.snapshot.write().await;
        state.channels.clear();
        state.current_drop = None;
        state.wanted_items.clear();
        state.manual_mode = ManualMode::default();
        state.mining = crate::dto::MiningStatus {
            state: crate::dto::MiningState::AccountRequired,
            ..Default::default()
        };
        state.login = Login {
            status: app.text("login.status.logged_out", &[]).await,
            ..Login::default()
        };
        state.campaigns = archived;
        state.inventory_status = InventoryStatus::default();
        state.inventory_refresh.sequence += 1;
        state.inventory_refresh.state = RefreshState::Idle;
        state.inventory_refresh.error = None;
        state.settings.games_available.clear();
    }
}

pub(super) async fn authenticate(
    app: &Arc<App>,
    settings: &Settings,
    endpoints: &Endpoints,
    cancel: &CancellationToken,
    confirmed: &Notify,
) -> Result<(TwitchClient, Session), TwitchError> {
    let mut storage = Restarts::default();
    loop {
        let attempt = async {
            let saved = Session::load(&app.data.path)?;
            let mut http = TwitchHttp::build(
                settings,
                saved.as_ref().map(|s| s.device_id.as_str()),
                cancel.clone(),
                endpoints.clone(),
            )?;
            let session = if let Some(saved) = saved {
                saved.restore(&http).await?
            } else {
                http.discover_device().await?;
                let http = Arc::new(http);
                let login = DeviceLogin::start(http.clone()).await?;
                publish_login(
                    app,
                    Login {
                        status: message_in(settings.language, "login.status.waiting_auth", &[]),
                        user_id: None,
                        oauth_pending: Some(login.code.clone()),
                        ..Login::default()
                    },
                )
                .await;
                app.status(app.text("login.status.required", &[]).await)
                    .await;
                let session = login.finish(confirmed).await?;
                save_session(app, &session).await?;
                return Ok((TwitchClient::new(http, &session), session));
            };
            save_session(app, &session).await?;
            Ok((TwitchClient::new(Arc::new(http), &session), session))
        }
        .await;
        let delay = match attempt {
            Ok(result) => return Ok(result),
            Err(TwitchError::Cancelled) => return Err(TwitchError::Cancelled),
            Err(TwitchError::Unauthorized) => {
                remove_session(app).await?;
                reset_session(app).await;
                RESTART_DELAY
            }
            Err(error) => {
                publish_login(
                    app,
                    Login {
                        status: message_in(settings.language, "login.status.required", &[]),
                        ..Login::default()
                    },
                )
                .await;
                app.activity("gui.backend.twitch_error", &[("error", &error.to_string())])
                    .await;
                // Typically a session that Twitch just validated but that cannot be saved.
                if error == TwitchError::Storage {
                    storage.delay(Duration::ZERO)
                } else {
                    RESTART_DELAY
                }
            }
        };
        tokio::select! {biased;_=cancel.cancelled()=>return Err(TwitchError::Cancelled),_=tokio::time::sleep(delay)=>{}}
    }
}
pub(super) async fn save_session(app: &Arc<App>, session: &Session) -> Result<(), TwitchError> {
    let directory = app.data.path.clone();
    let saved = session.clone();
    tokio::task::spawn_blocking(move || saved.save(&directory))
        .await
        .map_err(|_| TwitchError::Storage)?
}

pub(super) async fn run_generation(
    app: Arc<App>,
    settings: Settings,
    endpoints: Endpoints,
    cancel: CancellationToken,
    confirmed: Arc<Notify>,
    intent: watch::Receiver<Intent>,
    resume: Arc<Mutex<Resume>>,
) -> Result<(), TwitchError> {
    let (client, session) = authenticate(&app, &settings, &endpoints, &cancel, &confirmed).await?;
    if cancel.is_cancelled() {
        return Err(TwitchError::Cancelled);
    }
    publish_login(
        &app,
        Login {
            status: message_in(settings.language, "login.status.logged_in", &[]),
            user_id: Some(session.user_id),
            oauth_pending: None,
            ..Login::default()
        },
    )
    .await;
    let journal = Arc::new(Mutex::new(
        ClaimJournal::load(&app.data.path).map_err(|_| TwitchError::Storage)?,
    ));
    let (events, receiver) = mpsc::channel(256);
    let mut pool = PubSub::start(client.clone(), events);
    let profile_due = Notify::new();
    let profile = async {
        loop {
            refresh_profile(app.clone(), client.clone()).await?;
            profile_due.notified().await;
        }
    };
    let validation = validate_session(&app, &session, &client.http, &profile_due);
    let (queries, requests) = mpsc::channel(4);
    *app.directory_queries.write().await = Some(queries);
    let directory = read_directory(client.clone(), requests);
    let mut mining = Mining::new(app.clone(), client.clone(), journal, intent, receiver);
    {
        let saved = resume.lock().await;
        mining.restore(&saved);
    }
    let result = {
        let run = mining.run(&mut pool);
        tokio::pin!(run);
        let reads = async {
            tokio::select! { biased;
                _ = cancel.cancelled() => Err(TwitchError::Cancelled),
                result = validation => result,
                result = profile => result,
                result = directory => result,
            }
        };
        tokio::select! {
            biased;
            result = &mut run => result,
            result = reads => {
                cancel.cancel();
                // Mining may be awaiting durable writes outside its job set.
                match run.await {
                    Err(error) if error != TwitchError::Cancelled => Err(error),
                    _ => result,
                }
            },
        }
    };
    *resume.lock().await = mining.resume();
    cancel.cancel();
    *app.directory_queries.write().await = None;
    // Owned jobs include durable claim writes. Cancellation stops network work,
    // while any confirmed claim finishes its disk transaction before logout.
    while mining.jobs.join_next().await.is_some() {}
    pool.close().await;
    result
}

async fn validate_session(
    app: &App,
    session: &Session,
    http: &TwitchHttp,
    profile_due: &Notify,
) -> Result<(), TwitchError> {
    use crate::app::activity::{ActivityEvent, Category, Severity};

    let mut delay = Duration::from_secs(3600);
    loop {
        tokio::select! { biased;
            _ = http.cancel.cancelled() => return Err(TwitchError::Cancelled),
            _ = tokio::time::sleep(delay) => {},
        }
        match session.validate(http).await {
            Ok(Some(_)) => {
                profile_due.notify_one();
                app.recover_activity(&ActivityEvent::new(
                    "gui.backend.twitch_error",
                    Category::Connection,
                    Severity::Warning,
                    &[("operation", "validation")],
                ))
                .await;
                delay = Duration::from_secs(3600);
            }
            // Drain mining before the existing authentication path renews credentials.
            Ok(None) => return Ok(()),
            Err(error @ (TwitchError::Unauthorized | TwitchError::Cancelled)) => return Err(error),
            Err(error) => {
                app.record_activity(ActivityEvent::new(
                    "gui.backend.twitch_error",
                    Category::Connection,
                    Severity::Warning,
                    &[("operation", "validation"), ("error", &error.to_string())],
                ))
                .await;
                delay = Duration::from_secs(60);
            }
        }
    }
}

pub(super) async fn read_directory(
    client: TwitchClient,
    mut requests: mpsc::Receiver<crate::app::commands::DirectoryRequest>,
) -> Result<(), TwitchError> {
    loop {
        let request = tokio::select! { biased;
            _ = client.http.cancel.cancelled() => return Err(TwitchError::Cancelled),
            request = requests.recv() => request.ok_or(TwitchError::Cancelled)?,
        };
        use crate::app::commands::DirectoryRequest;
        let fatal = match request {
            DirectoryRequest::Games {
                query,
                mut complete,
            } => {
                let result = tokio::select! { biased;
                    _ = complete.closed() => continue,
                    result = client.games(&query) => result,
                };
                let fatal = result.as_ref().err().copied();
                let _ = complete.send(result);
                fatal
            }
            DirectoryRequest::Channels {
                query,
                mut complete,
            } => {
                let result = tokio::select! { biased;
                    _ = complete.closed() => continue,
                    result = client.search_channels(&query) => result,
                };
                let fatal = result.as_ref().err().copied();
                let _ = complete.send(result);
                fatal
            }
        }
        .filter(|error| matches!(error, TwitchError::Unauthorized | TwitchError::Cancelled));
        if let Some(error) = fatal {
            return Err(error);
        }
    }
}

pub(super) async fn refresh_profile(
    app: Arc<App>,
    client: TwitchClient,
) -> Result<(), TwitchError> {
    if let Some(profile) = client.account_profile().await? {
        let mut state = app.snapshot.write().await;
        if !client.http.cancel.is_cancelled() && state.login.user_id == Some(client.user_id) {
            state.login.profile = Some(profile);
        }
    }
    Ok(())
}
