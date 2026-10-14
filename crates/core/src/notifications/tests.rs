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
        remote_control: false,
        include_image: false,
        token: "123:secret_token".into(),
        chat_id: "42".into(),
        remove: false,
    }
}

#[tokio::test]
async fn offline_fixture_cannot_deliver_even_after_configuration_changes() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut service = Telegram::open(dir.path()).unwrap();
    service.endpoint = server.uri();
    service.disable_for_fixture();
    service.save(update(&service)).unwrap();
    for test in [MessageKind::Test, MessageKind::Claim, MessageKind::Reply] {
        assert_eq!(
            service
                .send(
                    service.epoch(),
                    "offline",
                    "",
                    &CancellationToken::new(),
                    test,
                    None,
                )
                .await,
            Err(Error::Disabled)
        );
    }
    assert!(
        service
            .bot("", &CancellationToken::new())
            .await
            .bot
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn bot_lookup_caches_safe_public_identity_without_twitch_credentials() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    Mock::given(method("GET"))
        .and(path("/bot123:secret_token/getMe"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{
                "id":123,"is_bot":true,"first_name":"Drops <&> Miner","username":"drops_miner_bot"
            }})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let cancel = CancellationToken::new();
    let (first, second) = tokio::join!(service.bot("", &cancel), service.bot("", &cancel));
    for view in [first, second] {
        let public = serde_json::to_value(view).unwrap();
        assert_eq!(public["revision"], service.view().revision);
        assert_eq!(
            public["bot"],
            json!({"name":"Drops <&> Miner","username":"drops_miner_bot"})
        );
        assert!(!public.to_string().contains("secret_token"));
    }
    let requests = server.received_requests().await.unwrap();
    for header in ["authorization", "cookie", "client-id"] {
        assert!(!requests[0].headers.contains_key(header));
    }
    assert!(requests[0].body.is_empty());
}

#[tokio::test]
async fn bot_lookup_ignores_invalid_or_failed_responses_without_affecting_saved_settings() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    let valid = json!({"ok":true,"result":{
        "id":123,"is_bot":true,"first_name":"Drops Miner","username":"drops_miner_bot"
    }});
    let mut responses = vec![
        ResponseTemplate::new(401),
        ResponseTemplate::new(302).insert_header("location", "/redirected"),
        ResponseTemplate::new(200).set_body_string("not JSON"),
        ResponseTemplate::new(200).set_body_string("x".repeat(65537)),
    ];
    for (key, value) in [
        ("username", json!("../bad")),
        ("id", json!(456)),
        ("is_bot", json!(false)),
        ("first_name", json!("x".repeat(65))),
    ] {
        let mut invalid = valid.clone();
        invalid["result"][key] = value;
        responses.push(ResponseTemplate::new(200).set_body_json(invalid));
    }
    for response in responses {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/bot123:secret_token/getMe"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let revision = service.view().revision;
        assert!(
            service
                .bot("", &CancellationToken::new())
                .await
                .bot
                .is_none()
        );
        assert_eq!(service.view().revision, revision);
        assert!(service.view().enabled && service.view().token_configured);
        assert!(service.state.lock().unwrap().bot.is_none());
    }
}

#[tokio::test]
async fn bot_lookup_cancels_old_tokens_and_shutdown_and_times_out() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    let service = Arc::new(service);
    Mock::given(method("GET"))
        .and(path("/bot123:secret_token/getMe"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{
                    "id":123,"is_bot":true,"first_name":"Old bot","username":"old_bot"
                }}))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;
    let original = service.view().revision;
    let querying = service.clone();
    let lookup = tokio::spawn(async move { querying.bot("", &CancellationToken::new()).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut config = update(&service);
    config.token = "456:new_token".into();
    service.save(config).unwrap();
    let stale = tokio::time::timeout(Duration::from_secs(1), lookup)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.revision, original);
    assert!(stale.bot.is_none());
    assert!(service.state.lock().unwrap().bot.is_none());

    Mock::given(method("GET"))
        .and(path("/bot456:new_token/getMe"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{
                "id":456,"is_bot":true,"first_name":"New bot","username":"new_bot"
            }})),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        service
            .bot("", &CancellationToken::new())
            .await
            .bot
            .unwrap()
            .username,
        "new_bot"
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(service.bot("", &cancel).await.bot.is_none());
    service.save(update(&service)).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(4),
            service.bot("", &CancellationToken::new())
        )
        .await
        .unwrap()
        .bot
        .is_none()
    );
}

