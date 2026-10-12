use super::*;

async fn gql_requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/gql")
        .count()
}

async fn save_auto_claim(mining: &Mining, enabled: bool) -> Settings {
    let settings = mining
        .app
        .settings
        .read()
        .await
        .patched(&json!({"auto_claim_drops":enabled}))
        .unwrap();
    mining
        .app
        .change_settings(|_| Ok(settings))
        .await
        .unwrap()
        .values
}

#[tokio::test]
async fn disabled_auto_claim_keeps_watching_and_inventory_running() {
    for manual in [false, true] {
        let server = MockServer::start().await;
        gql_mock(&server, |q| match q["operationName"].as_str().unwrap() {
            "Inventory" => json!({"data":{"currentUser":{"inventory":{
                "dropCampaignsInProgress":[campaign_json("one")],"gameEventDrops":[]
            }}}}),
            "DropCurrentSessionContext" => {
                json!({"data":{"currentUser":{"dropCurrentSession":null}}})
            }
            operation => panic!("unexpected operation: {operation}"),
        })
        .await;
        Mock::given(method("POST"))
            .and(path("/track"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (dir, mut mining, _intent, mut pool) = miner(&server).await;
        select(&mut mining).await;
        let settings = save_auto_claim(&mining, false).await;
        let mut notifications = mining.app.notifications.subscribe();
        if manual {
            mining.manual = Some(ManualSelection::new(10, None));
        }
        mining.channels[0].beacon_url = Some(format!("{}/track", server.uri()).parse().unwrap());
        mining
            .event(Event::Claim {
                id: "drop-one".into(),
                instance: "earned-instance".into(),
            })
            .await
            .unwrap();
        mining.schedule(&settings).await;
        assert!(mining.busy.contains(&JobKind::Watch));
        finish_job(&mut mining, &pool).await;
        mining.poll_at = Some(Instant::now());
        mining.schedule(&settings).await;
        assert!(mining.busy.contains(&JobKind::Poll));
        finish_job(&mut mining, &pool).await;
        mining
            .event(Event::Notification("claim-reminder".into()))
            .await
            .unwrap();
        mining.schedule(&settings).await;
        assert!(mining.busy.contains(&JobKind::Inventory));
        finish_job(&mut mining, &pool).await;
        mining.channels_dirty = false;
        mining.schedule(&settings).await;
        assert!(!mining.busy.contains(&JobKind::Notification));
        assert!(mining.notifications.contains("claim-reminder"));
        assert!(!mining.campaigns[0].drops[0].claimed);
        assert!(mining.journal.lock().await.pending(42).is_empty());
        assert_eq!(History::load(dir.path()).total(), 0);
        assert!(notifications.try_recv().is_err());
        pool.close().await;
    }
}

#[tokio::test]
async fn disabling_auto_claim_preserves_reminders_with_stale_scheduler_settings() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| {
        assert_eq!(q["operationName"], "OnsiteNotifications_DeleteNotification");
        assert_eq!(q["variables"]["input"]["id"], "claim-reminder");
        json!({"data":{"deleteNotification":null}})
    })
    .await;
    let (_dir, mut mining, _intent, mut pool) = miner(&server).await;
    mining.notifications.insert("claim-reminder".into());
    let stale = mining.app.settings.read().await.clone();
    save_auto_claim(&mining, false).await;
    mining.schedule(&stale).await;
    assert!(mining.jobs.is_empty());
    assert!(mining.notifications.contains("claim-reminder"));
    assert_eq!(gql_requests(&server).await, 0);

    let enabled = save_auto_claim(&mining, true).await;
    mining.schedule(&enabled).await;
    assert!(mining.busy.contains(&JobKind::Notification));
    finish_job(&mut mining, &pool).await;
    assert!(mining.notifications.is_empty());
    assert_eq!(gql_requests(&server).await, 1);
    pool.close().await;
}

#[tokio::test]
async fn disabled_auto_claim_preserves_restart_intents_until_reenabled() {
    let server = MockServer::start().await;
    gql_mock(&server, |q| {
        assert_eq!(q["operationName"], "DropsPage_ClaimDropRewards");
        assert_eq!(q["variables"]["input"]["dropInstanceID"], "earned-instance");
        json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}})
    })
    .await;
    let (dir, mut mining, _intent, mut pool) = miner(&server).await;
    let settings = save_auto_claim(&mining, false).await;
    mining.campaigns[0].drops[0].claim_id = Some("earned-instance".into());
    let pending = PendingClaim::new(
        42,
        &mining.campaigns[0],
        &mining.campaigns[0].drops[0],
        &settings,
    );
    mining
        .journal
        .lock()
        .await
        .prepare(pending.clone())
        .unwrap();
    // Restore an unacknowledged attempt without its catalog record, as after restart.
    mining.journal = Arc::new(Mutex::new(ClaimJournal::load(dir.path()).unwrap()));
    mining.campaigns.clear();
    mining.recover_claims(&HashMap::new()).await.unwrap();
    mining.schedule(&settings).await;
    assert!(mining.jobs.is_empty());
    assert_eq!(gql_requests(&server).await, 0);
    assert_eq!(History::load(dir.path()).total(), 0);
    assert_eq!(mining.pending_claims[0].instance, pending.instance);
    let saved = std::fs::read(dir.path().join("settings.json")).unwrap();
    assert!(
        !Settings::from_saved(serde_json::from_slice(&saved).unwrap())
            .unwrap()
            .auto_claim_drops
    );

    let enabled = save_auto_claim(&mining, true).await;
    mining.schedule(&enabled).await;
    finish_job(&mut mining, &pool).await;
    assert_eq!(History::load(dir.path()).total(), 1);
    assert!(mining.pending_claims.is_empty());
    assert!(mining.journal.lock().await.settled(42, "drop-one"));
    mining.schedule(&enabled).await;
    assert!(mining.jobs.is_empty());
    assert_eq!(gql_requests(&server).await, 1);
    pool.close().await;
}

