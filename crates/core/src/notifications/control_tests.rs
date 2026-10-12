use super::*;
use crate::notifications::{Telegram, Update};
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn incoming(id: i64, text: &str, date: i64) -> Value {
    json!({"update_id":id, "message":{"date":date, "text":text,
        "chat":{"id":42, "type":"private"}, "from":{"id":42, "is_bot":false}}})
}

fn configuration(service: &Telegram) -> Update {
    Update {
        revision: service.view().revision,
        enabled: false,
        remote_control: true,
        include_image: false,
        token: "123:control_test".into(),
        chat_id: "42".into(),
        remove: false,
    }
}

fn session(app: &Application) -> Session {
    let state = app.telegram.state.lock().unwrap();
    Session {
        config: state.config.clone(),
        epoch: state.epoch,
        cancel: state.cancel.clone(),
        started: Utc::now().timestamp(),
    }
}

async fn application(
    server: &MockServer,
) -> (
    tempfile::TempDir,
    Arc<Application>,
    tokio::sync::mpsc::Receiver<crate::app::CommandRequest>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, commands) = Application::open(dir.path().into()).unwrap();
    Arc::get_mut(&mut app).unwrap().telegram.endpoint = server.uri();
    app.telegram.save(configuration(&app.telegram)).unwrap();
    app.snapshot.write().await.login.user_id = Some(7);
    (dir, app, commands)
}

#[test]
fn only_fresh_direct_commands_from_the_configured_private_owner_are_accepted() {
    let now = 2000;
    let valid = incoming(1, "/pause", now);
    assert_eq!(command(&valid, 42, 1990, now), Some(Command::Pause));
    let mut invalid = Vec::new();
    for (pointer, value) in [
        ("/message/chat/id", json!(43)),
        ("/message/chat/type", json!("group")),
        ("/message/from/id", json!(43)),
        ("/message/from/is_bot", json!(true)),
        ("/message/from/is_bot", Value::Null),
        ("/message/date", json!(1989)),
        ("/message/date", json!(2006)),
        ("/message/text", json!("/pause\n/resume")),
        ("/message/text", json!("x".repeat(257))),
    ] {
        let mut update = valid.clone();
        *update.pointer_mut(pointer).unwrap() = value;
        invalid.push(update);
    }
    for key in [
        "forward_origin",
        "forward_from",
        "forward_date",
        "sender_chat",
        "via_bot",
        "edit_date",
        "business_connection_id",
    ] {
        let mut update = valid.clone();
        update["message"][key] = json!({});
        invalid.push(update);
    }
    invalid.push(json!({"update_id":1, "edited_message":valid["message"]}));
    for update in invalid {
        assert_eq!(command(&update, 42, 1990, now), None, "{update}");
    }
    assert_eq!(command(&valid, -42, 1990, now), None);
    assert_eq!(command(&valid, 42, 0, now + 121), None);
    assert_eq!(
        command(&incoming(2, "/mine  Just Chatting ", now), 42, now, now),
        Some(Command::Mine("Just Chatting".into()))
    );
    for input in ["/mine", "/delete", "/pause@other_bot"] {
        assert_eq!(
            command(&incoming(2, input, now), 42, now, now),
            Some(Command::Help)
        );
    }
}

#[test]
fn controls_default_off_and_require_a_private_chat_without_changing_alert_compatibility() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("telegram.json"),
        br#"{"enabled":true,"token":"123:control_test","chat_id":"-42"}"#,
    )
    .unwrap();
    let service = Telegram::open(dir.path()).unwrap();
    assert!(!service.view().remote_control);
    assert!(service.view().enabled);
    let mut update = configuration(&service);
    update.chat_id = "-42".into();
    assert!(matches!(service.save(update), Err(Error::PrivateChat)));
    assert_eq!(service.view().chat_id, "-42");
    service.save(configuration(&service)).unwrap();
    assert!(service.view().remote_control && !service.view().enabled);
    assert!(Telegram::open(dir.path()).unwrap().view().remote_control);
    assert!(
        !serde_json::to_string(&service.view())
            .unwrap()
            .contains("control_test")
    );
}

#[tokio::test]
async fn commands_save_only_the_requested_setting_and_use_confirmed_progress() {
    let server = MockServer::start().await;
    let (dir, app, mut commands) = application(&server).await;
    {
        let mut state = app.snapshot.write().await;
        state.settings.values.games_to_watch = vec!["Other game".into()];
        state.campaigns.push(crate::dto::CampaignView {
            game_name: "Straße".into(),
            ..Default::default()
        });
        state.manual_mode.active = true;
        state.current_drop = Some(crate::dto::Progress {
            drop_id: "drop".into(),
            drop_name: "<&reward>".into(),
            campaign_id: "campaign".into(),
            campaign_name: "Campaign".into(),
            game_name: "Game".into(),
            current_minutes: 99,
            confirmed_minutes: 12,
            confirmed_at: Some(Utc::now()),
            required_minutes: 100,
            progress: 0.99,
            remaining_seconds: 60,
        });
    }
    let worker = tokio::spawn(async move {
        while let Some(request) = commands.recv().await {
            assert!(matches!(
                request.command,
                crate::app::Command::SettingsChanged
            ));
            let _ = request.complete.send(Ok(()));
        }
    });
    let session = session(&app);
    let reply = session.execute(&app, Command::Status).await.unwrap();
    assert!(reply.contains("12 / 100") && !reply.contains("99 /"));
    assert!(reply.contains("&lt;&amp;reward&gt;") && reply.contains("Manual channel"));
    app.snapshot
        .write()
        .await
        .current_drop
        .as_mut()
        .unwrap()
        .confirmed_at = None;
    assert!(
        session
            .execute(&app, Command::Status)
            .await
            .unwrap()
            .contains("not confirmed")
    );
    session.execute(&app, Command::Pause).await.unwrap();
    assert!(app.settings.read().await.mining_paused);
    session
        .execute(&app, Command::Mine("STRASSE".into()))
        .await
        .unwrap();
    session
        .execute(&app, Command::Mine("straße".into()))
        .await
        .unwrap();
    assert_eq!(
        app.settings.read().await.games_to_watch,
        ["Other game", "Straße"]
    );
    assert!(app.snapshot.read().await.manual_mode.active);
    assert!(app.settings.read().await.mining_paused);
    assert!(
        session
            .execute(&app, Command::Mine("unknown".into()))
            .await
            .unwrap()
            .contains("not found")
    );
    session.execute(&app, Command::Resume).await.unwrap();
    let disk: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(disk["mining_paused"], false);
    app.shutdown.cancel();
    app.drain_writes().await;
    worker.abort();
    let _ = worker.await;
}

