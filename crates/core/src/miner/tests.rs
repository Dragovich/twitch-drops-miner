use super::session::{authenticate, reset_session};
use super::*;
use crate::{
    domain::{ChannelIdentity, Game, MAX_ESTIMATED_MINUTES},
    store::{CampaignArchive, History, HistoryFilter},
    twitch::tests::{campaign_json, gql_mock, http, session, validation},
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn miner(server: &MockServer) -> (tempfile::TempDir, Mining, watch::Sender<Intent>, PubSub) {
    let dir = tempfile::tempdir().unwrap();
    let (app, _commands) = App::open(dir.path().to_owned()).unwrap();
    let client = TwitchClient::new(Arc::new(http(server)), &session());
    let (intent, receiver) = watch::channel(Intent::default());
    let (events, reader) = mpsc::channel(256);
    let journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
    let pool = PubSub::start(client.clone(), events);
    let mut mining = Mining::new(app, client, journal, receiver, reader);
    mining.refresh = false;
    mining.next_refresh = Instant::now() + Duration::from_secs(1800);
    mining.campaigns =
        vec![Campaign::parse(&campaign_json("one"), &HashMap::new(), Utc::now()).unwrap()];
    mining.channels = vec![Channel {
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
        broadcast_id: Some("stream1".into()),
        viewers: Some(100),
        drops_enabled: true,
        acl_based: false,
        beacon_url: None,
    }];
    mining.channels_loaded = true;
    (dir, mining, intent, pool)
}
async fn select(mining: &mut Mining) -> Settings {
    let settings = Settings {
        games_to_watch: vec!["Rust".into()],
        ..Settings::default()
    };
    *mining.app.settings.write().await = settings.clone();
    mining.reselect(&settings).await;
    settings
}
async fn finish_job(mining: &mut Mining, pool: &PubSub) {
    let completed = tokio::time::timeout(Duration::from_secs(5), mining.jobs.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    mining.busy.remove(&completed.kind);
    mining.complete(completed.job, pool).await.unwrap();
}

#[tokio::test]
async fn acknowledged_telemetry_also_checks_new_stream_segments() {
    let server = MockServer::start().await;
    gql_mock(&server, |_| json!({"data":{"streamPlaybackAccessToken":{"value":"mock-token","signature":"mock-signature"}}})).await;
    for (method_name, url, body) in [
        ("POST", "/track", ""),
        (
            "GET",
            "/api/channel/hls/streamer.m3u8",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n/media.m3u8\n",
        ),
        (
            "GET",
            "/media.m3u8",
            "#EXTM3U\n#EXTINF:2,\n/one.ts\n#EXTINF:2,\n/two.ts\n",
        ),
        ("HEAD", "/one.ts", ""),
        ("HEAD", "/two.ts", ""),
    ] {
        Mock::given(method(method_name))
            .and(path(url))
            .respond_with(
                ResponseTemplate::new(if method_name == "POST" { 204 } else { 200 })
                    .insert_header("Content-Type", "application/vnd.apple.mpegurl")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
    }
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.refresh = false;
    miner.channels_dirty = false;
    miner.next_refresh = Instant::now() + Duration::from_secs(3600);
    miner.channels[0].beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
    let minutes = miner.campaigns[0].drops[0].confirmed_minutes;
    for _ in 0..3 {
        miner.schedule(&settings).await;
        miner.schedule_playback(&settings);
        while !miner.jobs.is_empty() {
            finish_job(&mut miner, &pool).await;
        }
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|r| r.method == "HEAD").count(), 2);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, minutes);
    pool.close().await;
}

#[tokio::test]
async fn playback_runs_independently_without_accelerating_telemetry_or_progress() {
    let server = MockServer::start().await;
    gql_mock(&server, |_| json!({"data":{"streamPlaybackAccessToken":{"value":"mock-token","signature":"mock-signature"}}})).await;
    for url in ["/api/channel/hls/streamer.m3u8", "/media.m3u8"] {
        let body = if url == "/media.m3u8" {
            "#EXTM3U\n#EXTINF:2,\n/one.ts\n"
        } else {
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n/media.m3u8\n"
        };
        Mock::given(method("GET"))
            .and(path(url))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("Content-Type", "application/vnd.apple.mpegurl"),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.next_watch = Instant::now() + WATCH_INTERVAL;
    miner.poll_at = Some(Instant::now() + PROGRESS_DELAY);
    let watch_due = miner.next_watch;
    let poll_due = miner.poll_at;
    let minutes = miner.campaigns[0].drops[0].confirmed_minutes;
    // A slow telemetry, claim or progress job must not prevent segment checks.
    miner
        .busy
        .extend([JobKind::Watch, JobKind::Claim, JobKind::Poll]);
    miner.schedule_playback(&settings);
    finish_job(&mut miner, &pool).await;
    miner.schedule_playback(&settings);
    assert!(miner.jobs.is_empty());
    tokio::time::pause();
    tokio::time::advance(POLL_INTERVAL).await;
    tokio::time::resume();
    miner.schedule_playback(&settings);
    assert!(miner.busy.contains(&JobKind::Playback));
    finish_job(&mut miner, &pool).await;
    assert_eq!(miner.next_watch, watch_due);
    assert_eq!(miner.poll_at, poll_due);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, minutes);
    assert_eq!(miner.campaigns[0].drops[0].estimated_minutes, 0);
    // Same-broadcast metadata replacement must retain successful segment deduplication.
    let state = miner.playback.take().unwrap();
    let mut refreshed = miner.channels[0].clone();
    refreshed.viewers = Some(999);
    assert!(state.matches(&refreshed));
    refreshed.broadcast_id = Some("new-broadcast".into());
    assert!(!state.matches(&refreshed));
    pool.close().await;
}

#[tokio::test]
async fn pause_stream_changes_and_clear_cancel_playback_and_reject_late_results() {
    for change in ["pause", "offline", "broadcast", "clear"] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&server)
            .await;
        let (_dir, mut miner, intent, mut pool) = miner(&server).await;
        let mut settings = select(&mut miner).await;
        miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(120))));
        let manual = miner.manual;
        miner.schedule_playback(&settings);
        let cancel = miner.playback_cancel.clone().unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.method == "POST")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        match change {
            "pause" => {
                settings.mining_paused = true;
                *miner.app.settings.write().await = settings.clone();
            }
            "offline" => miner.event(Event::Offline(10)).await.unwrap(),
            "broadcast" => miner.channels[0].broadcast_id = Some("replacement".into()),
            "clear" => {
                intent
                    .send(Intent {
                        clear: 1,
                        ..Default::default()
                    })
                    .unwrap();
                miner.apply_intent(&pool).await;
            }
            _ => unreachable!(),
        }
        miner.reselect(&settings).await;
        assert!(cancel.is_cancelled(), "{change}");
        if change == "pause" {
            settings.mining_paused = false;
            *miner.app.settings.write().await = settings.clone();
            miner.reselect(&settings).await;
            assert_eq!(miner.manual, manual);
        }
        // Even a result delivered after resume cannot restore the cancelled request's state.
        finish_job(&mut miner, &pool).await;
        if change != "pause" {
            assert!(miner.playback.is_none());
        }
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn playback_requires_selection_but_supports_unknown_manual_streams_and_timers() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = Settings::default();
    miner.schedule_playback(&settings);
    assert!(miner.jobs.is_empty());
    miner.campaigns.clear();
    miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(60))));
    miner.reselect(&settings).await;
    miner.schedule_playback(&settings);
    assert!(miner.busy.contains(&JobKind::Playback));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    miner.reselect(&settings).await;
    assert!(miner.playback_cancel.as_ref().unwrap().is_cancelled());
    assert!(miner.manual.is_none());
    finish_job(&mut miner, &pool).await;
    miner.schedule_playback(&settings);
    assert!(miner.jobs.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn playback_dedup_survives_metadata_and_claim_wait_during_a_segment_request() {
    let server = MockServer::start().await;
    gql_mock(&server, |_| json!({"data":{"streamPlaybackAccessToken":{"value":"mock-token","signature":"mock-signature"}}})).await;
    for (url, body) in [
        (
            "/api/channel/hls/streamer.m3u8",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n/media.m3u8\n",
        ),
        ("/media.m3u8", "#EXTM3U\n#EXTINF:2,\n/one.ts\n"),
    ] {
        Mock::given(method("GET"))
            .and(path(url))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body, "application/vnd.apple.mpegurl"),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("HEAD"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(150)))
        .mount(&server)
        .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.schedule_playback(&settings);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == "HEAD")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut channel = miner.channels[0].clone();
    channel.viewers = Some(123);
    miner
        .complete(
            Job::Update {
                result: Ok(vec![channel]),
                requested_at: Instant::now(),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.claim_wait = Some(("another-reward".into(), Instant::now() + PROGRESS_DELAY, 0));
    finish_job(&mut miner, &pool).await;
    miner.claim_wait = None;
    miner.next_playback = Instant::now();
    miner.schedule_playback(&settings);
    finish_job(&mut miner, &pool).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "HEAD")
            .count(),
        1
    );
    assert_eq!(miner.channels[0].viewers, Some(123));
    pool.close().await;
}

#[tokio::test]
async fn pause_keeps_acknowledged_segments_but_retries_the_interrupted_request() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = MockServer::start().await;
    gql_mock(&server, |_| json!({"data":{"streamPlaybackAccessToken":{"value":"mock-token","signature":"mock-signature"}}})).await;
    for (url, body) in [
        (
            "/api/channel/hls/streamer.m3u8",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n/media.m3u8\n",
        ),
        (
            "/media.m3u8",
            "#EXTM3U\n#EXTINF:2,\n/fast.ts\n#EXTINF:2,\n/slow.ts\n",
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(url))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body, "application/vnd.apple.mpegurl"),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("HEAD"))
        .and(path("/fast.ts"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let reads = calls.clone();
    Mock::given(method("HEAD"))
        .and(path("/slow.ts"))
        .respond_with(move |_: &wiremock::Request| {
            if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(200).set_delay(Duration::from_secs(30))
            } else {
                ResponseTemplate::new(200)
            }
        })
        .mount(&server)
        .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let mut settings = select(&mut miner).await;
    miner.schedule_playback(&settings);
    tokio::time::timeout(Duration::from_secs(2), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    settings.mining_paused = true;
    *miner.app.settings.write().await = settings.clone();
    miner.reselect(&settings).await;
    finish_job(&mut miner, &pool).await;
    settings.mining_paused = false;
    *miner.app.settings.write().await = settings.clone();
    miner.reselect(&settings).await;
    miner.schedule_playback(&settings);
    finish_job(&mut miner, &pool).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    pool.close().await;
}

#[tokio::test]
async fn pause_stops_automatic_and_manual_watches_and_fences_late_results() {
    for manual in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/track"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (_dir, mut miner, intent, mut pool) = miner(&server).await;
        let mut settings = select(&mut miner).await;
        miner.channels[0].beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
        if manual {
            miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(300))));
        }
        let selection = miner.manual;
        let id = miner.campaigns[0].drops[0].id.clone();
        miner.confirm(&id, 13, &settings);
        miner.publish(&settings).await.unwrap();
        let progress = miner.app.snapshot.read().await.current_drop.clone();
        assert!(progress.is_some());
        let requested_at = Instant::now();
        let channel = miner.channels[0].clone();
        miner.spawn(JobKind::Watch, std::future::pending());
        settings.mining_paused = true;
        *miner.app.settings.write().await = settings.clone();
        miner.reselect(&settings).await;
        assert!(
            miner
                .jobs
                .join_next()
                .await
                .unwrap()
                .err()
                .unwrap()
                .is_cancelled()
        );
        miner.busy.remove(&JobKind::Watch);
        miner.watch_abort = None;
        miner.publish(&settings).await.unwrap();
        {
            let state = miner.app.snapshot.read().await;
            assert_eq!(state.mining.state, crate::dto::MiningState::Paused);
            assert_eq!(state.current_drop, progress);
            assert!(state.channels.iter().all(|c| !c.watching));
        }
        miner.poll_at = Some(Instant::now());
        miner.schedule(&settings).await;
        assert!(miner.jobs.is_empty(), "pause must block polls and beacons");
        assert_eq!(miner.manual, selection);
        let original_stream = miner.channels[0].clone();
        miner.channels[0].broadcast_id = None;
        miner.reselect(&settings).await;
        miner.publish(&settings).await.unwrap();
        assert_eq!(
            miner.app.snapshot.read().await.current_drop,
            progress,
            "a paused reward survives the stream going offline"
        );
        let original_reward = miner.campaigns[0].drops[0].clone();
        miner.campaigns[0].drops[0].confirm(14, Utc::now());
        miner.publish(&settings).await.unwrap();
        assert_eq!(
            miner
                .app
                .snapshot
                .read()
                .await
                .current_drop
                .as_ref()
                .unwrap()
                .confirmed_minutes,
            14
        );
        miner.campaigns[0].drops[0] = original_reward;
        miner.channels[0] = original_stream;
        settings.mining_paused = false;
        *miner.app.settings.write().await = settings.clone();
        miner.reselect(&settings).await;
        miner
            .complete(
                Job::Watch {
                    channel: Box::new(channel),
                    result: Ok(true),
                    requested_at,
                    at: Instant::now(),
                },
                &pool,
            )
            .await
            .unwrap();
        miner
            .complete(
                Job::Poll {
                    channel: 10,
                    result: Ok(Some((id, 29))),
                    requested_at,
                },
                &pool,
            )
            .await
            .unwrap();
        assert!(
            miner.poll_at.is_none(),
            "old watch cannot restore a polling deadline"
        );
        assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 13);
        miner.schedule(&settings).await;
        assert!(miner.busy.contains(&JobKind::Watch));
        finish_job(&mut miner, &pool).await;
        assert_eq!(miner.manual, selection);
        settings.mining_paused = true;
        miner.reselect(&settings).await;
        intent.send_modify(|intent| intent.clear += 1);
        miner.apply_intent(&pool).await;
        miner.publish(&settings).await.unwrap();
        assert!(
            miner.app.snapshot.read().await.current_drop.is_none(),
            "cache clear must discard the retained paused reward"
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn paused_mining_keeps_inventory_and_earned_claims_running() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "Inventory" => json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[],"gameEventDrops":[]}}}}),
        "DropsPage_ClaimDropRewards" => json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}),
        _ => panic!("unexpected mining operation while paused"),
    }).await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = Settings {
        mining_paused: true,
        ..Settings::default()
    };
    *miner.app.settings.write().await = settings.clone();
    miner.reselect(&settings).await;
    miner.campaigns[0].drops[0].claim_id = Some("earned-instance".into());
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Claim));
    finish_job(&mut miner, &pool).await;
    assert_eq!(History::load(dir.path()).total(), 1);
    miner.refresh = true;
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Inventory));
    finish_job(&mut miner, &pool).await;
    assert_eq!(
        miner.app.snapshot.read().await.mining.state,
        crate::dto::MiningState::Paused
    );
    pool.close().await;
}

#[tokio::test]
async fn game_directory_cancels_abandoned_queries_and_ends_with_its_generation() {
    use crate::app::commands::{GameQuery, GameRequest};
    let server = MockServer::start().await;
    Mock::given(path("/helix/search/categories"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[]}))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    let (sender, requests) = mpsc::channel(4);
    let run = tokio::spawn(session::game_directory(client.clone(), requests));
    let (complete, result) = oneshot::channel();
    sender
        .send(GameRequest {
            query: GameQuery::Search("first".into()),
            complete,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(result);
    server.reset().await;
    Mock::given(path("/helix/search/categories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .mount(&server)
        .await;
    let (complete, result) = oneshot::channel();
    sender
        .send(GameRequest {
            query: GameQuery::Search("second".into()),
            complete,
        })
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), result)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_empty()
    );
    client.http.cancel.cancel();
    assert_eq!(run.await.unwrap(), Err(TwitchError::Cancelled));
    assert!(sender.is_closed());
}

