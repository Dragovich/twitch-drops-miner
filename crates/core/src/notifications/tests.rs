use super::*;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn update(service: &Telegram) -> Update {
    Update {
        revision: service.view().revision,
        enabled: true,
        token: "123:secret_token".into(),
        chat_id: "42".into(),
        remove: false,
    }
}

#[tokio::test]
async fn private_config_preserves_tokens_and_rejects_stale_or_invalid_updates() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = Application::open(dir.path().into()).unwrap();
    let old = app.telegram.view();
    assert!(!old.enabled);
    let saved = app.save_telegram(update(&app.telegram)).await.unwrap();
    let public = serde_json::to_string(&saved).unwrap();
    assert!(!public.contains("secret_token"));
    assert!(
        !serde_json::to_string(&*app.snapshot.read().await)
            .unwrap()
            .contains("secret_token")
    );
    assert!(saved.enabled && saved.token_configured);
    let mut changed = update(&app.telegram);
    changed.token.clear();
    changed.chat_id = "-1234".into();
    app.save_telegram(changed).await.unwrap();
    let loaded = Telegram::open(dir.path()).unwrap();
    assert_eq!(
        loaded.state.lock().unwrap().config.token,
        "123:secret_token"
    );
    let mut stale = update(&app.telegram);
    stale.revision = old.revision;
    assert!(matches!(
        app.save_telegram(stale).await,
        Err(crate::app::AppError::Telegram(Error::Conflict))
    ));
    let mut invalid = update(&app.telegram);
    invalid.token = "bad/token".into();
    assert!(app.save_telegram(invalid).await.is_err());
    let mut removed = update(&app.telegram);
    removed.remove = true;
    let view = app.save_telegram(removed).await.unwrap();
    assert!(!view.enabled && !view.token_configured && view.chat_id.is_empty());
}

#[test]
fn unreadable_config_and_failed_writes_preserve_original_state() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("telegram.json"), "{").unwrap();
    let service = Telegram::open(dir.path()).unwrap();
    assert!(service.view().unreadable);
    assert!(matches!(
        service.save(update(&service)),
        Err(Error::Storage)
    ));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("telegram.json")).unwrap(),
        "{"
    );
    let other = tempfile::tempdir().unwrap();
    let service = Telegram::open(other.path()).unwrap();
    std::fs::create_dir(other.path().join("telegram.json")).unwrap();
    let revision = service.view().revision;
    assert!(matches!(
        service.save(update(&service)),
        Err(Error::Storage)
    ));
    assert_eq!(service.view().revision, revision);
    assert!(!service.view().enabled);
}

async fn configured(server: &MockServer) -> (tempfile::TempDir, Telegram) {
    let dir = tempfile::tempdir().unwrap();
    let mut service = Telegram::open(dir.path()).unwrap();
    service.endpoint = server.uri();
    service.save(update(&service)).unwrap();
    (dir, service)
}

#[tokio::test]
async fn sends_plain_text_without_twitch_credentials_and_checks_api_success() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    Mock::given(method("POST"))
        .and(path("/bot123:secret_token/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    service
        .send(
            service.epoch(),
            "<Reward> & game",
            "",
            &CancellationToken::new(),
            false,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("cookie"));
    assert!(!request.headers.contains_key("client-id"));
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["text"], "<Reward> & game");
    assert!(body.get("parse_mode").is_none());
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":false,"description":"secret_token"})),
        )
        .mount(&server)
        .await;
    service.state.lock().unwrap().next_send = Instant::now();
    let error = service
        .send(
            service.epoch(),
            "test",
            "",
            &CancellationToken::new(),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(error, Error::Delivery);
    assert!(!error.to_string().contains("secret_token"));
}