#[tokio::test]
async fn private_config_preserves_tokens_and_rejects_stale_or_invalid_updates() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = Application::open(dir.path().into()).unwrap();
    let old = app.telegram.view();
    assert!(!old.enabled);
    assert!(!old.include_image);
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
    changed.include_image = true;
    app.save_telegram(changed).await.unwrap();
    let loaded = Telegram::open(dir.path()).unwrap();
    assert!(loaded.view().include_image);
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
    assert!(!view.include_image);
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
async fn sends_html_without_twitch_credentials_and_checks_api_success() {
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
            "<b>&lt;Reward&gt;</b> &amp; game",
            "",
            &CancellationToken::new(),
            MessageKind::Claim,
            None,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let request = &requests[0];
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("cookie"));
    assert!(!request.headers.contains_key("client-id"));
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["text"], "<b>&lt;Reward&gt;</b> &amp; game");
    assert_eq!(body["parse_mode"], "HTML");
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
            MessageKind::Claim,
            None,
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
    let mut config = update(&app.telegram);
    config.include_image = true;
    app.save_telegram(config).await.unwrap();
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
            image_url: "https://static-cdn.jtvnw.net/reward.png".into(),
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
            Error::Oversized,
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
                    MessageKind::Claim,
                    None,
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
            MessageKind::Claim,
            None,
        )
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    let epoch = service.epoch();
    service.cancel_pending();
    assert_eq!(
        service
            .send(
                epoch,
                "stale",
                "",
                &CancellationToken::new(),
                MessageKind::Claim,
                None
            )
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
    let send = service.send(
        service.epoch(),
        "test",
        "",
        &cancel,
        MessageKind::Claim,
        None,
    );
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
    let mut config = update(&app.telegram);
    config.include_image = true;
    app.save_telegram(config).await.unwrap();
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
            image_url: "https://static-cdn.jtvnw.net/reward.png".into(),
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
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["text"],
        "Drop claimed\n<b>Game</b> \u{2014} Campaign\nReward: <b>Reward</b>"
    );
    assert_eq!(body["parse_mode"], "HTML");
    assert_eq!(
        body["link_preview_options"]["url"],
        "https://static-cdn.jtvnw.net/reward.png"
    );
}

#[test]
fn existing_telegram_config_defaults_to_text_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("telegram.json"),
        serde_json::to_vec(&json!({"enabled":true,"token":"123:secret","chat_id":"42"})).unwrap(),
    )
    .unwrap();
    let service = Telegram::open(dir.path()).unwrap();
    assert!(service.view().enabled);
    assert!(!service.view().unreadable);
    assert!(!service.view().include_image);
}

#[tokio::test]
async fn optional_reward_preview_preserves_text_and_only_shares_public_twitch_artwork() {
    let server = MockServer::start().await;
    let (_dir, service) = configured(&server).await;
    let artwork = "https://static-cdn.jtvnw.net/twitch-quests-assets/REWARD/reward.png";
    for (enabled, image, expected) in [
        (false, Some(artwork), false),
        (true, Some(artwork), true),
        (true, None, false),
        (true, Some(""), false),
        (true, Some("http://static-cdn.jtvnw.net/reward.png"), false),
        (
            true,
            Some("https://static-cdn.jtvnw.net.evil.test/reward.png"),
            false,
        ),
        (
            true,
            Some("https://user:secret@static-cdn.jtvnw.net/reward.png"),
            false,
        ),
        (
            true,
            Some("https://static-cdn.jtvnw.net/reward.png?token=secret"),
            false,
        ),
        (
            true,
            Some("https://static-cdn.jtvnw.net/reward.png#secret"),
            false,
        ),
        (
            true,
            Some("https://static-cdn.jtvnw.net:8443/reward.png"),
            false,
        ),
        (true, Some("https://127.0.0.1/reward.png"), false),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/bot123:secret_token/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .mount(&server)
            .await;
        let mut config = update(&service);
        config.include_image = enabled;
        service.save(config).unwrap();
        service.state.lock().unwrap().next_send = Instant::now();
        service
            .send(
                service.epoch(),
                "Drop claimed\n<b>Game</b> \u{2014} Campaign\nReward: <b>&lt;Reward&gt;</b>",
                "",
                &CancellationToken::new(),
                MessageKind::Claim,
                image,
            )
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body["text"],
            "Drop claimed\n<b>Game</b> \u{2014} Campaign\nReward: <b>&lt;Reward&gt;</b>"
        );
        assert_eq!(body["parse_mode"], "HTML");
        assert!(body.get("reply_markup").is_none());
        assert_eq!(
            body["link_preview_options"],
            if expected {
                json!({"url":artwork,"prefer_small_media":true,"show_above_text":false})
            } else {
                json!({"is_disabled":true})
            }
        );
    }
    assert!(
        preview_url(&format!(
            "https://static-cdn.jtvnw.net/{}",
            "x".repeat(2048)
        ))
        .is_none()
    );
}

#[test]
fn claim_message_escapes_names_and_bounds_them_before_formatting() {
    let mut claim = Claim {
        account: 1,
        drop_id: "drop".into(),
        game: "Shakes & Fidget".into(),
        campaign: "<Campaign> &amp;".into(),
        reward: "</b>\u{1f3c6}<b>".into(),
        image_url: String::new(),
        epoch: 0,
    };
    assert_eq!(
        claim.message(crate::config::Language::En),
        "Drop claimed\n<b>Shakes &amp; Fidget</b> \u{2014} &lt;Campaign&gt; &amp;amp;\nReward: <b>&lt;/b&gt;\u{1f3c6}&lt;b&gt;</b>"
    );
    assert_eq!(
        claim.message(crate::config::Language::Ru),
        "Дроп получен\n<b>Shakes &amp; Fidget</b> \u{2014} &lt;Campaign&gt; &amp;amp;\nНаграда: <b>&lt;/b&gt;\u{1f3c6}&lt;b&gt;</b>"
    );

    claim.game = "\u{1f3c6}".repeat(2000);
    claim.campaign = "<".repeat(2000);
    claim.reward = "&".repeat(2000);
    assert_eq!(
        claim.message(crate::config::Language::En),
        format!(
            "Drop claimed\n<b>{}</b> \u{2014} {}\nReward: <b>{}</b>",
            "\u{1f3c6}".repeat(480),
            "&lt;".repeat(480),
            "&amp;".repeat(480)
        )
    );
}