#[tokio::test]
async fn logout_and_reconfiguration_fence_queued_and_stale_commands() {
    let server = MockServer::start().await;
    let (_dir, app, _commands) = application(&server).await;
    let old = session(&app);
    let slot = app.settings_slot.acquire().await.unwrap();
    let execute = old.execute(&app, Command::Pause);
    tokio::pin!(execute);
    tokio::select! { result = &mut execute => panic!("unexpected {result:?}"), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
    app.telegram.cancel_pending();
    assert!(matches!(
        execute.await,
        Err(AppError::Telegram(Error::Cancelled))
    ));
    drop(slot);
    app.telegram.save(configuration(&app.telegram)).unwrap();
    assert!(matches!(
        old.execute(&app, Command::Pause).await,
        Err(AppError::Telegram(Error::Cancelled))
    ));
    assert!(!app.settings.read().await.mining_paused);
    app.snapshot.write().await.login.user_id = None;
    assert!(
        session(&app)
            .execute(&app, Command::Resume)
            .await
            .unwrap()
            .contains("Connect your Twitch")
    );
    app.shutdown.cancel();
    app.drain_writes().await;
}

#[tokio::test]
async fn polling_discards_backlog_deduplicates_and_replies_without_claim_alerts() {
    let server = MockServer::start().await;
    let (_dir, app, mut commands) = application(&server).await;
    let writes = Arc::new(AtomicUsize::new(0));
    let count = writes.clone();
    let worker = tokio::spawn(async move {
        while let Some(request) = commands.recv().await {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = request.complete.send(Ok(()));
        }
    });
    Mock::given(method("POST"))
        .and(path("/bot123:control_test/getUpdates"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let now = Utc::now().timestamp();
            let result = match body["offset"].as_i64().unwrap() {
                -1 => vec![incoming(10, "/resume", now)],
                11 => vec![incoming(11, "/pause", now), incoming(11, "/pause", now)],
                _ => vec![incoming(11, "/pause", now)],
            };
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":result}))
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bot123:control_test/setMyCommands"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":true})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bot123:control_test/sendMessage"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let remote = tokio::spawn(run(app.clone()));
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let requests = server.received_requests().await.unwrap();
            if requests
                .iter()
                .filter(|r| r.url.path().ends_with("getUpdates"))
                .count()
                >= 3
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(app.settings.read().await.mining_paused);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    let requests = server.received_requests().await.unwrap();
    for request in &requests {
        for header in ["authorization", "cookie", "client-id"] {
            assert!(!request.headers.contains_key(header));
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        if request.url.path().ends_with("setMyCommands") {
            assert_eq!(body["scope"], json!({"type":"chat","chat_id":"42"}));
        }
        if request.url.path().ends_with("getUpdates") {
            assert_eq!(body["allowed_updates"], json!(["message"]));
        }
    }
    app.shutdown.cancel();
    remote.await.unwrap();
    app.drain_writes().await;
    worker.abort();
    let _ = worker.await;
}

#[tokio::test]
async fn requests_are_bounded_cancellable_and_do_not_change_webhooks() {
    let server = MockServer::start().await;
    let (_dir, app, _commands) = application(&server).await;
    let session = session(&app);
    for (response, error) in [
        (
            ResponseTemplate::new(409).set_body_string("secret"),
            Failure::Conflict,
        ),
        (
            ResponseTemplate::new(429).set_body_json(json!({"parameters":{"retry_after":999}})),
            Failure::RateLimited(60),
        ),
        (
            ResponseTemplate::new(200).set_body_string("x".repeat(65537)),
            Failure::Unavailable,
        ),
        (
            ResponseTemplate::new(302).insert_header("location", "/redirect"),
            Failure::Unavailable,
        ),
        (
            ResponseTemplate::new(200).set_body_json(json!({"ok":false})),
            Failure::Unavailable,
        ),
    ] {
        server.reset().await;
        Mock::given(path("/bot123:control_test/getUpdates"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(session.api(&app, "getUpdates", json!({})).await, Err(error));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
    server.reset().await;
    Mock::given(path("/bot123:control_test/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(20)))
        .mount(&server)
        .await;
    let pending = session.api(&app, "getUpdates", json!({}));
    tokio::pin!(pending);
    tokio::select! { result = &mut pending => panic!("unexpected {result:?}"), _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
    app.telegram.cancel_pending();
    assert_eq!(pending.await, Err(Failure::Cancelled));
    server.reset().await;
    app.telegram.disable_for_fixture();
    run(app.clone()).await;
    assert!(server.received_requests().await.unwrap().is_empty());
}