#[tokio::test]
async fn profile_survives_network_renewal_but_not_logout_or_account_changes() {
    let server = MockServer::start().await;
    let (_dir, mining, _intent, mut pool) = miner(&server).await;
    let profile = crate::dto::AccountProfile {
        display_name: "Miner".into(),
        ..Default::default()
    };
    session::publish_login(
        &mining.app,
        Login {
            user_id: Some(42),
            profile: Some(profile.clone()),
            ..Default::default()
        },
    )
    .await;
    session::publish_login(
        &mining.app,
        Login {
            user_id: Some(42),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        mining.app.snapshot.read().await.login.profile,
        Some(profile)
    );
    session::publish_login(
        &mining.app,
        Login {
            user_id: Some(43),
            ..Default::default()
        },
    )
    .await;
    assert!(mining.app.snapshot.read().await.login.profile.is_none());
    gql_mock(
        &server,
        |_| json!({"data":{"user":{"id":"42","login":"miner"}}}),
    )
    .await;
    session::refresh_profile(mining.app.clone(), mining.client.clone())
        .await
        .unwrap();
    assert!(
        mining.app.snapshot.read().await.login.profile.is_none(),
        "late data cannot replace a different account"
    );
    reset_session(&mining.app).await;
    session::refresh_profile(mining.app.clone(), mining.client.clone())
        .await
        .unwrap();
    assert!(
        mining.app.snapshot.read().await.login.profile.is_none(),
        "late data cannot undo logout"
    );
    pool.close().await;
}

#[tokio::test]
async fn authenticated_inventory_failure_keeps_login_and_recovers_only_after_success() {
    let server = MockServer::start().await;
    let (_dir, mut mining, _intent, mut pool) = miner(&server).await;
    session::publish_login(
        &mining.app,
        Login {
            user_id: Some(42),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        mining.app.snapshot.read().await.mining.state,
        crate::dto::MiningState::Discovering
    );
    mining
        .complete(
            Job::Inventory {
                result: Err(TwitchError::Network),
                requested_at: Utc::now(),
                refresh_sequence: 0,
            },
            &pool,
        )
        .await
        .unwrap();
    {
        let snapshot = mining.app.snapshot.read().await;
        assert_eq!(snapshot.login.user_id, Some(42));
        assert_ne!(
            snapshot.mining.state,
            crate::dto::MiningState::AccountRequired
        );
        assert!(!snapshot.activity.last().unwrap().recovered);
    }
    mining
        .complete(Job::Notification(Ok(())), &pool)
        .await
        .unwrap();
    assert!(
        !mining
            .app
            .snapshot
            .read()
            .await
            .activity
            .last()
            .unwrap()
            .recovered
    );
    mining
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    campaigns: vec![],
                    awards: HashMap::new(),
                    rejected_account_ids: HashSet::new(),
                    status: InventoryStatus {
                        available: true,
                        ..Default::default()
                    },
                }),
                requested_at: Utc::now(),
                refresh_sequence: 0,
            },
            &pool,
        )
        .await
        .unwrap();
    assert!(
        mining
            .app
            .snapshot
            .read()
            .await
            .activity
            .last()
            .unwrap()
            .recovered
    );
    pool.close().await;
}

#[tokio::test]
async fn notification_failures_preserve_retry_deadlines_and_due_manual_or_automatic_watches() {
    for manual in [false, true] {
        for backing_off in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/beacon"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;
            let (_dir, mut mining, _intent, mut pool) = miner(&server).await;
            let settings = select(&mut mining).await;
            if manual {
                mining.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(60))));
            }
            let selection = mining.manual;
            mining.channels[0].beacon_url =
                Some(format!("{}/beacon", server.uri()).parse().unwrap());
            mining.next_watch = Instant::now();
            let watch_due = mining.next_watch;
            if backing_off {
                mining.next_retry = Instant::now() + Duration::from_secs(30);
            }
            let retry_due = mining.next_retry;
            let console_count = mining.app.snapshot.read().await.console.len();
            mining
                .complete(Job::Notification(Err(TwitchError::Status(500))), &pool)
                .await
                .unwrap();
            assert_eq!(mining.next_retry, retry_due);
            assert_eq!(mining.next_watch, watch_due);
            assert_eq!(mining.manual, selection);
            assert_eq!(mining.watching, Some(10));
            assert_eq!(
                mining.app.snapshot.read().await.console.len(),
                console_count + 1
            );
            if !backing_off {
                assert!(mining.next_retry <= Instant::now());
                mining.schedule(&settings).await;
                assert!(mining.busy.contains(&JobKind::Watch));
                finish_job(&mut mining, &pool).await;
            }
            pool.close().await;
        }
    }
    for error in [TwitchError::Unauthorized, TwitchError::Cancelled] {
        let server = MockServer::start().await;
        let (_dir, mut mining, _intent, mut pool) = miner(&server).await;
        assert_eq!(
            mining.complete(Job::Notification(Err(error)), &pool).await,
            Err(error)
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn mining_priority_switches_preserve_reports_claim_reconciliation_and_manual_timers() {
    for mode in ["short_events", "ending_soonest"] {
        let server = MockServer::start().await;
        let (dir, mut miner, intent, mut pool) = miner(&server).await;
        let now = Utc::now();
        miner.campaigns[0].starts_at = now - chrono::Duration::days(3);
        miner.campaigns[0].ends_at = now + chrono::Duration::days(3);
        miner.campaigns[0].drops[0].starts_at = miner.campaigns[0].starts_at;
        miner.campaigns[0].drops[0].ends_at = miner.campaigns[0].ends_at;
        let mut event = Campaign::parse(&campaign_json("event"), &HashMap::new(), now).unwrap();
        event.game.id = 2;
        event.game.name = "Event game".into();
        event.starts_at = miner.campaigns[0].starts_at;
        event.ends_at = miner.campaigns[0].ends_at;
        let mut side = event.drops[0].clone();
        side.id = "side".into();
        side.starts_at = event.starts_at;
        side.ends_at = event.ends_at;
        event.drops[0].starts_at = now;
        event.drops[0].ends_at = now + chrono::Duration::hours(3);
        event.drops.push(side);
        let mut stream = miner.channels[0].clone();
        stream.identity.id = 20;
        stream.game = Some(event.game.clone());
        miner.campaigns.push(event);
        miner.channels.push(stream);
        let manual = Settings::default()
            .patched(&json!({"games_to_watch":["Rust", "Event game"]}))
            .unwrap();
        *miner.app.settings.write().await = manual.clone();
        miner.reselect(&manual).await;
        let old_poll = Instant::now();
        assert_eq!(miner.watching, Some(10));
        miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(120))));
        let deadline = miner.manual.unwrap().expires_at;
        let settings = manual
            .patched(&json!({"mining_priority_mode":mode}))
            .unwrap();
        *miner.app.settings.write().await = settings.clone();
        intent.send_modify(|intent| intent.settings += 1);
        miner.apply_intent(&pool).await;
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(10));
        assert_eq!(miner.manual.unwrap().expires_at, deadline);
        assert!(miner.channels_dirty);
        miner.manual = None;
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(20));
        miner
            .complete(
                Job::Poll {
                    channel: 10,
                    requested_at: old_poll,
                    result: Ok(Some(("drop-one".into(), 59))),
                },
                &pool,
            )
            .await
            .unwrap();
        miner.publish(&settings).await.unwrap();
        assert_eq!(
            miner
                .app
                .snapshot
                .read()
                .await
                .current_drop
                .as_ref()
                .unwrap()
                .drop_id,
            "drop-event"
        );
        assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 12);
        // Twitch, rather than the priority preview, owns the displayed reward.
        miner
            .complete(
                Job::Poll {
                    channel: 20,
                    requested_at: Instant::now(),
                    result: Ok(Some(("side".into(), 15))),
                },
                &pool,
            )
            .await
            .unwrap();
        miner.publish(&settings).await.unwrap();
        assert_eq!(
            miner
                .app
                .snapshot
                .read()
                .await
                .current_drop
                .as_ref()
                .unwrap()
                .drop_id,
            "side"
        );
        let due = miner.next_watch;
        miner.reselect(&settings).await;
        assert_eq!(miner.next_watch, due);
        assert!(miner.confirm("drop-event", 60, &settings));
        miner.campaigns[1].drops[0].claim_id = Some("account-event-instance".into());
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(10));
        let old_event_poll = Instant::now() - Duration::from_secs(1);
        miner
            .complete(
                Job::Poll {
                    channel: 20,
                    requested_at: old_event_poll,
                    result: Ok(Some(("drop-event".into(), 59))),
                },
                &pool,
            )
            .await
            .unwrap();
        miner
            .complete(
                Job::Inventory {
                    requested_at: now,
                    refresh_sequence: 0,
                    result: Err(TwitchError::Network),
                },
                &pool,
            )
            .await
            .unwrap();
        miner.publish(&settings).await.unwrap();
        assert_eq!(
            miner
                .app
                .snapshot
                .read()
                .await
                .current_drop
                .as_ref()
                .unwrap()
                .drop_id,
            "drop-one"
        );
        assert!(!miner.campaigns[1].drops[0].claimed);
        assert_eq!(History::load(dir.path()).total(), 0);
        let mut lagging = miner.campaigns.clone();
        lagging[1].drops[0].confirm(50, Utc::now());
        lagging[1].drops[0].claim_id = None;
        miner
            .complete(
                Job::Inventory {
                    requested_at: Utc::now(),
                    refresh_sequence: 0,
                    result: Ok(Inventory {
                        rejected_account_ids: HashSet::new(),
                        campaigns: lagging,
                        awards: HashMap::new(),
                        status: InventoryStatus {
                            available: true,
                            ..InventoryStatus::default()
                        },
                    }),
                },
                &pool,
            )
            .await
            .unwrap();
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(10));
        assert_eq!(miner.campaigns[1].drops[0].confirmed_minutes, 60);
        assert_eq!(
            miner.campaigns[1].drops[0].claim_id.as_deref(),
            Some("account-event-instance")
        );
        assert_eq!(History::load(dir.path()).total(), 0);
        let mut claimed = miner.campaigns.clone();
        claimed[1].drops[0].mark_claimed(Utc::now());
        miner
            .complete(
                Job::Inventory {
                    requested_at: Utc::now(),
                    refresh_sequence: 0,
                    result: Ok(Inventory {
                        rejected_account_ids: HashSet::new(),
                        campaigns: claimed,
                        awards: HashMap::new(),
                        status: InventoryStatus {
                            available: true,
                            ..InventoryStatus::default()
                        },
                    }),
                },
                &pool,
            )
            .await
            .unwrap();
        assert_eq!(History::load(dir.path()).total(), 1);
        assert_eq!(
            History::load(dir.path()).entries(&HistoryFilter::default())[0].id,
            "drop-event"
        );
        assert_eq!(
            miner.app.settings.read().await.games_to_watch,
            manual.games_to_watch
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn empty_current_drop_preserves_watching_polling_and_account_evidence() {
    for manual in [false, true] {
        for claim_wait in [false, true] {
            let server = MockServer::start().await;
            gql_mock(&server, |q| {
                assert_eq!(q["operationName"], "DropCurrentSessionContext");
                json!({"data":{"currentUser":{"dropCurrentSession":{
                    "__typename":"DropCurrentSession", "channel":null,
                    "currentMinutesWatched":0, "dropID":"", "game":null,
                    "requiredMinutesWatched":0
                }}}})
            })
            .await;
            Mock::given(method("POST"))
                .and(path("/beacon"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;
            let (dir, mut miner, _intent, mut pool) = miner(&server).await;
            let settings = select(&mut miner).await;
            if manual {
                miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(60))));
            }
            let selection = miner.manual;
            if claim_wait {
                miner.claim_wait = Some(("previous-reward".into(), Instant::now(), 0));
            }
            miner.channels[0].beacon_url =
                Some(format!("{}/beacon", server.uri()).parse().unwrap());
            miner.next_watch = Instant::now() + WATCH_INTERVAL;
            let watch_due = miner.next_watch;
            let retry_due = miner.next_retry;
            let console = miner.app.snapshot.read().await.console.clone();
            miner.poll_at = Some(Instant::now());
            miner.schedule(&settings).await;
            assert!(miner.busy.contains(&JobKind::Poll));
            finish_job(&mut miner, &pool).await;

            assert_eq!(miner.next_retry, retry_due, "no error-triggered backoff");
            assert_eq!(miner.app.snapshot.read().await.console, console);
            assert_eq!(miner.watching, Some(10));
            assert_eq!(miner.manual, selection);
            assert!(miner.claim_wait.is_none());
            assert!(miner.last_progress.is_none());
            let drop = &miner.campaigns[0].drops[0];
            assert_eq!(drop.confirmed_minutes, 12);
            assert!(drop.confirmed_at.is_some());
            assert!(!drop.claimed);
            assert!(drop.claim_id.is_none());
            assert_eq!(drop.estimated_minutes, u32::from(!manual && !claim_wait));
            assert_eq!(History::load(dir.path()).total(), 0);
            if claim_wait {
                assert!(
                    miner.next_watch <= Instant::now(),
                    "claim wait releases watching"
                );
            } else {
                assert_eq!(miner.next_watch, watch_due, "keep normal watch cadence");
                miner.schedule(&settings).await;
                assert!(miner.jobs.is_empty(), "do not send an early watch event");
                miner.next_watch = Instant::now();
            }
            miner.schedule(&settings).await;
            assert!(miner.busy.contains(&JobKind::Watch));
            finish_job(&mut miner, &pool).await;
            assert!(
                miner.poll_at.is_some(),
                "acknowledged watches keep progress polling"
            );
            miner.poll_at = Some(Instant::now());
            miner.schedule(&settings).await;
            assert!(miner.busy.contains(&JobKind::Poll));
            finish_job(&mut miner, &pool).await;
            assert_eq!(miner.next_retry, retry_due);
            pool.close().await;
        }
    }
}