#[tokio::test]
async fn disabling_auto_claim_fences_a_queued_job_without_delaying_reenable() {
    let server = MockServer::start().await;
    gql_mock(
        &server,
        |_| json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}),
    )
    .await;
    let (dir, mut mining, _intent, mut pool) = miner(&server).await;
    mining.campaigns[0].drops[0].claim_id = Some("earned-instance".into());
    let stale = mining.app.settings.read().await.clone();
    save_auto_claim(&mining, false).await;
    // The scheduler may already hold a settings snapshot from before the save.
    mining.schedule(&stale).await;
    assert!(mining.busy.contains(&JobKind::Claim));
    finish_job(&mut mining, &pool).await;
    assert_eq!(gql_requests(&server).await, 0);
    assert_eq!(History::load(dir.path()).total(), 0);
    assert!(!mining.claim_retry.contains_key("drop-one"));

    let settings = save_auto_claim(&mining, true).await;
    mining.schedule(&settings).await;
    finish_job(&mut mining, &pool).await;
    assert_eq!(History::load(dir.path()).total(), 1);
    assert_eq!(gql_requests(&server).await, 1);
    pool.close().await;
}

#[tokio::test]
async fn disabling_auto_claim_drains_an_active_claim_without_claiming_the_next_reward() {
    let server = MockServer::start().await;
    let sent = Arc::new(Notify::new());
    let observed = sent.clone();
    Mock::given(method("POST"))
        .and(path("/gql"))
        .respond_with(move |_: &wiremock::Request| {
            observed.notify_one();
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"claimDropRewards":{"status":"ELIGIBLE_FOR_ALL"}}}))
        })
        .expect(1)
        .mount(&server)
        .await;
    let (dir, mut mining, _intent, mut pool) = miner(&server).await;
    let mut notifications = mining.app.notifications.subscribe();
    let settings = mining.app.settings.read().await.clone();
    mining
        .campaigns
        .push(Campaign::parse(&campaign_json("two"), &HashMap::new(), Utc::now()).unwrap());
    for campaign in &mut mining.campaigns {
        campaign.drops[0].claim_id = Some(format!("instance-{}", campaign.id));
    }
    // Keep the acknowledged claim's history write in flight while the setting is saved.
    let app = mining.app.clone();
    let history = app.history.lock().await;
    mining.schedule(&settings).await;
    tokio::time::timeout(Duration::from_secs(5), sent.notified())
        .await
        .unwrap();
    let disabled = save_auto_claim(&mining, false).await;
    drop(history);
    finish_job(&mut mining, &pool).await;
    assert_eq!(History::load(dir.path()).total(), 1);
    assert!(mining.campaigns[0].drops[0].claimed);
    assert!(!mining.campaigns[1].drops[0].claimed);
    assert!(mining.journal.lock().await.settled(42, "drop-one"));
    assert!(notifications.try_recv().is_ok());
    assert!(notifications.try_recv().is_err());
    mining.schedule(&disabled).await;
    assert!(mining.jobs.is_empty());
    assert_eq!(gql_requests(&server).await, 1);
    pool.close().await;
}

#[tokio::test]
async fn disabling_auto_claim_does_not_unlock_prerequisites_or_hide_manual_claims() {
    let server = MockServer::start().await;
    let (dir, mut mining, _intent, mut pool) = miner(&server).await;
    select(&mut mining).await;
    let settings = save_auto_claim(&mining, false).await;
    let mut notifications = mining.app.notifications.subscribe();
    let mut successor = mining.campaigns[0].drops[0].clone();
    successor.id = "successor".into();
    successor.prerequisites = vec!["drop-one".into()];
    mining.campaigns[0].drops.push(successor);
    mining.campaigns[0].drops[0].claim_id = Some("earned-instance".into());
    mining.campaigns[0].drops[0].confirmed_minutes = 60;
    mining.reselect(&settings).await;
    assert!(mining.watching.is_none());
    assert!(!mining.campaigns[0].can_mine(&settings, Utc::now()));
    assert_eq!(History::load(dir.path()).total(), 0);

    let mut confirmed = mining.campaigns.clone();
    confirmed[0].drops[0].mark_claimed(Utc::now());
    let (refresh_sequence, _) = mining.app.begin_inventory_refresh().await;
    mining
        .complete(
            Job::Inventory {
                result: Ok(Inventory {
                    campaigns: confirmed,
                    awards: HashMap::new(),
                    rejected_account_ids: HashSet::new(),
                    status: InventoryStatus {
                        available: true,
                        ..Default::default()
                    },
                }),
                requested_at: Utc::now(),
                refresh_sequence,
            },
            &pool,
        )
        .await
        .unwrap();
    mining.reselect(&settings).await;
    assert_eq!(mining.watching, Some(10));
    assert!(mining.campaigns[0].can_mine(&settings, Utc::now()));
    assert_eq!(History::load(dir.path()).total(), 1);
    assert!(notifications.try_recv().is_err());
    assert_eq!(gql_requests(&server).await, 0);
    pool.close().await;
}