#[tokio::test]
async fn test_delivery_does_not_discard_claims_and_clears_old_errors() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = Application::open(dir.path().into()).unwrap();
    Arc::get_mut(&mut app).unwrap().telegram.endpoint = server.uri();
    app.save_telegram(update(&app.telegram)).await.unwrap();
    app.snapshot.write().await.login.user_id = Some(1);
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true}))
                .set_delay(Duration::from_millis(100)),
        )
        .mount(&server)
        .await;
    report(&app, Err(Error::Delivery)).await;
    let events = app.notifications.subscribe();
    let worker = tokio::spawn(run(app.clone(), events));
    let sending = app.clone();
    let test = tokio::spawn(async move { sending.test_telegram().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        app.test_telegram().await,
        Err(crate::app::AppError::Telegram(Error::Busy))
    ));
    let event = Notification {
        title: "Drop claimed".into(),
        message: "Reward".into(),
        claim: Some(Claim {
            account: 1,
            drop_id: "new-drop".into(),
            game: "Game".into(),
            campaign: "Campaign".into(),
            reward: "Reward".into(),
            epoch: app.telegram.epoch(),
        }),
    };
    assert!(app.notifications.send(event).is_ok());
    test.await.unwrap().unwrap();
    assert!(
        app.snapshot
            .read()
            .await
            .activity
            .iter()
            .any(|event| event.code == "gui.backend.telegram_delivery_failed" && event.recovered)
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.received_requests().await.unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    report(&app, Err(Error::Delivery)).await;
    assert!(!app.snapshot.read().await.activity.last().unwrap().recovered);
}

#[tokio::test]
async fn rejected_and_ambiguous_responses_are_not_retried_or_followed() {
    let server = MockServer::start().await;
    for (response, expected) in [
        (ResponseTemplate::new(401), Error::Token),
        (ResponseTemplate::new(403), Error::Chat),
        (
            ResponseTemplate::new(500).set_body_string("secret_token"),
            Error::Delivery,
        ),
        (
            ResponseTemplate::new(302).insert_header("location", format!("{}/leak", server.uri())),
            Error::Delivery,
        ),
        (
            ResponseTemplate::new(200).set_body_string("x".repeat(65537)),
            Error::Delivery,
        ),
    ] {
        server.reset().await;
        let (_dir, service) = configured(&server).await;
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(&server)
            .await;
        assert_eq!(
            service
                .send(
                    service.epoch(),
                    "test",
                    "",
                    &CancellationToken::new(),
                    false
                )
                .await,
            Err(expected)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn explicit_rate_limit_retries_once_and_configuration_change_cancels_delivery() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({"ok":false,"parameters":{"retry_after":1}})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .with_priority(10)
        .mount(&server)
        .await;
    service
        .send(
            service.epoch(),
            "test",
            "",
            &CancellationToken::new(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    let epoch = service.epoch();
    service.cancel_pending();
    assert_eq!(
        service
            .send(epoch, "stale", "", &CancellationToken::new(), false)
            .await,
        Err(Error::Cancelled)
    );
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    service.state.lock().unwrap().next_send = Instant::now();
    let cancel = CancellationToken::new();
    let send = service.send(service.epoch(), "test", "", &cancel, false);
    tokio::pin!(send);
    tokio::select! { result = &mut send => panic!("completed early: {result:?}"), _ = tokio::time::sleep(Duration::from_millis(50)) => {} }
    service.save(update(&service)).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), send)
            .await
            .unwrap(),
        Err(Error::Cancelled)
    );
}

#[tokio::test]
async fn worker_deduplicates_claims_and_never_exposes_internal_metadata() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _) = Application::open(dir.path().into()).unwrap();
    Arc::get_mut(&mut app).unwrap().telegram.endpoint = server.uri();
    app.save_telegram(update(&app.telegram)).await.unwrap();
    app.snapshot.write().await.login.user_id = Some(1);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    let events = app.notifications.subscribe();
    let worker = tokio::spawn(run(app.clone(), events));
    let event = Notification {
        title: "Drop claimed".into(),
        message: "Reward".into(),
        claim: Some(Claim {
            account: 1,
            drop_id: "drop".into(),
            game: "Game".into(),
            campaign: "Campaign".into(),
            reward: "Reward".into(),
            epoch: app.telegram.epoch(),
        }),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        json!({"title":"Drop claimed","message":"Reward"})
    );
    assert!(app.notifications.send(event.clone()).is_ok());
    assert!(app.notifications.send(event).is_ok());
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    app.shutdown.cancel();
    worker.await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