#[tokio::test]
async fn completed_transition_displays_twitchs_next_reward_before_claim_evidence() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| {
        assert_eq!(q["operationName"], "DropCurrentSessionContext");
        json!({"data":{"currentUser":{"dropCurrentSession":{"channel":{"id":"10"},"dropID":"next-reward","currentMinutesWatched":4}}}})
    }).await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let mut next = miner.campaigns[0].drops[0].clone();
    next.id = "next-reward".into();
    next.confirmed_minutes = 0;
    let mut almost_done = next.clone();
    almost_done.id = "other-reward".into();
    almost_done.confirmed_minutes = 59;
    miner.campaigns[0].drops.extend([next, almost_done]);
    for (id, minutes) in [("drop-one", 60), ("next-reward", 3)] {
        miner
            .event(Event::Progress {
                id: id.into(),
                minutes,
            })
            .await
            .unwrap();
    }
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "next-reward"
    );
    assert!(!miner.campaigns[0].drops[0].claimed);
    assert!(miner.campaigns[0].drops[0].claim_id.is_none());
    assert_eq!(History::load(dir.path()).total(), 0);
    // A CurrentDrop response must make the same transition as PubSub.
    miner.last_progress = Some(("drop-one".into(), Instant::now()));
    miner.poll_at = Some(Instant::now());
    miner.next_watch = Instant::now() + WATCH_INTERVAL;
    miner.schedule(&settings).await;
    assert!(
        miner.busy.contains(&JobKind::Poll),
        "completed progress must not suppress CurrentDrop"
    );
    finish_job(&mut miner, &pool).await;
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "next-reward"
    );
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_preserves_selected_game_priority_over_automatic_badges() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let mut settings = select(&mut miner).await;
    settings.auto_mine_badges = true;
    settings.games_to_watch.push("Second".into());
    *miner.app.settings.write().await = settings.clone();
    let mut second =
        Campaign::parse(&campaign_json("second"), &HashMap::new(), Utc::now()).unwrap();
    second.game.id = 2;
    second.game.name = "Second".into();
    let mut channel = miner.channels[0].clone();
    channel.identity.id = 11;
    channel.game = Some(second.game.clone());
    let mut badge = Campaign::parse(&campaign_json("badge"), &HashMap::new(), Utc::now()).unwrap();
    badge.game.id = 509663;
    badge.game.name = "Special Events".into();
    badge.allowed_channels = vec![miner.channels[0].identity.clone()];
    badge.drops[0].confirmed_minutes = 59;
    badge.drops[0].benefits[0].kind = "BADGE".into();
    miner.channels.push(channel);
    miner.campaigns.extend([second, badge]);
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "drop-one"
    );
    miner.confirm("drop-one", 60, &settings);
    miner.reselect(&settings).await;
    assert_eq!(
        miner.watching,
        Some(11),
        "the next selected game outranks the optional badge on the old channel"
    );
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "drop-second"
    );
    assert!(!miner.campaigns[0].drops[0].claimed);
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_uses_successor_evidence_even_when_final_progress_and_claim_events_are_missing()
 {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.campaigns[0].drops[0].confirmed_minutes = 57;
    let mut next = miner.campaigns[0].drops[0].clone();
    next.id = "next".into();
    next.confirmed_minutes = 0;
    next.prerequisites = vec!["drop-one".into()];
    miner.campaigns[0].drops.push(next);
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at: Instant::now(),
                result: Ok(Some(("next".into(), 3))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "next"
    );
    assert!(miner.refresh);
    assert!(
        !miner.progress_eligible("next", 10, &settings),
        "reported successor progress is not a prerequisite claim"
    );
    assert!(!miner.campaigns[0].drops[0].claimed);
    assert_eq!(History::load(dir.path()).total(), 0);
    let reported_at = miner.last_progress.as_ref().unwrap().1;
    miner
        .event(Event::Progress {
            id: "next".into(),
            minutes: 3,
        })
        .await
        .unwrap();
    assert_eq!(
        miner.last_progress.as_ref().unwrap().1,
        reported_at,
        "duplicate progress cannot postpone polling"
    );
    miner
        .event(Event::Progress {
            id: "drop-one".into(),
            minutes: 58,
        })
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "next",
        "late predecessor progress must not replace the reported successor"
    );
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 58);
    // The same ordering holds through intermediate rewards, even without benefits.
    let mut intermediate = miner.campaigns[0].drops[1].clone();
    intermediate.id = "intermediate".into();
    intermediate.benefits.clear();
    miner.campaigns[0].drops[1].prerequisites = vec!["intermediate".into()];
    miner.campaigns[0].drops.push(intermediate);
    miner
        .event(Event::Progress {
            id: "drop-one".into(),
            minutes: 59,
        })
        .await
        .unwrap();
    assert_eq!(miner.last_progress.as_ref().unwrap().0, "next");
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_reconciles_and_releases_channel_without_unlocking_prerequisites() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let mut next = miner.campaigns[0].drops[0].clone();
    next.id = "dependent".into();
    next.prerequisites = vec!["drop-one".into()];
    miner.campaigns[0].drops.push(next);
    miner.campaigns[0].drops[0].confirmed_minutes = 59;
    miner.campaigns[0].drops[0].estimated_minutes = 1;
    miner.reselect(&settings).await;
    assert_eq!(
        miner.watching,
        Some(10),
        "estimated completion is not account evidence"
    );
    assert!(!miner.refresh);
    miner
        .event(Event::Progress {
            id: "drop-one".into(),
            minutes: 60,
        })
        .await
        .unwrap();
    assert!(
        miner.refresh,
        "confirmed completion needs account reconciliation even without a claim notification"
    );
    miner.reselect(&settings).await;
    assert!(
        miner.watching.is_none(),
        "completed unclaimed reward kept its channel eligible"
    );
    assert!(!miner.campaigns[0].prerequisites_met(&miner.campaigns[0].drops[1]));
    assert!(!miner.campaigns[0].view(&settings, Utc::now()).finished);
    assert_eq!(History::load(dir.path()).total(), 0);
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_rejects_old_polls_after_same_channel_stream_replacement() {
    for rebuild in [false, true] {
        let server = MockServer::start().await;
        let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
        let settings = select(&mut miner).await;
        let update_started = Instant::now();
        let poll_started = Instant::now();
        let mut channels = miner.channels.clone();
        channels[0].broadcast_id = Some("stream2".into());
        let update = if rebuild {
            Job::Channels {
                result: Ok(channels),
                requested_at: update_started,
            }
        } else {
            Job::Update {
                result: Ok(channels),
                requested_at: update_started,
            }
        };
        miner.complete(update, &pool).await.unwrap();
        miner
            .complete(
                Job::Poll {
                    channel: 10,
                    requested_at: poll_started,
                    result: Ok(Some(("drop-one".into(), 40))),
                },
                &pool,
            )
            .await
            .unwrap();
        assert_eq!(
            miner.campaigns[0].drops[0].confirmed_minutes, 12,
            "a poll from stream1 changed progress after stream2 was published"
        );
        assert!(miner.last_progress.is_none());
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(10));
        pool.close().await;
    }
}

#[tokio::test]
async fn completed_transition_retains_expired_claim_evidence_on_partial_inventory() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.confirm("drop-one", 60, &settings);
    miner.campaigns[0].ends_at = Utc::now() - chrono::Duration::seconds(1);
    miner
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    rejected_account_ids: HashSet::new(),
                    campaigns: vec![],
                    awards: HashMap::new(),
                    status: InventoryStatus::default(),
                }),
                requested_at: Utc::now(),
                refresh_sequence: 0,
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(
        miner.campaigns.len(),
        1,
        "partial refresh must preserve unresolved completion through the claim grace period"
    );
    assert!(!miner.campaigns[0].drops[0].claimed);
    miner.refresh = false;
    miner.next_progress_refresh = Instant::now();
    miner.busy.insert(JobKind::Inventory);
    miner.schedule(&settings).await;
    assert!(miner.refresh);
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_preserves_new_claim_ids_alongside_newer_progress() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let requested_at = Utc::now();
    miner.confirm("drop-one", 60, &settings);
    let mut raw = campaign_json("one");
    raw["timeBasedDrops"][0]["self"]["dropInstanceID"] = json!("new-account-instance");
    miner
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    rejected_account_ids: HashSet::new(),
                    campaigns: vec![Campaign::parse(&raw, &HashMap::new(), Utc::now()).unwrap()],
                    awards: HashMap::new(),
                    status: InventoryStatus {
                        available: true,
                        ..InventoryStatus::default()
                    },
                }),
                requested_at,
                refresh_sequence: 0,
            },
            &pool,
        )
        .await
        .unwrap();
    let drop = &miner.campaigns[0].drops[0];
    assert_eq!(drop.confirmed_minutes, 60);
    assert_eq!(drop.claim_id.as_deref(), Some("new-account-instance"));
    assert!(!drop.claimed);
    assert_eq!(History::load(dir.path()).total(), 0);
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_recovers_failed_and_delayed_inventory_and_claim_evidence() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for auto_claimed in [false, true] {
        let server = MockServer::start().await;
        let inventories = Arc::new(AtomicUsize::new(0));
        let claims = Arc::new(AtomicUsize::new(0));
        let inventory_count = inventories.clone();
        let claim_count = claims.clone();
        gql_mock(&server, move |q| match q["operationName"].as_str().unwrap() {
            "Inventory" => {
                let attempt = inventory_count.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    return json!({"data":{"currentUser":{"inventory":null}}});
                }
                let mut campaign = campaign_json("one");
                campaign["timeBasedDrops"][0]["self"]["currentMinutesWatched"] = json!(59);
                if attempt >= 2 {
                    campaign["timeBasedDrops"][0]["self"]["isClaimed"] = json!(auto_claimed);
                    if !auto_claimed {
                        campaign["timeBasedDrops"][0]["self"]["dropInstanceID"] = json!("account-instance");
                    }
                }
                let mut next = campaign_json("next")["timeBasedDrops"][0].clone();
                next["preconditionDrops"] = json!([{"id":"drop-one"}]);
                campaign["timeBasedDrops"].as_array_mut().unwrap().push(next);
                json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign],"gameEventDrops":[]}}}})
            },
            "DropsPage_ClaimDropRewards" => {
                assert!(!auto_claimed, "auto-claims are imported without a claim RPC");
                assert_eq!(q["variables"]["input"]["dropInstanceID"], "account-instance");
                let attempt = claim_count.fetch_add(1, Ordering::SeqCst);
                json!({"data":{"claimDropRewards":{"status":if attempt == 0 {"NOT_ELIGIBLE"} else {"DROP_INSTANCE_ALREADY_CLAIMED"}}}})
            },
            other => panic!("unexpected operation {other}"),
        }).await;
        let (dir, mut miner, _intent, mut pool) = miner(&server).await;
        let settings = select(&mut miner).await;
        miner.confirm("drop-one", 60, &settings);
        miner.reselect(&settings).await;
        assert!(miner.watching.is_none());
        for attempt in 0..3 {
            if attempt > 0 {
                // Advance the recovery deadline without waiting a real minute for mock I/O.
                miner.next_progress_refresh = Instant::now();
                miner.next_refresh = Instant::now();
            }
            miner.schedule(&settings).await;
            assert!(miner.busy.contains(&JobKind::Inventory));
            for _ in 0..5 {
                miner.confirm("drop-one", 60, &settings);
                miner.schedule(&settings).await;
                assert!(!miner.refresh, "completion must join the in-flight refresh");
            }
            finish_job(&mut miner, &pool).await;
            assert_eq!(inventories.load(Ordering::SeqCst), attempt + 1);
            assert!(
                miner.next_retry <= Instant::now(),
                "inventory failure must not pause watch work"
            );
            if attempt < 2 {
                assert!(!miner.campaigns[0].drops[0].claimed);
                assert!(miner.campaigns[0].drops[0].claim_id.is_none());
                // Failed refreshes preserve evidence; fresh contrary inventory corrects it.
                assert_eq!(
                    miner.campaigns[0].drops[0].confirmed_minutes,
                    if attempt == 0 { 60 } else { 59 }
                );
                assert_eq!(miner.campaigns[0].drops[0].progress_disputed, attempt == 1);
                assert_eq!(History::load(dir.path()).total(), 0);
                assert!(miner.next_progress_refresh > Instant::now() + Duration::from_secs(59));
                miner.channels_dirty = false;
                miner.schedule(&settings).await;
                assert!(miner.jobs.is_empty(), "no immediate inventory retry loop");
            }
        }
        if !auto_claimed {
            assert_eq!(History::load(dir.path()).total(), 0);
            miner.schedule(&settings).await;
            finish_job(&mut miner, &pool).await;
            assert!(!miner.campaigns[0].drops[0].claimed);
            assert_eq!(History::load(dir.path()).total(), 0);
            assert_eq!(
                miner.campaigns[0].drops[0].claim_id.as_deref(),
                Some("account-instance")
            );
            assert!(miner.claim_retry["drop-one"] > Instant::now() + Duration::from_secs(59));
            miner.claim_retry.insert("drop-one".into(), Instant::now());
            miner.schedule(&settings).await;
            finish_job(&mut miner, &pool).await;
        }
        miner.reselect(&settings).await;
        miner.publish(&settings).await.unwrap();
        assert!(miner.campaigns[0].drops[0].claimed);
        assert!(miner.campaigns[0].prerequisites_met(&miner.campaigns[0].drops[1]));
        assert_eq!(miner.watching, Some(10));
        assert_eq!(
            miner
                .app
                .snapshot
                .read()
                .await
                .current_drop
                .as_ref()
                .unwrap()
                .drop_id,
            "drop-next"
        );
        assert_eq!(History::load(dir.path()).total(), 1);
        assert!(
            ClaimJournal::load(dir.path())
                .unwrap()
                .pending(42)
                .is_empty()
        );
        assert_eq!(
            claims.load(Ordering::SeqCst),
            if auto_claimed { 0 } else { 2 }
        );
        pool.close().await;
    }
}

#[tokio::test(start_paused = true)]
async fn completed_transition_refreshes_at_most_once_per_minute_and_stops_at_claim_deadline() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.confirm("drop-one", 60, &settings);
    assert!(miner.refresh);
    miner.refresh = false;
    miner.reselect(&settings).await;
    tokio::time::advance(Duration::from_secs(59)).await;
    miner.confirm("drop-one", 60, &settings);
    assert!(!miner.refresh);
    tokio::time::advance(Duration::from_secs(1)).await;
    // Keep work owned but held so scheduling itself can be checked without network I/O.
    miner.busy.insert(JobKind::Inventory);
    miner.schedule(&settings).await;
    assert!(
        miner.refresh,
        "reconciliation must retry without another progress notification"
    );
    miner.refresh = false;
    miner.campaigns[0].ends_at = Utc::now() - chrono::Duration::hours(24);
    tokio::time::advance(Duration::from_secs(60)).await;
    miner.schedule(&settings).await;
    assert!(
        !miner.refresh,
        "expired claim grace must not cause endless reconciliation"
    );
    pool.close().await;
}

#[tokio::test]
async fn completed_transition_ignores_old_polls_and_unrelated_or_regressive_progress() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let mut next = miner.campaigns[0].drops[0].clone();
    next.id = "next".into();
    next.confirmed_minutes = 0;
    miner.campaigns[0].drops.push(next);
    let mut other = Campaign::parse(&campaign_json("other"), &HashMap::new(), Utc::now()).unwrap();
    other.game.id = 2;
    miner.campaigns.push(other);
    let requested_at = Instant::now();
    miner.confirm("drop-one", 60, &settings);
    miner.confirm("next", 3, &settings);
    for (id, minutes) in [("drop-one", 59), ("drop-one", 60), ("drop-other", 30)] {
        miner
            .event(Event::Progress {
                id: id.into(),
                minutes,
            })
            .await
            .unwrap();
    }
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at,
                result: Ok(Some(("drop-one".into(), 12))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .drop_id,
        "next"
    );
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 60);
    assert_eq!(miner.campaigns[0].drops[1].confirmed_minutes, 3);
    assert_eq!(
        miner.campaigns[1].drops[0].confirmed_minutes, 30,
        "unrelated account evidence still updates its reward"
    );

    let mut another = miner.channels[0].clone();
    another.identity.id = 11;
    miner.channels.push(another);
    let requested_at = Instant::now();
    miner.manual = Some(ManualSelection::new(11, None));
    miner.reselect(&settings).await;
    miner.manual = Some(ManualSelection::new(10, Some(Duration::from_secs(60))));
    miner.reselect(&settings).await;
    let deadline = miner.manual.unwrap().expires_at;
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at,
                result: Ok(Some(("next".into(), 25))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    assert!(
        miner.app.snapshot.read().await.current_drop.is_none(),
        "a poll from an earlier watch must not seed manual progress"
    );
    assert_eq!(miner.campaigns[0].drops[1].confirmed_minutes, 3);
    assert_eq!(miner.manual.unwrap().expires_at, deadline);
    pool.close().await;
}

#[tokio::test]
async fn refresh_finishes_after_publication_and_partial_or_failed_requests_keep_known_campaigns() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "Inventory" => json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign_json("one")],"gameEventDrops":[]}}}}),
        other => panic!("unexpected operation {other}"),
    }).await;
    let mut public = campaign_json("public");
    public["allow"] = json!({"isEnabled":false});
    public["timeBasedDrops"][0]["preconditionDrops"] = serde_json::Value::Null;
    Mock::given(method("GET"))
        .and(path("/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lastUpdatedAt":Utc::now().to_rfc3339(),
            "data":[{"rewards":[public]}]
        })))
        .mount(&server)
        .await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    intent.send_modify(|v| v.refresh += 1);
    miner.apply_intent(&pool).await;
    miner.schedule(&Settings::default()).await;
    assert_eq!(
        miner.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Refreshing
    );
    intent.send_modify(|v| v.refresh += 1);
    miner.apply_intent(&pool).await;
    assert!(
        !miner.refresh,
        "a repeated request should join the in-flight inventory job"
    );
    finish_job(&mut miner, &pool).await;
    {
        let state = miner.app.snapshot.read().await;
        assert_eq!(state.inventory_refresh.state, RefreshState::Refreshed);
        assert!(state.inventory_refresh.error.is_none());
        assert!(state.inventory_status.available);
        assert!(state.campaigns.iter().any(|c| c.id == "one"));
        assert!(state.campaigns.iter().any(|c| c.id == "public"));
    }
    let (sequence, _) = miner.app.begin_inventory_refresh().await;
    miner
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    rejected_account_ids: HashSet::new(),
                    campaigns: vec![],
                    status: InventoryStatus {
                        available: false,
                        checked_at: Some(Utc::now()),
                        catalog_updated_at: None,
                    },
                    awards: HashMap::new(),
                }),
                requested_at: Utc::now(),
                refresh_sequence: sequence,
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(
        miner.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Failed
    );
    assert_eq!(
        miner.campaigns.len(),
        2,
        "partial responses must not erase still-active metadata"
    );
    let (sequence, _) = miner.app.begin_inventory_refresh().await;
    miner
        .complete(
            Job::Inventory {
                result: Err(TwitchError::Network),
                requested_at: Utc::now(),
                refresh_sequence: sequence,
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(
        miner.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Failed
    );
    assert_eq!(miner.campaigns.len(), 2);
    let (sequence, _) = miner.app.begin_inventory_refresh().await;
    miner
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    rejected_account_ids: HashSet::new(),
                    campaigns: vec![],
                    status: InventoryStatus {
                        available: true,
                        checked_at: Some(Utc::now()),
                        catalog_updated_at: None,
                    },
                    awards: HashMap::new(),
                }),
                requested_at: Utc::now(),
                refresh_sequence: sequence,
            },
            &pool,
        )
        .await
        .unwrap();
    assert!(
        miner.campaigns.is_empty(),
        "a complete empty result is authoritative"
    );
    let (sequence, _) = miner.app.begin_inventory_refresh().await;
    reset_session(&miner.app).await;
    miner.app.finish_inventory_refresh(sequence, None).await;
    assert_eq!(
        miner.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Idle
    );
    pool.close().await;
}

#[tokio::test(start_paused = true)]
async fn unknown_progress_requests_inventory_without_fabricating_rewards_or_repeated_scans() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner
        .event(Event::Progress {
            id: "unknown".into(),
            minutes: 1,
        })
        .await
        .unwrap();
    assert!(miner.refresh);
    assert!(miner.last_progress.is_none());
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 12);
    miner.refresh = false;
    miner
        .event(Event::Progress {
            id: "another-unknown".into(),
            minutes: 2,
        })
        .await
        .unwrap();
    assert!(!miner.refresh);
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(!miner.confirm("unknown", 3, &Settings::default()));
    assert!(miner.refresh);
    pool.close().await;
}

#[tokio::test]
async fn only_selected_games_send_beacons_and_inventory_io_does_not_block_watch_cadence() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/streamer"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"beacon_url":"{}/track"}}"#, server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/track"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.schedule(&Settings::default()).await;
    assert!(miner.jobs.is_empty());
    let settings = select(&mut miner).await;
    assert_eq!(miner.watching, Some(10));
    miner.busy.insert(JobKind::Inventory);
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Watch));
    finish_job(&mut miner, &pool).await;
    assert!(miner.next_watch >= Instant::now() + Duration::from_secs(58));
    assert!(miner.poll_at.unwrap() >= Instant::now() + Duration::from_secs(19));
    miner.schedule(&settings).await;
    assert!(miner.jobs.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.iter().filter(|r| r.url.path() == "/track").count(),
        1
    );
    *miner.app.settings.write().await = Settings::default();
    miner.reselect(&Settings::default()).await;
    assert!(miner.watching.is_none());
    pool.close().await;
}

#[tokio::test]
async fn failed_cached_beacon_is_rediscovered_and_late_metadata_cannot_restore_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/streamer"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"beacon_url":"{}/new-track"}}"#, server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    for (endpoint, status) in [("/old-track", 410), ("/new-track", 204)] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
    }
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.channels[0].beacon_url = Some(format!("{}/old-track", server.uri()).parse().unwrap());
    let stale_channels = miner.channels.clone();
    let requested_at = Instant::now();
    let settings = select(&mut miner).await;
    miner.schedule(&settings).await;
    finish_job(&mut miner, &pool).await;
    assert!(miner.channels[0].beacon_url.is_none());
    miner
        .complete(
            Job::Update {
                result: Ok(stale_channels),
                requested_at,
            },
            &pool,
        )
        .await
        .unwrap();
    assert!(miner.channels[0].beacon_url.is_none());
    miner.next_watch = Instant::now();
    miner.schedule(&settings).await;
    finish_job(&mut miner, &pool).await;
    assert_eq!(
        miner.channels[0].beacon_url.as_ref().unwrap().path(),
        "/new-track"
    );
    pool.close().await;
}

#[tokio::test]
async fn watch_cache_changes_preserve_overlapping_stream_metadata_in_both_completion_orders() {
    for discovery in [false, true] {
        for change in [
            "offline",
            "broadcast",
            "category",
            "drops-disabled",
            "unchanged",
        ] {
            let server = MockServer::start().await;
            let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
            select(&mut miner).await;
            miner.channels[0].beacon_url = Some(format!("{}/old", server.uri()).parse().unwrap());
            let watch_started = Instant::now();
            let mut watched = miner.channels[0].clone();
            watched.beacon_url = None;
            let metadata_started = Instant::now();
            let mut fresh = miner.channels[0].clone();
            match change {
                "offline" => fresh = Channel::offline(fresh.identity, false),
                "broadcast" => {
                    fresh.broadcast_id = Some("stream2".into());
                    fresh.beacon_url = None;
                }
                "category" => fresh.game = None,
                "drops-disabled" => fresh.drops_enabled = false,
                _ => {}
            }
            let update = if discovery {
                Job::Channels {
                    result: Ok(vec![fresh.clone()]),
                    requested_at: metadata_started,
                }
            } else {
                Job::Update {
                    result: Ok(vec![fresh.clone()]),
                    requested_at: metadata_started,
                }
            };
            let watch = Job::Watch {
                channel: Box::new(watched),
                result: Err(TwitchError::Network),
                requested_at: watch_started,
                at: Instant::now(),
            };
            if change == "unchanged" {
                // A newer unchanged refresh cannot suppress an in-flight failure.
                miner.complete(update, &pool).await.unwrap();
                miner.complete(watch, &pool).await.unwrap();
            } else {
                miner.complete(watch, &pool).await.unwrap();
                miner.complete(update, &pool).await.unwrap();
            }
            let current = &miner.channels[0];
            assert_eq!(current.broadcast_id, fresh.broadcast_id, "{change}");
            assert_eq!(current.game, fresh.game, "{change}");
            assert_eq!(current.drops_enabled, fresh.drops_enabled, "{change}");
            assert!(current.beacon_url.is_none(), "{change}");
            assert_eq!(miner.watch_failures, 1, "{change}");
            pool.close().await;
        }
    }
}

#[tokio::test]
async fn repeated_watch_failures_renew_connections_but_success_and_stale_results_do_not() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    select(&mut miner).await;
    for (result, count) in [
        (Err(TwitchError::Network), 1),
        (Ok(false), 2),
        (Ok(true), 0),
        (Err(TwitchError::Network), 1),
        (Err(TwitchError::Network), 2),
        (Err(TwitchError::Network), 3),
    ] {
        let completed = miner
            .complete(
                Job::Watch {
                    channel: Box::new(miner.channels[0].clone()),
                    result,
                    requested_at: Instant::now(),
                    at: Instant::now(),
                },
                &pool,
            )
            .await;
        assert_eq!(miner.watch_failures, count);
        assert_eq!(
            completed,
            if count == 3 {
                Err(TwitchError::Network)
            } else {
                Ok(())
            }
        );
    }
    miner.watch_failures = 0;
    miner.refresh_channels.clear();
    let stale = miner.channels[0].clone();
    let requested_at = Instant::now();
    miner.channels[0].beacon_url = Some(format!("{}/fresh", server.uri()).parse().unwrap());
    miner
        .beacon_events
        .insert(10, requested_at + Duration::from_nanos(1));
    for result in [Err(TwitchError::Network), Ok(true)] {
        miner
            .complete(
                Job::Watch {
                    channel: Box::new(stale.clone()),
                    result,
                    requested_at,
                    at: Instant::now(),
                },
                &pool,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        miner.channels[0].beacon_url.as_ref().unwrap().path(),
        "/fresh"
    );
    assert_eq!(miner.watch_failures, 0);
    assert!(miner.refresh_channels.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn already_claimed_automatic_badge_is_recorded_once_and_does_not_block_next_reward() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| {
        assert_eq!(q["operationName"], "DropsPage_ClaimDropRewards");
        json!({"data":{"claimDropRewards":{"status":"DROP_INSTANCE_ALREADY_CLAIMED"}}})
    })
    .await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = Settings {
        auto_mine_badges: true,
        ..Settings::default()
    };
    *miner.app.settings.write().await = settings.clone();
    miner.campaigns[0].drops[0].benefits[0].kind = "BADGE".into();
    let mut next = miner.campaigns[0].drops[0].clone();
    next.id = "next-badge".into();
    next.prerequisites = vec!["drop-one".into()];
    miner.campaigns[0].drops.push(next);
    miner.campaigns[0].drops[0].claim_id = Some("earned-instance".into());
    miner.reselect(&settings).await;
    miner.schedule(&settings).await;
    finish_job(&mut miner, &pool).await;
    assert!(miner.campaigns[0].drops[0].claimed);
    assert_eq!(History::load(dir.path()).total(), 1);
    for _ in 0..8 {
        miner
            .complete(
                Job::Poll {
                    channel: 10,
                    requested_at: Instant::now(),
                    result: Ok(Some(("drop-one".into(), 60))),
                },
                &pool,
            )
            .await
            .unwrap();
    }
    assert!(
        miner.claim_wait.is_none(),
        "a stale CurrentDrop must not block watching forever"
    );
    assert_eq!(
        miner.campaigns[0]
            .first_drop(&settings, Utc::now())
            .unwrap()
            .id,
        "next-badge"
    );
    assert!(miner.journal.lock().await.pending(42).is_empty());
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Watch));
    assert!(!miner.busy.contains(&JobKind::Claim));
    miner.client.http.cancel.cancel();
    while miner.jobs.join_next().await.is_some() {}
    pool.close().await;
}

#[tokio::test]
async fn inventory_imports_confirmed_badges_without_claiming_or_resurrecting_cleared_history() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let mut notifications = miner.app.notifications.subscribe();
    let awarded_at = Utc::now() - chrono::Duration::minutes(1);
    let awards = HashMap::from([
        ("benefit-one".into(), awarded_at),
        ("benefit-unclaimed".into(), awarded_at),
    ]);
    let mut badge = campaign_json("one");
    badge["timeBasedDrops"][0]["self"] = serde_json::Value::Null;
    badge["timeBasedDrops"][0]["benefitEdges"][0]["benefit"]["distributionType"] = json!("BADGE");
    let mut unknown_time = campaign_json("unknown-time");
    unknown_time["timeBasedDrops"][0]["self"]["isClaimed"] = json!(true);
    let campaigns: Vec<_> = [badge, unknown_time, campaign_json("unclaimed")]
        .iter()
        .map(|v| Campaign::parse(v, &awards, Utc::now()).unwrap())
        .collect();
    miner.campaigns.push(
        Campaign::parse(&campaign_json("unknown-time"), &HashMap::new(), Utc::now()).unwrap(),
    );
    let started = Utc::now();
    for round in 0..3 {
        if round == 2 {
            miner.app.history.lock().await.clear().unwrap();
            *miner.app.history.lock().await = History::load(dir.path());
        }
        let (refresh_sequence, _) = miner.app.begin_inventory_refresh().await;
        let requested_at = Utc::now();
        if round == 0 {
            // Progress during refresh is not evidence that an incoming confirmed
            // claim (explicit account state or award inference) is unclaimed.
            for id in ["drop-one", "drop-unknown-time"] {
                miner
                    .event(Event::Progress {
                        id: id.into(),
                        minutes: 30,
                    })
                    .await
                    .unwrap();
            }
        }
        miner
            .complete(
                Job::Inventory {
                    result: Ok(Inventory {
                        rejected_account_ids: HashSet::new(),
                        campaigns: campaigns.clone(),
                        awards: awards.clone(),
                        status: InventoryStatus {
                            available: true,
                            checked_at: Some(Utc::now()),
                            catalog_updated_at: None,
                        },
                    }),
                    requested_at,
                    refresh_sequence,
                },
                &pool,
            )
            .await
            .unwrap();
        let history = History::load(dir.path());
        assert_eq!(history.total(), if round == 2 { 0 } else { 2 });
        if round == 0 {
            let entries = history.entries(&HistoryFilter::default());
            let badge = entries.iter().find(|e| e.id == "drop-one").unwrap();
            assert_eq!(badge.claimed_at, awarded_at);
            assert!(!badge.claimed_at_is_observed);
            let unknown = entries
                .iter()
                .find(|e| e.id == "drop-unknown-time")
                .unwrap();
            assert!(unknown.claimed_at_is_observed);
            assert!(unknown.claimed_at >= started);
            assert!(
                miner
                    .campaigns
                    .iter()
                    .find(|c| c.id == "one")
                    .unwrap()
                    .drops[0]
                    .claimed,
                "progress received during refresh must not erase Twitch claim evidence"
            );
        }
    }
    assert!(miner.jobs.is_empty());
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method != "POST"),
        "inventory imports must not make claim RPCs"
    );
    assert!(miner.journal.lock().await.pending(42).is_empty());
    assert!(notifications.try_recv().is_err());
    pool.close().await;
}

#[tokio::test]
async fn ignored_and_unselected_earned_rewards_are_claimed_once_and_archived_durably() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| {
        assert_eq!(q["operationName"], "DropsPage_ClaimDropRewards");
        json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}})
    })
    .await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let mut notifications = miner.app.notifications.subscribe();
    let settings = Settings {
        drop_name_blacklist: vec!["reward".into()],
        ..Settings::default()
    };
    miner.campaigns[0].drops[0].claim_id = Some("instance".into());
    miner.campaigns[0].ends_at = Utc::now() - chrono::Duration::hours(1);
    miner.schedule(&settings).await;
    miner.schedule(&settings).await;
    assert_eq!(miner.jobs.len(), 1);
    finish_job(&mut miner, &pool).await;
    let notification = notifications
        .try_recv()
        .expect("durable claim notification");
    let claim = notification.claim.expect("structured claim metadata");
    assert_eq!(claim.account, 42);
    assert_eq!(claim.drop_id, miner.campaigns[0].drops[0].id);
    assert!(miner.campaigns[0].drops[0].claimed);
    miner.publish(&settings).await.unwrap();
    assert!(miner.app.snapshot.read().await.wanted_items.is_empty());
    let history = History::load(dir.path());
    assert_eq!(history.total(), 1);
    assert_eq!(
        history.entries(&HistoryFilter::default())[0].image_url,
        "https://static-cdn.jtvnw.net/hat.png"
    );
    assert!(
        ClaimJournal::load(dir.path())
            .unwrap()
            .pending(42)
            .is_empty()
    );
    assert!(CampaignArchive::load(dir.path()).merge(vec![], Utc::now())[0].finished);
    miner.schedule(&settings).await;
    assert!(miner.jobs.is_empty());
    assert!(notifications.try_recv().is_err());
    pool.close().await;
}

#[tokio::test]
async fn clearing_history_before_claim_receipt_reconciliation_does_not_restore_the_row() {
    let server = MockServer::start().await;
    gql_mock(
        &server,
        |_| json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}),
    )
    .await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].claim_id = Some("instance".into());
    let pending = PendingClaim::new(
        42,
        &miner.campaigns[0],
        &miner.campaigns[0].drops[0],
        &Settings::default(),
    );
    assert!(
        claim(&miner.app, &miner.client, &miner.journal, pending)
            .await
            .unwrap()
    );
    assert_eq!(miner.app.history.lock().await.total(), 1);
    miner.app.history.lock().await.clear().unwrap();
    *miner.app.history.lock().await = History::load(dir.path());
    miner.recover_claims(&HashMap::new()).await.unwrap();
    assert_eq!(History::load(dir.path()).total(), 0);
    assert!(miner.journal.lock().await.pending(42).is_empty());
    assert!(miner.campaigns[0].drops[0].claimed);
    assert!(CampaignArchive::load(dir.path()).merge(vec![], Utc::now())[0].finished);
    pool.close().await;
}

#[tokio::test]
async fn claim_journal_recovers_after_history_write_failure_without_fabricating_claims() {
    let server = MockServer::start().await;
    gql_mock(
        &server,
        |_| json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}),
    )
    .await;
    let (dir, miner, _intent, mut pool) = miner(&server).await;
    let mut campaign = miner.campaigns[0].clone();
    campaign.drops[0].claim_id = Some("instance".into());
    let entry = PendingClaim::new(42, &campaign, &campaign.drops[0], &Settings::default());
    let file = dir.path().join("drop_history.json");
    std::fs::create_dir(&file).unwrap();
    assert_eq!(
        claim(&miner.app, &miner.client, &miner.journal, entry.clone()).await,
        Err(TwitchError::Storage)
    );
    assert_eq!(miner.app.history.lock().await.total(), 0);
    assert_eq!(ClaimJournal::load(dir.path()).unwrap().pending(42).len(), 1);
    std::fs::remove_dir(&file).unwrap();
    let mut miner = miner;
    miner.journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
    miner.recover_claims(&HashMap::new()).await.unwrap();
    assert_eq!(miner.app.history.lock().await.total(), 1);
    miner.campaigns[0].drops[0].mark_claimed(Utc::now());
    miner.recover_claims(&HashMap::new()).await.unwrap();
    miner.recover_claims(&HashMap::new()).await.unwrap();
    assert_eq!(miner.app.history.lock().await.total(), 1);
    assert!(
        ClaimJournal::load(dir.path())
            .unwrap()
            .pending(42)
            .is_empty()
    );
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/gql")
            .count(),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn journal_must_be_durable_before_remote_claim_and_failed_claims_are_not_history() {
    let server = MockServer::start().await;
    gql_mock(
        &server,
        |_| json!({"data":{"claimDropRewards":{"status":"NOT_ELIGIBLE"}}}),
    )
    .await;
    let (dir, miner, _intent, mut pool) = miner(&server).await;
    let mut notifications = miner.app.notifications.subscribe();
    let mut campaign = miner.campaigns[0].clone();
    campaign.drops[0].claim_id = Some("instance".into());
    let entry = PendingClaim::new(42, &campaign, &campaign.drops[0], &Settings::default());
    let file = dir.path().join("pending_claims.json");
    std::fs::create_dir(&file).unwrap();
    assert_eq!(
        claim(&miner.app, &miner.client, &miner.journal, entry.clone()).await,
        Err(TwitchError::Storage)
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/gql")
    );
    std::fs::remove_dir(&file).unwrap();
    assert!(
        !claim(&miner.app, &miner.client, &miner.journal, entry)
            .await
            .unwrap()
    );
    assert_eq!(miner.app.history.lock().await.total(), 0);
    assert!(
        ClaimJournal::load(dir.path())
            .unwrap()
            .pending(42)
            .is_empty()
    );
    assert!(notifications.try_recv().is_err());
    pool.close().await;
}

#[tokio::test]
async fn progress_stays_confirmed_only_with_account_evidence_and_stalls_at_fifteen_estimates() {
    for report in [None, Some(("unknown-drop", 12)), Some(("claimed-drop", 60))] {
        let server = MockServer::start().await;
        let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
        if report == Some(("claimed-drop", 60)) {
            let mut claimed = miner.campaigns[0].drops[0].clone();
            claimed.id = "claimed-drop".into();
            claimed.mark_claimed(Utc::now());
            miner.campaigns[0].drops.insert(0, claimed);
        }
        let target = miner.campaigns[0].drops.len() - 1;
        let settings = select(&mut miner).await;
        let confirmed = miner.campaigns[0].drops[target].confirmed_at;
        for estimate in 1..=15 {
            miner
                .complete(
                    Job::Poll {
                        requested_at: Instant::now(),
                        channel: 10,
                        result: Ok(report.map(|(id, minutes)| (id.into(), minutes))),
                    },
                    &pool,
                )
                .await
                .unwrap();
            let drop = &miner.campaigns[0].drops[target];
            assert_eq!(drop.estimated_minutes, estimate, "report={report:?}");
            assert_eq!(drop.confirmed_minutes, 12, "report={report:?}");
            assert_eq!(drop.confirmed_at, confirmed, "report={report:?}");
            assert!(!drop.claimed, "report={report:?}");
            miner.reselect(&settings).await;
            assert_eq!(
                miner.watching,
                (estimate < 15).then_some(10),
                "report={report:?}, estimate={estimate}"
            );
        }
        miner
            .event(Event::Progress {
                id: "drop-one".into(),
                minutes: 28,
            })
            .await
            .unwrap();
        assert_eq!(miner.campaigns[0].drops[target].estimated_minutes, 0);
        assert_eq!(miner.campaigns[0].drops[target].confirmed_minutes, 28);
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(10), "report={report:?}");
        pool.close().await;
    }
}

#[tokio::test]
async fn public_page_rejection_keeps_the_validated_saved_session() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/streamer"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "PlaybackAccessToken" => json!({"data":{"streamPlaybackAccessToken":null}}),
        "AccountProfile" | "AccountBadges" => json!({"data":null}),
        "Inventory" => {
            let mut campaign = campaign_json("one");
            campaign["allow"] = json!({"isEnabled":true,"channels":[{"id":"10","login":"streamer","displayName":"Streamer"}]});
            json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign],"gameEventDrops":[]}}}})
        },
        "VideoPlayerStreamInfoOverlayChannel" => json!({"data":{"user":{"id":"10","login":"streamer","displayName":"Streamer","stream":{"id":"stream1","viewersCount":100},"broadcastSettings":{"game":{"id":"1","name":"Rust","slug":"rust"}}}}}),
        "DropsHighlightService_AvailableDrops" => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"one"}]}}}),
        other => panic!("unexpected query {other}"),
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    app.settings.write().await.games_to_watch = vec!["Rust".into()];
    session().save(dir.path()).unwrap();
    let mut miner = Miner::new(app.clone(), commands);
    miner.endpoints = Endpoints::mock(&server.uri());
    let worker = tokio::spawn(miner.run());
    let recovered = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if Session::load(dir.path()).unwrap().is_none() {
                break false;
            }
            if app
                .snapshot
                .read()
                .await
                .console
                .iter()
                .any(|line| line.contains("HTTP 403"))
            {
                break true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(recovered, Ok(true));
    assert_eq!(
        Session::load(dir.path()).unwrap().unwrap().access_token,
        session().access_token
    );
}

#[tokio::test]
async fn successful_inventory_refresh_recovers_stalled_public_and_retained_rewards() {
    for retained in [false, true] {
        for confirmed in [false, true] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/track"))
                .respond_with(ResponseTemplate::new(204))
                .expect(1)
                .mount(&server)
                .await;
            let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
            let settings = select(&mut miner).await;
            if !confirmed {
                let drop = &mut miner.campaigns[0].drops[0];
                drop.confirmed_minutes = 0;
                drop.confirmed_at = None;
            }
            let before = miner.campaigns[0].drops[0].clone();
            for _ in 0..MAX_ESTIMATED_MINUTES {
                miner
                    .complete(
                        Job::Poll {
                            channel: 10,
                            requested_at: Instant::now(),
                            result: Ok(None),
                        },
                        &pool,
                    )
                    .await
                    .unwrap();
            }
            miner.reselect(&settings).await;
            assert!(miner.watching.is_none());
            miner
                .complete(
                    Job::Inventory {
                        result: Err(TwitchError::Network),
                        requested_at: Utc::now(),
                        refresh_sequence: 0,
                    },
                    &pool,
                )
                .await
                .unwrap();
            assert_eq!(
                miner.campaigns[0].drops[0].estimated_minutes,
                MAX_ESTIMATED_MINUTES
            );
            let mut raw = campaign_json("one");
            raw["timeBasedDrops"][0]["self"] = serde_json::Value::Null;
            let fresh = Campaign::parse(&raw, &HashMap::new(), Utc::now()).unwrap();
            miner
                .complete(
                    Job::Inventory {
                        result: Ok(Inventory {
                            rejected_account_ids: HashSet::new(),
                            campaigns: if retained { vec![] } else { vec![fresh] },
                            awards: HashMap::new(),
                            status: InventoryStatus {
                                available: !retained,
                                ..InventoryStatus::default()
                            },
                        }),
                        requested_at: Utc::now(),
                        refresh_sequence: 0,
                    },
                    &pool,
                )
                .await
                .unwrap();
            let drop = &miner.campaigns[0].drops[0];
            assert_eq!(drop.estimated_minutes, 0);
            assert_eq!(drop.confirmed_minutes, before.confirmed_minutes);
            assert_eq!(drop.confirmed_at, before.confirmed_at);
            assert!(!drop.claimed);
            miner.reselect(&settings).await;
            assert_eq!(miner.watching, Some(10));
            miner.channels[0].beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
            miner.schedule(&settings).await;
            finish_job(&mut miner, &pool).await;
            assert!(
                miner.poll_at.is_some(),
                "watching did not resume after recovery"
            );
            pool.close().await;
        }
    }
}

#[tokio::test]
async fn conflicting_live_progress_uses_inventory_and_keeps_watching_until_reconciled() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.confirm("drop-one", 42, &settings);
    let requested_at = Utc::now();
    let mut record = campaign_json("one");
    record["timeBasedDrops"][0]["self"]["currentMinutesWatched"] = json!(23);
    let campaign = Campaign::parse(&record, &HashMap::new(), Utc::now()).unwrap();
    let confirmed_at = campaign.drops[0].confirmed_at;
    miner
        .complete(
            Job::Inventory {
                requested_at,
                refresh_sequence: 0,
                result: Ok(Inventory {
                    campaigns: vec![campaign],
                    status: InventoryStatus {
                        available: true,
                        ..Default::default()
                    },
                    awards: HashMap::new(),
                    rejected_account_ids: HashSet::new(),
                }),
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 23);
    // Both live sources must remain provisional, including reported completion.
    miner
        .event(Event::Progress {
            id: "drop-one".into(),
            minutes: 43,
        })
        .await
        .unwrap();
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at: Instant::now(),
                result: Ok(Some(("drop-one".into(), 60))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    miner.publish(&settings).await.unwrap();
    let snapshot = miner.app.snapshot.read().await;
    let progress = snapshot.current_drop.as_ref().unwrap();
    assert_eq!(progress.confirmed_minutes, 23);
    assert_eq!(progress.confirmed_at, confirmed_at);
    assert_eq!(snapshot.campaigns[0].drops[0].confirmed_minutes, 23);
    assert_eq!(miner.watching, Some(10));
    assert_eq!(History::load(dir.path()).total(), 0);
    drop(snapshot);
    // Missing live reports must not accumulate enough estimates to stop this reward.
    for _ in 0..MAX_ESTIMATED_MINUTES + 1 {
        assert!(!miner.campaigns[0].bump_estimates(&settings, Utc::now()));
    }
    assert!(miner.campaigns[0].can_watch(&miner.channels[0], &settings, Utc::now()));
    // Only account completion resolves the disagreement; it is still not a claim.
    record["timeBasedDrops"][0]["self"]["currentMinutesWatched"] = json!(60);
    miner
        .complete(
            Job::Inventory {
                requested_at: Utc::now(),
                refresh_sequence: 0,
                result: Ok(Inventory {
                    campaigns: vec![Campaign::parse(&record, &HashMap::new(), Utc::now()).unwrap()],
                    status: InventoryStatus {
                        available: true,
                        ..Default::default()
                    },
                    awards: HashMap::new(),
                    rejected_account_ids: HashSet::new(),
                }),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 60);
    assert!(miner.watching.is_none());
    assert!(!miner.campaigns[0].drops[0].claimed);
    assert_eq!(History::load(dir.path()).total(), 0);
    pool.close().await;
}

#[tokio::test]
async fn disputed_progress_survives_partial_refresh_and_same_account_renewal() {
    let server = MockServer::start().await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let drop = &mut miner.campaigns[0].drops[0];
    drop.confirm(23, Utc::now());
    drop.reported_minutes = Some(42);
    drop.progress_disputed = true;
    let inventory = |campaigns, available| Job::Inventory {
        requested_at: Utc::now(),
        refresh_sequence: 0,
        result: Ok(Inventory {
            campaigns,
            status: InventoryStatus {
                available,
                ..Default::default()
            },
            awards: HashMap::new(),
            rejected_account_ids: HashSet::new(),
        }),
    };
    miner
        .complete(inventory(vec![], false), &pool)
        .await
        .unwrap();
    assert!(miner.campaigns[0].drops[0].progress_disputed);
    miner.confirm("drop-one", 43, &settings);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 23);
    // A public catalog record also supplies no new account evidence.
    let mut record = campaign_json("one");
    record["timeBasedDrops"][0]["self"] = serde_json::Value::Null;
    let public = Campaign::parse(&record, &HashMap::new(), Utc::now()).unwrap();
    miner
        .complete(inventory(vec![public], true), &pool)
        .await
        .unwrap();
    assert!(miner.campaigns[0].drops[0].progress_disputed);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 23);

    let saved = miner.resume();
    miner.campaigns.clear();
    miner.restore(&saved);
    assert!(miner.campaigns[0].drops[0].progress_disputed);
    miner.confirm("drop-one", 60, &settings);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 23);
    let mut other_account = miner.resume();
    other_account.user_id = Some(miner.client.user_id + 1);
    miner.campaigns.clear();
    miner.restore(&other_account);
    assert!(miner.campaigns.is_empty());
    miner.restore(&saved);
    // Neither a different campaign nor a replaced reward inherits a live counter.
    let other = Campaign::parse(&campaign_json("other"), &HashMap::new(), Utc::now()).unwrap();
    miner
        .complete(inventory(vec![other], true), &pool)
        .await
        .unwrap();
    assert!(!miner.campaigns[0].drops[0].progress_disputed);
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 12);

    miner.campaigns.clear();
    miner.restore(&saved);
    intent.send_modify(|intent| intent.clear += 1);
    miner.apply_intent(&pool).await;
    assert!(miner.resume().disputed_campaigns.is_empty());
    pool.close().await;
}

#[tokio::test(start_paused = true)]
async fn disputed_progress_retries_once_per_minute_without_claim_status_or_estimates() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.status.checked_at = Some(Utc::now());
    let drop = &mut miner.campaigns[0].drops[0];
    drop.confirm(23, Utc::now());
    drop.reported_minutes = Some(42);
    drop.progress_disputed = true;
    miner.next_watch = Instant::now() + Duration::from_secs(600);
    miner.next_progress_refresh = Instant::now() + Duration::from_secs(60);
    miner.busy.insert(JobKind::Inventory);
    // Channel loss should show waiting for a channel, not waiting for a claim.
    miner.event(Event::Offline(10)).await.unwrap();
    miner.reselect(&settings).await;
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner.app.snapshot.read().await.mining.state,
        crate::dto::MiningState::WaitingChannel
    );
    tokio::time::advance(Duration::from_secs(59)).await;
    miner.schedule(&settings).await;
    assert!(!miner.refresh);
    tokio::time::advance(Duration::from_secs(1)).await;
    miner.schedule(&settings).await;
    assert!(miner.refresh);
    miner.refresh = false;
    miner
        .complete(
            Job::Inventory {
                requested_at: Utc::now(),
                refresh_sequence: 0,
                result: Err(TwitchError::Network),
            },
            &pool,
        )
        .await
        .unwrap();
    for _ in 0..5 {
        miner.confirm("drop-one", 60, &settings);
        miner.schedule(&settings).await;
        assert!(!miner.refresh);
    }
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 23);
    assert!(miner.campaigns[0].drops[0].progress_disputed);
    tokio::time::advance(Duration::from_secs(60)).await;
    miner.schedule(&settings).await;
    assert!(miner.refresh);
    miner.refresh = false;
    miner.campaigns[0].ends_at = Utc::now() - chrono::Duration::hours(24);
    tokio::time::advance(Duration::from_secs(60)).await;
    miner.schedule(&settings).await;
    assert!(
        !miner.refresh,
        "expired disputes must stop reconciling at the claim deadline"
    );
    pool.close().await;
}

#[tokio::test]
async fn inventory_refresh_cannot_replace_progress_confirmed_after_the_request_started() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let before = Utc::now() - chrono::Duration::seconds(1);
    miner.confirm("drop-one", 31, &Settings::default());
    miner.campaigns[0].drops[0].estimated_minutes = MAX_ESTIMATED_MINUTES;
    let inventory = Inventory {
        rejected_account_ids: HashSet::new(),
        awards: HashMap::new(),
        campaigns: vec![
            Campaign::parse(&campaign_json("one"), &HashMap::new(), Utc::now()).unwrap(),
        ],
        status: InventoryStatus {
            available: true,
            ..InventoryStatus::default()
        },
    };
    miner
        .complete(
            Job::Inventory {
                refresh_sequence: 0,
                result: Ok(inventory),
                requested_at: before,
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(miner.campaigns[0].drops[0].confirmed_minutes, 31);
    assert_eq!(
        miner.campaigns[0].drops[0].estimated_minutes,
        MAX_ESTIMATED_MINUTES
    );
    miner.publish(&Settings::default()).await.unwrap();
    assert_eq!(
        miner.app.snapshot.read().await.campaigns[0].drops[0].confirmed_minutes,
        31
    );
    pool.close().await;
}

#[tokio::test]
async fn manual_offline_waits_and_cache_clear_and_refresh_setting_preserve_user_data() {
    let server = MockServer::start().await;
    let (dir, mut miner, intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    session().save(dir.path()).unwrap();
    miner.app.data.save_settings(&settings).unwrap();
    let mut alternate = miner.channels[0].clone();
    alternate.identity.id = 11;
    miner.channels.push(alternate);
    intent.send_modify(|v| {
        v.selected = Some(10);
        v.manual_revision += 1;
    });
    miner.apply_intent(&pool).await;
    assert_eq!(miner.manual, Some(ManualSelection::new(10, None)));
    miner.event(Event::Offline(10)).await.unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, None);
    assert!(miner.manual.is_some());
    miner.event(Event::Changed(10)).await.unwrap();
    assert!(miner.refresh_channels[&10] <= Instant::now() + Duration::from_secs(2));
    miner.campaigns[0].drops[0].ends_at = Utc::now() + chrono::Duration::seconds(10);
    miner.set_transition();
    assert!(miner.next_transition.unwrap() <= Utc::now() + chrono::Duration::seconds(10));
    miner
        .app
        .settings
        .write()
        .await
        .minimum_refresh_interval_minutes = 1;
    intent.send_modify(|v| v.settings += 1);
    miner.apply_intent(&pool).await;
    assert_eq!(
        miner.next_refresh,
        miner.last_inventory + Duration::from_secs(60)
    );
    let epoch = miner.epoch;
    intent.send_modify(|v| {
        v.clear += 1;
        v.refresh += 1
    });
    miner.apply_intent(&pool).await;
    assert!(miner.refresh);
    assert!(miner.channels.is_empty());
    assert!(miner.campaigns.is_empty());
    assert!(miner.manual.is_none());
    assert_ne!(miner.epoch, epoch);
    assert_eq!(Session::load(dir.path()).unwrap().unwrap().user_id, 42);
    assert_eq!(
        miner.app.data.settings().unwrap().games_to_watch,
        vec!["Rust"]
    );
    pool.close().await;
}

#[tokio::test]
async fn published_channels_use_mining_eligibility_including_real_special_acls() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let mut unrelated = miner.channels[0].clone();
    unrelated.identity.id = 11;
    unrelated.game.as_mut().unwrap().id = 2;
    unrelated.game.as_mut().unwrap().name = "Other game".into();
    unrelated.acl_based = true;
    miner.channels.push(unrelated.clone());
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .channels
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![10]
    );
    miner.publish(&Settings::default()).await.unwrap();
    assert!(miner.app.snapshot.read().await.channels.is_empty());
    let mut special = miner.campaigns[0].clone();
    special.id = "special".into();
    special.game.id = 509663;
    special.game.name = "Special Events".into();
    special.allowed_channels = vec![unrelated.identity];
    miner.campaigns.push(special);
    let settings = Settings {
        games_to_watch: vec!["Rust".into(), "Special Events".into()],
        ..settings
    };
    miner.publish(&settings).await.unwrap();
    assert_eq!(miner.app.snapshot.read().await.channels.len(), 2);
    miner.event(Event::Changed(11)).await.unwrap();
    miner.publish(&settings).await.unwrap();
    assert_eq!(miner.app.snapshot.read().await.channels.len(), 1);
    pool.close().await;
}

#[tokio::test]
async fn channel_changes_refresh_during_slow_inventory_and_viewers_only_update_the_channel() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "VideoPlayerStreamInfoOverlayChannel" => json!({"data":{"user":{"stream":null}}}),
        "DropsHighlightService_AvailableDrops" => {
            json!({"data":{"channel":{"viewerDropCampaigns":[]}}})
        }
        other => panic!("unexpected operation {other}"),
    })
    .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.publish(&settings).await.unwrap();
    miner.publish = false;
    miner
        .event(Event::Viewers { id: 10, count: 456 })
        .await
        .unwrap();
    assert!(!miner.publish);
    assert_eq!(
        miner.app.snapshot.read().await.channels[0].viewers,
        Some(456)
    );
    miner.event(Event::Changed(10)).await.unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, None);
    miner.refresh_channels.insert(10, Instant::now());
    miner.busy.insert(JobKind::Inventory);
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Update));
    finish_job(&mut miner, &pool).await;
    assert!(!miner.channels[0].online());
    pool.close().await;
}

fn external_channel(miner: &Mining) -> Channel {
    let mut channel = miner.channels[0].clone();
    channel.identity = ChannelIdentity {
        id: 999,
        login: "extra_streamer".into(),
        name: "Extra Streamer".into(),
    };
    channel
}

#[tokio::test]
async fn manual_watch_does_not_wait_for_or_write_settings() {
    let server = MockServer::start().await;
    let (dir, mut miner, intent, mut pool) = miner(&server).await;
    intent.send_modify(|v| {
        v.manual_revision = 1;
        v.channel_login = Some("extra_streamer".into());
    });
    miner.apply_intent(&pool).await;
    let slot = miner.app.settings_slot.clone();
    let _permit = slot.acquire().await.unwrap();
    let resolved = external_channel(&miner);
    tokio::time::timeout(
        Duration::from_secs(1),
        miner.complete(
            Job::Manual {
                revision: 1,
                requested_at: Instant::now(),
                result: Box::new(Ok(Some(resolved))),
            },
            &pool,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(miner.manual.is_some());
    assert!(!dir.path().join("settings.json").exists());
    assert!(miner.app.data.settings().unwrap().games_to_watch.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn renewal_keeps_pending_requests_and_restores_confirmed_channel_after_failed_replacement() {
    let server = MockServer::start().await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    intent.send_modify(|v| {
        v.manual_revision = 1;
        v.channel_login = Some("extra_streamer".into());
    });
    miner.apply_intent(&pool).await;
    let pending = miner.resume();
    let (_other_dir, mut renewed, _other_intent, mut other_pool) = self::miner(&server).await;
    renewed.restore(&pending);
    assert_eq!(renewed.lookup, Some(("extra_streamer".into(), 1)));
    assert_eq!(renewed.manual_pending.as_deref(), Some("extra_streamer"));
    let resolved = external_channel(&miner);
    miner
        .complete(
            Job::Manual {
                revision: 1,
                requested_at: Instant::now(),
                result: Box::new(Ok(Some(resolved))),
            },
            &pool,
        )
        .await
        .unwrap();
    intent.send_modify(|v| {
        v.manual_revision = 2;
        v.channel_login = Some("missing".into());
    });
    miner.apply_intent(&pool).await;
    miner
        .complete(
            Job::Manual {
                revision: 2,
                requested_at: Instant::now(),
                result: Box::new(Ok(None)),
            },
            &pool,
        )
        .await
        .unwrap();
    let saved = miner.resume();
    assert_eq!(
        saved.channel.as_ref().unwrap().identity.login,
        "extra_streamer"
    );
    assert!(saved.lookup.is_none());
    renewed.channels.clear();
    renewed.restore(&saved);
    renewed.intent = intent.subscribe();
    renewed.app = miner.app.clone();
    renewed.channels_loaded = false;
    assert!(!renewed.channels[0].online());
    assert_eq!(renewed.manual_pending, None);
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "VideoPlayerStreamInfoOverlayChannel" => {
            assert_eq!(q["variables"]["channel"], "extra_streamer");
            json!({"data":{"user":{"id":"999","displayName":"Extra Streamer","stream":{"id":"fresh"},"broadcastSettings":{"game":{"id":"1","name":"Rust"}}}}})
        },
        "DropsHighlightService_AvailableDrops" => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"one"}]}}}),
        "DirectoryPage_Game" => json!({"data":{"game":{"streams":{"edges":[]}}}}),
        other => panic!("unexpected operation {other}"),
    }).await;
    renewed.channels_dirty = true;
    let settings = renewed.app.settings.read().await.clone();
    renewed.schedule(&settings).await;
    finish_job(&mut renewed, &other_pool).await;
    renewed.reselect(&settings).await;
    assert_eq!(renewed.manual, Some(ManualSelection::new(999, None)));
    assert_eq!(renewed.watching, Some(999));
    pool.close().await;
    other_pool.close().await;
}

#[tokio::test]
async fn manual_mode_waits_for_fresh_stream_but_lookup_never_waits_for_inventory() {
    let server = MockServer::start().await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    miner.manual = Some(ManualSelection::new(10, None));
    let fresh = miner.channels[0].clone();
    miner.event(Event::Changed(10)).await.unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.manual, Some(ManualSelection::new(10, None)));
    assert_eq!(miner.watching, None);
    miner.refresh_channels.insert(10, Instant::now());
    miner
        .complete(
            Job::Update {
                result: Err(TwitchError::Network),
                requested_at: Instant::now(),
            },
            &pool,
        )
        .await
        .unwrap();
    assert!(miner.refresh_channels.contains_key(&10));
    miner.reselect(&settings).await;
    assert_eq!(miner.manual, Some(ManualSelection::new(10, None)));
    miner
        .complete(
            Job::Update {
                result: Ok(vec![fresh]),
                requested_at: Instant::now(),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.manual, Some(ManualSelection::new(10, None)));
    assert_eq!(miner.watching, Some(10));
    miner.campaigns.clear();
    miner.watching = None;
    intent.send_modify(|v| {
        v.manual_revision += 1;
        v.channel_login = Some("extra_streamer".into());
    });
    miner.apply_intent(&pool).await;
    gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
        "VideoPlayerStreamInfoOverlayChannel" => json!({"data":{"user":{"id":"999","displayName":"Extra Streamer","stream":{"id":"live"},"broadcastSettings":{"game":{"id":"1","name":"Rust"}}}}}),
        "DropsHighlightService_AvailableDrops" => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"one"}]}}}),
        other => panic!("unexpected operation {other}"),
    }).await;
    miner.channels_dirty = false;
    miner.busy.insert(JobKind::Inventory); // A subsequent scan must not delay the lookup.
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Manual));
    finish_job(&mut miner, &pool).await;
    miner.reselect(&settings).await;
    assert_eq!(miner.manual, Some(ManualSelection::new(999, None)));
    assert_eq!(miner.watching, Some(999));
    assert!(miner.manual_error.is_none());
    pool.close().await;
}

#[tokio::test]
async fn manual_channel_preserves_settings_and_survives_catalog_rebuilds() {
    let server = MockServer::start().await;
    let (dir, mut miner, intent, mut pool) = miner(&server).await;
    miner.app.settings.write().await.games_to_watch = vec!["Other game".into()];
    miner
        .app
        .settings
        .write()
        .await
        .minimum_refresh_interval_minutes = 17;
    let inventory = miner.campaigns.clone();
    let requested_at = Instant::now();
    intent.send_modify(|v| {
        v.manual_revision += 1;
        v.channel_login = Some("extra_streamer".into());
    });
    miner.apply_intent(&pool).await;
    let resolved = external_channel(&miner);
    miner
        .complete(
            Job::Manual {
                revision: 1,
                requested_at,
                result: Box::new(Ok(Some(resolved))),
            },
            &pool,
        )
        .await
        .unwrap();
    let settings = miner.app.settings.read().await.clone();
    assert_eq!(settings.games_to_watch, ["Other game"]);
    assert_eq!(settings.minimum_refresh_interval_minutes, 17);
    assert!(!dir.path().join("settings.json").exists());
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(999));
    miner
        .complete(
            Job::Channels {
                requested_at,
                result: Ok(vec![]),
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(miner.channels[0].identity.id, 999);
    miner
        .complete(
            Job::Inventory {
                refresh_sequence: 0,
                requested_at: Utc::now(),
                result: Ok(Inventory {
                    rejected_account_ids: HashSet::new(),
                    campaigns: inventory,
                    awards: HashMap::new(),
                    status: InventoryStatus::default(),
                }),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(999));
    miner.publish(&settings).await.unwrap();
    assert!(miner.app.snapshot.read().await.manual_mode.active);
    pool.close().await;
}

#[tokio::test]
async fn manual_lookup_rejects_offline_and_stale_results_but_does_not_require_eligible_rewards() {
    let server = MockServer::start().await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    let requested_at = Instant::now();
    let mut offline = external_channel(&miner);
    offline.broadcast_id = None;
    assert_eq!(
        miner.finish_manual(Ok(Some(offline)), requested_at, None),
        Some("gui.channels.offline")
    );
    intent.send_modify(|v| v.manual_revision = 2);
    let resolved = external_channel(&miner);
    miner
        .complete(
            Job::Manual {
                revision: 1,
                requested_at,
                result: Box::new(Ok(Some(resolved))),
            },
            &pool,
        )
        .await
        .unwrap();
    assert!(miner.manual.is_none());
    assert_eq!(miner.channels.len(), 1);
    let resolved = miner.channels[0].clone();
    miner.event(Event::Offline(10)).await.unwrap();
    assert_eq!(
        miner.finish_manual(Ok(Some(resolved)), requested_at, None),
        Some("gui.channels.offline")
    );
    for restricted in [false, true] {
        miner.campaigns[0].allowed_channels = if restricted {
            vec![miner.channels[0].identity.clone()]
        } else {
            vec![]
        };
        let settings = Settings {
            drop_name_blacklist: vec!["reward".into()],
            ..Settings::default()
        };
        let mut resolved = external_channel(&miner);
        resolved.broadcast_id = Some("manual-live".into());
        resolved.game = None;
        resolved.drops_enabled = false;
        assert_eq!(
            miner.finish_manual(Ok(Some(resolved)), Instant::now(), None),
            None
        );
        miner.reselect(&settings).await;
        assert_eq!(miner.watching, Some(999));
        miner.publish(&settings).await.unwrap();
        assert!(miner.app.snapshot.read().await.current_drop.is_none());
    }
    assert!(miner.app.data.settings().unwrap().games_to_watch.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn manual_unknown_stream_sends_beacons_without_catalog_or_selection_and_stops_on_exit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/extra_streamer"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"beacon_url":"{}/track"}}"#, server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/track"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    let mut resolved = external_channel(&miner);
    resolved.drops_enabled = false;
    resolved.game = None;
    miner.campaigns.clear();
    intent.send_modify(|v| {
        v.manual_revision = 1;
        v.channel_login = Some("extra_streamer".into());
    });
    miner.apply_intent(&pool).await;
    miner
        .complete(
            Job::Manual {
                revision: 1,
                requested_at: Instant::now(),
                result: Box::new(Ok(Some(resolved))),
            },
            &pool,
        )
        .await
        .unwrap();
    let settings = Settings::default();
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(999));
    miner.publish(&settings).await.unwrap();
    {
        let state = miner.app.snapshot.read().await;
        assert_eq!(state.channels.len(), 1);
        assert!(state.channels[0].watching);
        assert!(state.current_drop.is_none());
        assert!(state.manual_mode.expires_at.is_none());
    }
    miner.schedule(&settings).await;
    assert!(miner.busy.contains(&JobKind::Watch));
    finish_job(&mut miner, &pool).await;
    assert!(miner.next_watch > Instant::now() + Duration::from_secs(58));
    intent.send_modify(|v| {
        v.manual_revision += 1;
        v.channel_login = None;
    });
    miner.apply_intent(&pool).await;
    miner.reselect(&settings).await;
    assert!(miner.watching.is_none());
    assert!(miner.manual.is_none());
    miner.channels_dirty = false;
    miner.next_watch = Instant::now();
    miner.schedule(&settings).await;
    assert!(miner.jobs.is_empty());
    assert!(miner.app.data.settings().unwrap().games_to_watch.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn manual_timer_survives_renewal_and_expiry_restores_auto_without_replaying_the_request() {
    let server = MockServer::start().await;
    let (_dir, mut miner, intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    tokio::time::pause();
    let mut resolved = external_channel(&miner);
    resolved.game = None;
    resolved.drops_enabled = false;
    intent.send_modify(|v| {
        v.manual_revision = 1;
        v.channel_login = Some("extra_streamer".into());
        v.manual_duration = Some(Duration::from_secs(60));
    });
    miner.apply_intent(&pool).await;
    miner
        .complete(
            Job::Manual {
                revision: 1,
                requested_at: Instant::now(),
                result: Box::new(Ok(Some(resolved.clone()))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(999));
    let deadline = miner.manual.unwrap().expires_at.unwrap();
    tokio::time::advance(Duration::from_secs(30)).await;
    let saved = miner.resume();
    assert_eq!(saved.manual.unwrap().expires_at, Some(deadline));
    miner.channels.clear();
    miner.restore(&saved);
    assert!(!miner.channels[0].online());
    assert_eq!(miner.manual.unwrap().expires_at, Some(deadline));
    miner
        .complete(
            Job::Update {
                requested_at: Instant::now(),
                result: Ok(vec![resolved.clone()]),
            },
            &pool,
        )
        .await
        .unwrap();
    let mut automatic = resolved;
    automatic.identity.id = 10;
    automatic.game = Some(miner.campaigns[0].game.clone());
    automatic.drops_enabled = true;
    miner.channels.push(automatic);
    tokio::time::advance(Duration::from_secs(29)).await;
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(999));
    tokio::time::advance(Duration::from_secs(1)).await;
    miner.reselect(&settings).await;
    assert!(miner.manual.is_none());
    assert_eq!(miner.watching, Some(10));
    let expired = miner.resume();
    miner.restore(&expired);
    miner.apply_intent(&pool).await;
    assert!(miner.manual.is_none());
    assert!(miner.lookup.is_none());
    pool.close().await;
}

#[tokio::test]
async fn cancelled_timed_lookup_is_either_pending_or_consumed_never_both() {
    let server = MockServer::start().await;
    for cancel_before in [true, false] {
        let (_dir, mut miner, intent, mut pool) = miner(&server).await;
        intent.send_modify(|v| {
            v.manual_revision = 1;
            v.channel_login = Some("extra_streamer".into());
            v.manual_duration = Some(Duration::from_secs(60));
        });
        miner.apply_intent(&pool).await;
        let resolved = external_channel(&miner);
        let app = miner.app.clone();
        let held = app.settings.write().await;
        let cancelled = miner.client.http.cancel.clone();
        {
            let complete = miner.complete(
                Job::Manual {
                    revision: 1,
                    requested_at: Instant::now(),
                    result: Box::new(Ok(Some(resolved))),
                },
                &pool,
            );
            tokio::pin!(complete);
            tokio::select! {
                _ = &mut complete => panic!("settings read should be blocked"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if cancel_before {
                cancelled.cancel();
            }
            drop(held);
            let result = complete.await;
            assert_eq!(
                result,
                if cancel_before {
                    Err(TwitchError::Cancelled)
                } else {
                    Ok(())
                }
            );
            cancelled.cancel();
        }
        let saved = miner.resume();
        if cancel_before {
            assert!(saved.manual.is_none());
            assert_eq!(saved.lookup, Some(("extra_streamer".into(), 1)));
        } else {
            assert!(saved.manual.unwrap().expires_at.is_some());
            assert!(
                saved.lookup.is_none(),
                "a consumed timed request must not restart its timer"
            );
            miner.restore(&saved);
            assert_eq!(
                miner.manual.unwrap().expires_at,
                saved.manual.unwrap().expires_at
            );
            assert!(miner.lookup.is_none());
        }
        pool.close().await;
    }
}

#[tokio::test]
async fn manual_progress_requires_twitch_evidence_and_never_invents_estimates() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = Settings::default();
    miner.manual = Some(ManualSelection::new(10, None));
    miner.reselect(&settings).await;
    miner.publish(&settings).await.unwrap();
    assert!(miner.app.snapshot.read().await.current_drop.is_none());
    let estimate = miner.campaigns[0].drops[0].estimated_minutes;
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at: Instant::now(),
                result: Ok(None),
            },
            &pool,
        )
        .await
        .unwrap();
    assert_eq!(miner.campaigns[0].drops[0].estimated_minutes, estimate);
    miner
        .complete(
            Job::Poll {
                channel: 10,
                requested_at: Instant::now(),
                result: Ok(Some(("drop-one".into(), 24))),
            },
            &pool,
        )
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    assert_eq!(
        miner
            .app
            .snapshot
            .read()
            .await
            .current_drop
            .as_ref()
            .unwrap()
            .confirmed_minutes,
        24
    );
    assert!(miner.app.data.settings().unwrap().games_to_watch.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn completed_archives_and_subscription_only_campaigns_never_become_live_mining_inventory() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].mark_claimed(Utc::now());
    miner.publish(&Settings::default()).await.unwrap();
    miner.campaigns.clear();
    let mut subscription = campaign_json("sub");
    subscription["timeBasedDrops"][0]["requiredMinutesWatched"] = 0.into();
    miner
        .campaigns
        .push(Campaign::parse(&subscription, &HashMap::new(), Utc::now()).unwrap());
    let settings = select(&mut miner).await;
    miner.publish(&settings).await.unwrap();
    let state = miner.app.snapshot.read().await;
    assert_eq!(state.campaigns.len(), 1);
    assert!(state.campaigns[0].finished);
    assert!(state.wanted_items.is_empty());
    assert!(state.current_drop.is_none());
    assert!(miner.watching.is_none());
    pool.close().await;
}

#[tokio::test]
async fn concurrent_logout_and_shutdown_drain_owned_work_before_removing_only_twitch_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    session().save(dir.path()).unwrap();
    std::fs::write(dir.path().join("cookies.jar"), b"rollback copy").unwrap();
    let settings = Settings::default()
        .patched(&json!({"games_to_watch":["Rust"]}))
        .unwrap();
    app.data.save_settings(&settings).unwrap();
    let campaign = Campaign::parse(&campaign_json("history"), &HashMap::new(), Utc::now()).unwrap();
    app.history
        .lock()
        .await
        .record(campaign.history_entry(&campaign.drops[0], Utc::now()))
        .unwrap();
    let mut owner = Miner::new(app.clone(), commands);
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    let complete = Arc::new(Notify::new());
    let released = complete.clone();
    let (drained, drained_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        stopped.cancelled().await;
        released.notified().await;
        let _ = drained.send(());
        Ok(())
    });
    let generation = Generation {
        cancel,
        confirmed: Arc::new(Notify::new()),
        task,
    };
    let (first, first_result) = oneshot::channel();
    let logout = tokio::spawn(async move {
        owner.logout(generation, first).await;
    });
    let second_app = app.clone();
    let second = tokio::spawn(async move { second_app.command(Command::Logout).await });
    let shutdown_app = app.clone();
    let shutdown = tokio::spawn(async move { shutdown_app.command(Command::Shutdown).await });
    tokio::time::timeout(Duration::from_secs(3), app.shutdown.cancelled())
        .await
        .unwrap();
    assert!(Session::load(dir.path()).unwrap().is_some());
    complete.notify_one();
    drained_rx.await.unwrap();
    first_result.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    shutdown.await.unwrap().unwrap();
    logout.await.unwrap();
    assert!(Session::load(dir.path()).unwrap().is_none());
    assert_eq!(
        std::fs::read(dir.path().join("cookies.jar")).unwrap(),
        b"rollback copy"
    );
    assert_eq!(History::load(dir.path()).total(), 1);
    assert_eq!(app.data.settings().unwrap().games_to_watch, vec!["Rust"]);
}

#[tokio::test]
async fn full_owner_authenticates_saved_session_and_logout_cancels_inventory_before_new_device_flow()
 {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tv"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/oauth2/device")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code":"private","user_code":"NEWCODE","verification_uri":"https://www.twitch.tv/activate","interval":1,"expires_in":60}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/gql"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(30))
                .set_body_json(json!({"data":{"currentUser":{"inventory":{}}}})),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    session().save(dir.path()).unwrap();
    let mut miner = Miner::new(app.clone(), commands);
    miner.endpoints = Endpoints::mock(&server.uri());
    let worker = tokio::spawn(miner.run());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/gql")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), app.command(Command::Logout))
        .await
        .unwrap()
        .unwrap();
    assert!(Session::load(dir.path()).unwrap().is_none());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app
                .snapshot
                .read()
                .await
                .login
                .oauth_pending
                .as_ref()
                .is_some_and(|v| v.code == "NEWCODE")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(app.snapshot.read().await.campaigns.is_empty());
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(Session::load(dir.path()).unwrap().is_none());
}

#[tokio::test]
async fn claim_ready_event_survives_an_older_inventory_request() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].confirmed_at = Some(Utc::now() - chrono::Duration::seconds(2));
    let requested_at = Utc::now() - chrono::Duration::seconds(1);
    miner
        .event(Event::Claim {
            id: "drop-one".into(),
            instance: "ready-instance".into(),
        })
        .await
        .unwrap();
    let inventory = Inventory {
        rejected_account_ids: HashSet::new(),
        awards: HashMap::new(),
        campaigns: vec![
            Campaign::parse(&campaign_json("one"), &HashMap::new(), Utc::now()).unwrap(),
        ],
        status: InventoryStatus::default(),
    };
    miner
        .complete(
            Job::Inventory {
                refresh_sequence: 0,
                result: Ok(inventory),
                requested_at,
            },
            &pool,
        )
        .await
        .unwrap();
    let instance = miner.campaigns[0].drops[0].claim_id.clone();
    pool.close().await;
    assert_eq!(
        instance.as_deref(),
        Some("ready-instance"),
        "late inventory erased the already received account claim ID"
    );
}

#[tokio::test]
async fn late_poll_cannot_erase_newer_pubsub_progress() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let requested_at = Instant::now();
    miner.confirm("drop-one", 31, &settings);
    miner
        .complete(
            Job::Poll {
                requested_at,
                channel: 10,
                result: Ok(Some(("drop-one".into(), 12))),
            },
            &pool,
        )
        .await
        .unwrap();
    let minutes = miner.campaigns[0].drops[0].confirmed_minutes;
    pool.close().await;
    assert_eq!(
        minutes, 31,
        "poll begun before PubSub confirmation must not replace it"
    );
}

#[tokio::test]
async fn late_channel_update_cannot_resurrect_offline_stream() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let stale = miner.channels.clone();
    miner.event(Event::Offline(10)).await.unwrap();
    miner
        .complete(
            Job::Update {
                result: Ok(stale),
                requested_at: Instant::now() - Duration::from_secs(1),
            },
            &pool,
        )
        .await
        .unwrap();
    let online = miner.channels[0].online();
    pool.close().await;
    assert!(
        !online,
        "inflight channel query replaced a newer stream-down event"
    );
}

#[tokio::test]
async fn claimed_reward_stays_complete_after_delayed_progress_event() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].mark_claimed(Utc::now());
    miner
        .event(Event::Progress {
            id: "drop-one".into(),
            minutes: 12,
        })
        .await
        .unwrap();
    let minutes = miner.campaigns[0].drops[0].confirmed_minutes;
    let claimed = miner.campaigns[0].drops[0].claimed;
    pool.close().await;
    assert!(claimed);
    assert_eq!(minutes, 60, "claimed reward now shows partial progress");
}

#[tokio::test]
async fn estimate_ceiling_fetches_inventory_and_resumes_watching() {
    let server = MockServer::start().await;
    gql_mock(&server, |query| {
        assert_eq!(query["operationName"], "Inventory");
        json!({"data":{"currentUser":{"inventory":{
            "dropCampaignsInProgress":[campaign_json("one")], "gameEventDrops":[]
        }}}})
    })
    .await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    for _ in 0..MAX_ESTIMATED_MINUTES {
        miner
            .complete(
                Job::Poll {
                    requested_at: Instant::now(),
                    channel: 10,
                    result: Ok(None),
                },
                &pool,
            )
            .await
            .unwrap();
    }
    miner.reselect(&settings).await;
    assert!(miner.watching.is_none());
    miner.schedule(&settings).await;
    finish_job(&mut miner, &pool).await;
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == "/gql")
            .count(),
        1,
        "stall recovery must fetch account inventory before the periodic refresh"
    );
    let drop = &miner.campaigns[0].drops[0];
    assert_eq!(drop.estimated_minutes, 0);
    assert_eq!(drop.confirmed_minutes, 12);
    assert!(!drop.claimed);
    miner.reselect(&settings).await;
    assert_eq!(miner.watching, Some(10));
    pool.close().await;
}

#[tokio::test]
async fn shutdown_before_queued_logout_still_removes_credentials() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (app, _unused) = App::open(dir.path().to_owned()).unwrap();
    session().save(dir.path()).unwrap();
    let (sender, commands) = mpsc::channel(4);
    let (shutdown, shutdown_result) = oneshot::channel();
    let (logout, logout_result) = oneshot::channel();
    sender
        .send(CommandRequest {
            command: Command::Shutdown,
            complete: shutdown,
        })
        .await
        .unwrap();
    sender
        .send(CommandRequest {
            command: Command::Logout,
            complete: logout,
        })
        .await
        .unwrap();
    let owner = Miner {
        app,
        commands,
        endpoints: Endpoints::mock(&server.uri()),
    };
    tokio::time::timeout(Duration::from_secs(3), owner.run())
        .await
        .unwrap()
        .unwrap();
    shutdown_result.await.unwrap().unwrap();
    let result = logout_result.await;
    assert!(
        Session::load(dir.path()).unwrap().is_none(),
        "queued logout was dropped after shutdown; result={result:?}"
    );
}

#[tokio::test]
async fn corrupt_twitch_session_can_reach_fresh_device_login() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/tv"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/oauth2/device")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code":"private","user_code":"NEWCODE","verification_uri":"https://www.twitch.tv/activate","interval":1,"expires_in":60}))).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let (app, _commands) = App::open(dir.path().to_owned()).unwrap();
    std::fs::write(dir.path().join("twitch_session.json"), b"{broken").unwrap();
    let owned = app.clone();
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    let endpoints = Endpoints::mock(&server.uri());
    let task = tokio::spawn(async move {
        authenticate(
            &owned,
            &Settings::default(),
            &endpoints,
            &stopped,
            &Notify::new(),
        )
        .await
    });
    let available = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if app.snapshot.read().await.login.oauth_pending.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    cancel.cancel();
    let _ = task.await;
    assert!(
        available,
        "invalid session endlessly retries storage instead of offering device login"
    );
}

#[tokio::test]
async fn hourly_token_validation_preserves_manual_choice() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
        .mount(&server)
        .await;
    for login in ["first", "second"] {
        Mock::given(method("GET"))
            .and(path(format!("/{login}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!(r#"{{"beacon_url":"{}/track"}}"#, server.uri())),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/track"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    gql_mock(&server,|q|match q["operationName"].as_str().unwrap(){
        "PlaybackAccessToken" => json!({"data":{"streamPlaybackAccessToken":null}}),
        "AccountProfile" | "AccountBadges" => json!({"data":null}),
        "Inventory"=>json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign_json("one")],"gameEventDrops":[]}}}}),
        "DirectoryPage_Game"=>json!({"data":{"game":{"streams":{"edges":[
            {"node":{"id":"b1","broadcaster":{"id":"10","login":"first","displayName":"First"},"game":{"id":"1","name":"Rust"},"viewersCount":100}},
            {"node":{"id":"b2","broadcaster":{"id":"11","login":"second","displayName":"Second"},"game":{"id":"1","name":"Rust"},"viewersCount":50}}
        ]}}}}),
        "DropCurrentSessionContext"=>json!({"data":{"currentUser":{"dropCurrentSession":{"dropID":"drop-one","currentMinutesWatched":12}}}}),
        "VideoPlayerStreamInfoOverlayChannel"=>json!({"data":{"user":{"id":"11","displayName":"Second","stream":{"id":"b2"},"broadcastSettings":{"game":{"id":"1","name":"Rust"}}}}}),
        "DropsHighlightService_AvailableDrops"=>json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"one"}]}}}),
        other=>panic!("unexpected operation {other}"),
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    app.settings.write().await.games_to_watch = vec!["Rust".into()];
    session().save(dir.path()).unwrap();
    let owner = Miner {
        app: app.clone(),
        commands,
        endpoints: Endpoints::mock(&server.uri()),
    };
    let worker = tokio::spawn(owner.run());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app.snapshot.read().await.channels.len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.command(Command::SelectChannel(11, None)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = app.snapshot.read().await;
            if state.manual_mode.active && state.channels.iter().any(|c| c.id == 11 && c.watching) {
                break;
            }
            drop(state);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let checked_before = app.snapshot.read().await.inventory_status.checked_at;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3601)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let revalidated = server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/oauth2/validate")
                .count()
                >= 2;
            let state = app.snapshot.read().await;
            if revalidated
                && state.channels.len() == 2
                && state.inventory_status.checked_at > checked_before
            {
                break;
            }
            drop(state);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let state = app.snapshot.read().await.clone();
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        state.manual_mode.active,
        "routine hourly validation exited manual mode"
    );
    assert!(state.channels.iter().any(|c| c.id == 11 && c.watching));
}

#[tokio::test]
async fn interrupted_final_claim_recovers_without_catalog_and_never_crosses_accounts() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].claim_id = Some("account-instance".into());
    let pending = PendingClaim::new(
        42,
        &miner.campaigns[0],
        &miner.campaigns[0].drops[0],
        &Settings::default(),
    );
    miner.journal.lock().await.prepare(pending.clone()).unwrap();
    let mut other = pending.clone();
    other.user_id = 99;
    miner.journal.lock().await.prepare(other).unwrap();
    miner.campaigns.clear();
    miner.journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
    let too_old = HashMap::from([(
        pending.benefits[0].clone(),
        pending.starts_at - chrono::Duration::seconds(1),
    )]);
    miner.recover_claims(&too_old).await.unwrap();
    assert_eq!(miner.app.history.lock().await.total(), 0);
    assert_eq!(miner.pending_claims.len(), 1);
    assert_eq!(miner.pending_claims[0].instance, "account-instance");
    let evidence = HashMap::from([(
        pending.benefits[0].clone(),
        pending.starts_at + chrono::Duration::seconds(1),
    )]);
    // A failed archive write leaves the journal intact after the idempotent history write.
    let archive = dir.path().join("completed_campaigns.json");
    std::fs::create_dir(&archive).unwrap();
    assert_eq!(
        miner.recover_claims(&evidence).await,
        Err(TwitchError::Storage)
    );
    assert_eq!(ClaimJournal::load(dir.path()).unwrap().pending(42).len(), 1);
    std::fs::remove_dir(&archive).unwrap();
    miner.recover_claims(&evidence).await.unwrap();
    miner.recover_claims(&evidence).await.unwrap();
    assert_eq!(History::load(dir.path()).total(), 1);
    assert!(CampaignArchive::load(dir.path()).merge(vec![], Utc::now())[0].finished);
    let journal = ClaimJournal::load(dir.path()).unwrap();
    assert!(journal.pending(42).is_empty());
    assert_eq!(journal.pending(99).len(), 1);
    pool.close().await;
}

#[tokio::test]
async fn pending_claim_replays_its_account_instance_only_within_original_grace_period() {
    let server = MockServer::start().await;
    gql_mock(&server, |query| {
        assert_eq!(
            query["variables"]["input"]["dropInstanceID"],
            "account-instance"
        );
        json!({"data":{"claimDropRewards":{"status":"DROP_INSTANCE_ALREADY_CLAIMED"}}})
    })
    .await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].claim_id = Some("account-instance".into());
    let pending = PendingClaim::new(
        42,
        &miner.campaigns[0],
        &miner.campaigns[0].drops[0],
        &Settings::default(),
    );
    miner.journal.lock().await.prepare(pending.clone()).unwrap();
    miner.campaigns.clear();
    miner.recover_claims(&HashMap::new()).await.unwrap();
    miner.pending_claims[0].retry_until = Utc::now();
    miner.schedule(&Settings::default()).await;
    assert!(miner.jobs.is_empty());
    miner.pending_claims[0].retry_until = pending.retry_until;
    miner.schedule(&Settings::default()).await;
    finish_job(&mut miner, &pool).await;
    assert_eq!(History::load(dir.path()).total(), 1);
    assert!(CampaignArchive::load(dir.path()).merge(vec![], Utc::now())[0].finished);
    assert!(miner.pending_claims.is_empty());
    pool.close().await;
}

#[tokio::test]
async fn viewer_event_cannot_discard_fresh_category_and_drop_eligibility() {
    let server = MockServer::start().await;
    let (_dir, mut miner, _intent, mut pool) = miner(&server).await;
    let settings = select(&mut miner).await;
    let mut updated = miner.channels.clone();
    updated[0].game.as_mut().unwrap().id = 2;
    updated[0].game.as_mut().unwrap().name = "Different Game".into();
    updated[0].drops_enabled = false;
    let requested_at = Instant::now() - Duration::from_secs(1);
    miner
        .event(Event::Viewers { id: 10, count: 500 })
        .await
        .unwrap();
    miner
        .complete(
            Job::Update {
                result: Ok(updated),
                requested_at,
            },
            &pool,
        )
        .await
        .unwrap();
    miner.reselect(&settings).await;
    let actual_game = miner.channels[0].game.as_ref().unwrap().id;
    let actual_viewers = miner.channels[0].viewers;
    let watching = miner.watching;
    pool.close().await;
    assert_eq!(actual_viewers, Some(500));
    assert_eq!(
        actual_game, 2,
        "viewer update discarded unrelated fresh stream metadata"
    );
    assert!(
        watching.is_none(),
        "miner kept watching a channel that switched out of the eligible category"
    );
}

#[tokio::test]
async fn pending_award_inference_cannot_override_explicit_unclaimed_account_state() {
    let server = MockServer::start().await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].claim_id = Some("account-instance".into());
    let pending = PendingClaim::new(
        42,
        &miner.campaigns[0],
        &miner.campaigns[0].drops[0],
        &Settings::default(),
    );
    miner.journal.lock().await.prepare(pending.clone()).unwrap();
    assert!(!miner.campaigns[0].drops[0].claimed);
    assert!(miner.campaigns[0].drops[0].confirmed_at.is_some());
    let awards = HashMap::from([(
        pending.benefits[0].clone(),
        pending.starts_at + chrono::Duration::seconds(1),
    )]);
    miner.recover_claims(&awards).await.unwrap();
    let history_count = History::load(dir.path()).total();
    let journal_count = ClaimJournal::load(dir.path()).unwrap().pending(42).len();
    pool.close().await;
    assert_eq!(
        history_count, 0,
        "award inference must not override the explicit account isClaimed:false edge"
    );
    assert_eq!(journal_count, 1);
}

#[tokio::test]
async fn rejected_account_records_block_award_recovery_but_preserve_durable_receipts() {
    for mode in ["malformed", "duplicate"] {
        for retention in ["active", "upcoming", "expired", "cold_start"] {
            for receipt in [false, true] {
                let server = MockServer::start().await;
                let (dir, mut mining, _intent, mut pool) = miner(&server).await;
                mining.campaigns[0].drops[0].claim_id = Some("account-instance".into());
                let pending = PendingClaim::new(
                    42,
                    &mining.campaigns[0],
                    &mining.campaigns[0].drops[0],
                    &Settings::default(),
                );
                mining
                    .journal
                    .lock()
                    .await
                    .prepare(pending.clone())
                    .unwrap();
                if receipt {
                    mining
                        .journal
                        .lock()
                        .await
                        .confirm(42, &pending.entry.id)
                        .unwrap();
                }
                // Reload the journal as on restart, including its confirmed receipt.
                mining.journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
                match retention {
                    "cold_start" => mining.campaigns.clear(),
                    "upcoming" => {
                        mining.campaigns[0].starts_at = Utc::now() + chrono::Duration::hours(1)
                    }
                    "expired" => {
                        mining.campaigns[0].ends_at = Utc::now() - chrono::Duration::seconds(1);
                        mining.campaigns[0].drops[0].confirm(60, Utc::now());
                    }
                    _ => {}
                }
                let unclaimed = campaign_json("one");
                let mut rejected = unclaimed.clone();
                let records = if mode == "malformed" {
                    rejected["allow"]
                        .as_object_mut()
                        .unwrap()
                        .remove("channels");
                    vec![rejected]
                } else {
                    rejected["timeBasedDrops"][0]["self"]["isClaimed"] = json!(true);
                    vec![unclaimed, rejected]
                };
                let awarded_at = pending.starts_at + chrono::Duration::seconds(1);
                let benefit = pending.benefits[0].clone();
                gql_mock(&server, move |_| {
                    json!({"data":{"currentUser":{"inventory":{
                        "dropCampaignsInProgress":records,
                        "gameEventDrops":[{"id":benefit,"lastAwardedAt":awarded_at.to_rfc3339()}]
                    }}}})
                })
                .await;
                // A matching public record must not substitute its award assumptions.
                Mock::given(method("GET")).and(path("/catalog"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "lastUpdatedAt":Utc::now().to_rfc3339(),"data":[{"gameId":"1","rewards":[campaign_json("one")]}]
                    }))).mount(&server).await;
                let requested_at = Utc::now();
                let inventory = mining.client.inventory().await.unwrap();
                mining
                    .complete(
                        Job::Inventory {
                            result: Ok(inventory),
                            requested_at,
                            refresh_sequence: 0,
                        },
                        &pool,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    History::load(dir.path()).total(),
                    usize::from(receipt),
                    "{mode}/{retention}/{receipt}"
                );
                assert_eq!(
                    ClaimJournal::load(dir.path()).unwrap().pending(42).len(),
                    usize::from(!receipt)
                );
                if retention != "cold_start" {
                    assert_eq!(mining.campaigns.len(), 1);
                    assert_eq!(mining.campaigns[0].drops[0].claimed, receipt);
                } else {
                    assert!(mining.campaigns.is_empty());
                }
                // Without a receipt, recovery must leave the exact issued instance pending.
                if !receipt {
                    assert_eq!(mining.pending_claims[0].instance, "account-instance");
                }
                pool.close().await;
            }
        }
    }
}

#[tokio::test]
async fn preclaim_publication_and_shutdown_leave_a_durable_receipt_for_finished_recovery() {
    let server = MockServer::start().await;
    gql_mock(
        &server,
        |_| json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}),
    )
    .await;
    let (dir, mut miner, _intent, mut pool) = miner(&server).await;
    miner.campaigns[0].drops[0].claim_id = Some("account-instance".into());
    let settings = Settings::default();
    let pending = PendingClaim::new(
        42,
        &miner.campaigns[0],
        &miner.campaigns[0].drops[0],
        &settings,
    );
    let history_revision = miner.app.snapshot.read().await.history_revision;
    // The durable RPC result must invalidate History even without a catalog record.
    miner.campaigns.clear();
    assert!(
        claim(&miner.app, &miner.client, &miner.journal, pending)
            .await
            .unwrap()
    );
    assert!(ClaimJournal::load(dir.path()).unwrap().pending(42)[0].confirmed);
    assert_eq!(
        miner.app.snapshot.read().await.history_revision,
        history_revision + 1
    );
    miner
        .event(Event::Viewers { id: 10, count: 500 })
        .await
        .unwrap();
    miner.publish(&settings).await.unwrap();
    miner.journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
    miner.campaigns.clear();
    miner.recover_claims(&HashMap::new()).await.unwrap();
    assert_eq!(
        miner.app.snapshot.read().await.history_revision,
        history_revision + 1
    );
    assert_eq!(History::load(dir.path()).total(), 1);
    assert_eq!(
        CampaignArchive::load(dir.path())
            .merge(vec![], Utc::now())
            .len(),
        1
    );
    assert!(
        ClaimJournal::load(dir.path())
            .unwrap()
            .pending(42)
            .is_empty()
    );
    pool.close().await;
}

#[tokio::test]
async fn channel_choice_during_hourly_reload_is_retained_until_channels_are_ready() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
        .mount(&server)
        .await;
    for login in ["first", "second"] {
        Mock::given(method("GET"))
            .and(path(format!("/{login}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!(r#"{{"beacon_url":"{}/track"}}"#, server.uri())),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/track"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let inventories = Arc::new(AtomicUsize::new(0));
    let count = inventories.clone();
    Mock::given(method("POST")).and(path("/gql")).respond_with(move |request: &wiremock::Request| {
        let body: serde_json::Value = request.body_json().unwrap();
        let handler = |q: &serde_json::Value| match q["operationName"].as_str().unwrap() {
            "PlaybackAccessToken" => json!({"data":{"streamPlaybackAccessToken":null}}),
        "AccountProfile" | "AccountBadges" => json!({"data":null}),
        "Inventory" => json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign_json("one")],"gameEventDrops":[]}}}}),
            "DirectoryPage_Game" => json!({"data":{"game":{"streams":{"edges":[
                {"node":{"id":"b1","broadcaster":{"id":"10","login":"first","displayName":"First"},"game":{"id":"1","name":"Rust"},"viewersCount":100}},
                {"node":{"id":"b2","broadcaster":{"id":"11","login":"second","displayName":"Second"},"game":{"id":"1","name":"Rust"},"viewersCount":50}}
            ]}}}}),
            "DropCurrentSessionContext" => json!({"data":{"currentUser":{"dropCurrentSession":{"dropID":"drop-one","currentMinutesWatched":12}}}}),
            _ => unreachable!(),
        };
        let delayed = body["operationName"] == "Inventory" && count.fetch_add(1, Ordering::SeqCst) >= 1;
        let value = body.as_array().map_or_else(|| handler(&body), |batch| serde_json::Value::Array(batch.iter().map(handler).collect()));
        let response = ResponseTemplate::new(200).set_body_json(value);
        if delayed { response.set_delay(Duration::from_millis(750)) } else { response }
    }).mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    app.settings.write().await.games_to_watch = vec!["Rust".into()];
    session().save(dir.path()).unwrap();
    let owner = Miner {
        app: app.clone(),
        commands,
        endpoints: Endpoints::mock(&server.uri()),
    };
    let worker = tokio::spawn(owner.run());
    tokio::time::timeout(Duration::from_secs(5), async {
        while app.snapshot.read().await.channels.len() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let checked_before = app.snapshot.read().await.inventory_status.checked_at;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3601)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        while inventories.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        app.snapshot
            .read()
            .await
            .channels
            .iter()
            .any(|c| c.id == 11)
    );
    app.command(Command::SelectChannel(11, None)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = app.snapshot.read().await;
            if state.channels.len() == 2 && state.inventory_status.checked_at > checked_before {
                break;
            }
            drop(state);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let state = app.snapshot.read().await.clone();
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        state.manual_mode.active,
        "accepted channel choice vanished during hourly reload"
    );
    assert!(state.channels.iter().any(|c| c.id == 11 && c.watching));
}
