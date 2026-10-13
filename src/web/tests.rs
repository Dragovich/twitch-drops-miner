use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use futures_util::{SinkExt, StreamExt};
use reqwest_websocket::{Message, Upgrade};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{App, router};

struct TestApp {
    app: Arc<App>,
    router: Router,
    _directory: tempfile::TempDir,
    worker: tokio::task::JoinHandle<()>,
}
impl TestApp {
    fn new(public: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (app, mut commands) = App::open(directory.path().to_owned(), public).unwrap();
        let worker = tokio::spawn(async move {
            while let Some(request) = commands.recv().await {
                let _ = request.complete.send(Ok(()));
            }
        });
        Self {
            router: router(app.clone()),
            app,
            _directory: directory,
            worker,
        }
    }
    async fn call(
        &self,
        method: Method,
        path: &str,
        data: Value,
        cookie: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, http::HeaderMap, Vec<u8>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:8080")
            .header("content-type", "application/json");
        if !cookie.is_empty() {
            builder = builder.header(header::COOKIE, format!("tdm_session={cookie}"));
        }
        for (key, value) in headers {
            builder = builder.header(*key, *value);
        }
        let response = self
            .router
            .clone()
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&data).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        (
            parts.status,
            parts.headers,
            to_bytes(body, 4 * 1024 * 1024).await.unwrap().to_vec(),
        )
    }
    async fn enable(&self) -> String {
        let (status,headers,_) = self.call(Method::POST,"/api/auth/settings",json!({"action":"enable","password":"test password","confirm_password":"test password"}),"",&[("x-tdm-request","1")]).await;
        assert_eq!(status, StatusCode::OK);
        cookie::Cookie::parse(headers[header::SET_COOKIE].to_str().unwrap())
            .unwrap()
            .value()
            .to_owned()
    }
}
impl Drop for TestApp {
    fn drop(&mut self) {
        self.app.shutdown.cancel();
        self.worker.abort();
    }
}

#[tokio::test]
async fn refresh_acknowledgement_does_not_mean_completion_and_requests_coalesce() {
    use crate::dto::RefreshState;
    let test = TestApp::new("");
    let headers = [("x-tdm-request", "1")];
    assert_eq!(
        test.call(Method::POST, "/api/reload", json!({}), "", &headers)
            .await
            .0,
        StatusCode::CONFLICT
    );
    test.app.snapshot.write().await.login.user_id = Some(42);
    for _ in 0..2 {
        assert_eq!(
            test.call(Method::POST, "/api/reload", json!({}), "", &headers)
                .await
                .0,
            StatusCode::OK
        );
        let state = test.app.snapshot.read().await;
        assert_eq!(state.inventory_refresh.sequence, 1);
        assert_eq!(state.inventory_refresh.state, RefreshState::Refreshing);
    }
    test.app.finish_inventory_refresh(1, None).await;
    assert_eq!(
        test.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Refreshed
    );
    test.app.refresh_inventory().await.unwrap();
    test.app.finish_inventory_refresh(1, None).await;
    assert_eq!(
        test.app.snapshot.read().await.inventory_refresh.state,
        RefreshState::Refreshing
    );
    test.app
        .finish_inventory_refresh(3, Some("catalog unavailable".into()))
        .await;
    let state = test.app.snapshot.read().await;
    assert_eq!(state.inventory_refresh.state, RefreshState::Failed);
    assert_eq!(
        state.inventory_refresh.error.as_deref(),
        Some("catalog unavailable")
    );
}

#[tokio::test]
async fn game_directory_requires_dashboard_auth_csrf_and_bounded_queries() {
    let test = TestApp::new("");
    let cookie = test.enable().await;
    let headers = [("x-tdm-request", "1")];
    assert_eq!(
        test.call(
            Method::POST,
            "/api/games",
            json!({"search":"Rust"}),
            "",
            &headers
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        test.call(
            Method::POST,
            "/api/games",
            json!({"search":"Rust"}),
            &cookie,
            &[]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for query in [
        json!({"search":""}),
        json!({"search":"x".repeat(101)}),
        json!({"names":[]}),
    ] {
        assert_eq!(
            test.call(Method::POST, "/api/games", query, &cookie, &headers)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        test.call(
            Method::POST,
            "/api/games",
            json!({"search":"Rust"}),
            &cookie,
            &headers
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn channel_search_requires_dashboard_auth_csrf_and_bounded_queries() {
    let test = TestApp::new("");
    let cookie = test.enable().await;
    let headers = [("x-tdm-request", "1")];
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/search",
            json!({"search":"Rust"}),
            "",
            &headers
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/search",
            json!({"search":"Rust"}),
            &cookie,
            &[]
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for query in [
        json!({"search":""}),
        json!({"search":"x".repeat(101)}),
        json!({"search":"streamer\n"}),
    ] {
        assert_eq!(
            test.call(
                Method::POST,
                "/api/channels/search",
                query,
                &cookie,
                &headers
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/search",
            json!({"search":"Rust"}),
            &cookie,
            &headers
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn refresh_command_failure_is_retryable_and_disconnected_callers_do_not_cancel_accepted_work()
{
    use crate::dto::RefreshState;
    let dir = tempfile::tempdir().unwrap();
    let (app, mut receiver) = App::open(dir.path().to_owned(), "").unwrap();
    app.snapshot.write().await.login.user_id = Some(42);
    let owned = app.clone();
    let caller = tokio::spawn(async move { owned.refresh_inventory().await });
    let command = receiver.recv().await.unwrap();
    caller.abort();
    let _ = caller.await;
    command.complete.send(Ok(())).unwrap();
    app.finish_inventory_refresh(1, None).await;
    drop(receiver);
    for _ in 0..2 {
        assert!(app.refresh_inventory().await.is_err());
        assert_eq!(
            app.snapshot.read().await.inventory_refresh.state,
            RefreshState::Failed
        );
    }
    app.drain_writes().await;
}

#[tokio::test]
async fn manual_channel_boundary_requires_login_and_rejects_arbitrary_urls() {
    let test = TestApp::new("");
    let headers = [("x-tdm-request", "1")];
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/select",
            json!({"channel":"streamer"}),
            "",
            &headers
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    test.app.snapshot.write().await.login.user_id = Some(42);
    for input in [
        "",
        "https://evil.test/private",
        "https://twitch.tv/user/videos",
        "http://localhost/private",
    ] {
        assert_eq!(
            test.call(
                Method::POST,
                "/api/channels/select",
                json!({"channel":input}),
                "",
                &headers
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/select",
            json!({"channel_id":999}),
            "",
            &headers
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        test.call(
            Method::POST,
            "/api/channels/select",
            json!({"channel":"https://www.twitch.tv/streamer"}),
            "",
            &headers
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(test.app.data.settings().unwrap().games_to_watch.is_empty());

    for duration in [
        json!(0),
        json!(-1),
        json!(1441),
        json!(1.5),
        json!("15"),
        json!(true),
    ] {
        assert_eq!(
            test.call(
                Method::POST,
                "/api/channels/select",
                json!({"channel":"streamer", "duration_minutes":duration}),
                "",
                &headers
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for duration in [Value::Null, json!(1), json!(1440)] {
        assert_eq!(
            test.call(
                Method::POST,
                "/api/channels/select",
                json!({"channel":"streamer", "duration_minutes":duration}),
                "",
                &headers
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    for selection in [json!({}), json!({"channel":"streamer", "channel_id":999})] {
        assert_eq!(
            test.call(
                Method::POST,
                "/api/channels/select",
                selection,
                "",
                &headers
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn public_assets_and_spa_allowlist_preserve_private_api_boundaries() {
    let test = TestApp::new("");
    assert_eq!(
        test.call(Method::GET, "/login", Value::Null, "", &[])
            .await
            .0,
        StatusCode::SEE_OTHER
    );
    let token = test.enable().await;
    for path in ["/", "/campaigns", "/history", "/activity", "/settings"] {
        let (status, headers, _) = test.call(Method::GET, path, Value::Null, "", &[]).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(headers[header::LOCATION], "/login");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let (status, headers, body) = test.call(Method::GET, path, Value::Null, &token, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        assert!(String::from_utf8(body).unwrap().contains("id=\"root\""));
    }
    for path in [
        "/api/status",
        "/api/channels",
        "/api/campaigns",
        "/api/console",
        "/api/settings",
        "/api/version",
        "/api/history",
        "/api/history/stats",
        "/socket.io/?EIO=4&transport=polling",
    ] {
        let (status, headers, _) = test.call(Method::GET, path, Value::Null, "", &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    for path in [
        "/api/reload",
        "/api/cache/clear",
        "/api/telegram",
        "/api/telegram/test",
        "/api/close",
        "/api/channels/select",
        "/api/mode/exit-manual",
        "/api/twitch/logout",
        "/api/oauth/confirm",
        "/api/settings",
        "/api/auth/logout",
        "/api/auth/settings",
    ] {
        assert_eq!(
            test.call(Method::POST, path, json!({}), "", &[("x-tdm-request", "1")])
                .await
                .0,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    for path in [
        "/api/login",
        "/api/settings/test-telegram",
        "/api/missing",
        "/unknown",
        "/__test/health",
        "/__test/reset",
    ] {
        assert_eq!(
            test.call(Method::GET, path, Value::Null, &token, &[])
                .await
                .0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let asset = super::Assets::iter()
        .find(|name| name.starts_with("assets/") && name.ends_with(".js"))
        .unwrap();
    let (_, headers, _) = test
        .call(Method::GET, &format!("/{asset}"), Value::Null, "", &[])
        .await;
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let logo = super::Assets::iter()
        .find(|name| name.starts_with("assets/twitch-drops-miner-logo-") && name.ends_with(".svg"))
        .expect("Vite emits the shared logo as a hashed asset");
    let (status, headers, body) = test
        .call(Method::GET, &format!("/{logo}"), Value::Null, "", &[])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert!(String::from_utf8(body).unwrap().contains("<svg"));
    assert_eq!(
        test.call(Method::GET, "/healthz", Value::Null, "", &[])
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn telegram_configuration_is_protected_and_tokens_are_write_only() {
    let test = TestApp::new("");
    let token = test.enable().await;
    for path in ["/api/telegram", "/api/telegram/test"] {
        assert_eq!(
            test.call(Method::POST, path, json!({}), "", &[("x-tdm-request", "1")])
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for path in ["/api/telegram", "/api/telegram/bot"] {
        assert_eq!(
            test.call(Method::GET, path, Value::Null, "", &[]).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (status, _, body) = test
        .call(Method::GET, "/api/telegram/bot", Value::Null, &token, &[])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(serde_json::from_slice::<Value>(&body).unwrap()["bot"].is_null());
    let revision = test.app.telegram.view().revision;
    let update =
        json!({"revision":revision,"enabled":true,"token":"123:private_token","chat_id":"42"});
    let (status, _, body) = test
        .call(
            Method::POST,
            "/api/telegram",
            update.clone(),
            &token,
            &[("x-tdm-request", "1")],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!String::from_utf8(body).unwrap().contains("private_token"));
    assert_eq!(
        test.call(
            Method::POST,
            "/api/telegram",
            update,
            &token,
            &[("x-tdm-request", "1")]
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    for path in ["/api/telegram", "/api/settings", "/api/status"] {
        let (status, _, body) = test.call(Method::GET, path, Value::Null, &token, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!String::from_utf8(body).unwrap().contains("private_token"));
    }
}

#[tokio::test]
async fn csrf_origin_and_secret_safe_validation_apply_before_mutation() {
    let test = TestApp::new("https://drops.example.com");
    for path in [
        "/api/reload",
        "/api/cache/clear",
        "/api/auth/settings",
        "/api/auth/login",
        "/api/oauth/confirm",
    ] {
        for headers in [
            vec![],
            vec![
                ("x-tdm-request", "1"),
                ("origin", "https://foreign.example"),
            ],
            vec![("x-tdm-request", "1"), ("sec-fetch-site", "cross-site")],
        ] {
            assert_eq!(
                test.call(Method::POST, path, json!({}), "", &headers)
                    .await
                    .0,
                StatusCode::FORBIDDEN
            );
        }
    }
    let (_, _, body) = test
        .call(
            Method::POST,
            "/api/auth/login",
            json!({"password":{"secret":"dont-echo-me"}}),
            "",
            &[("x-tdm-request", "1")],
        )
        .await;
    assert!(!String::from_utf8(body).unwrap().contains("dont-echo-me"));
    let token = test.enable().await;
    let (status, headers, _) = test
        .call(
            Method::POST,
            "/api/auth/logout",
            json!({}),
            &token,
            &[
                ("x-tdm-request", "1"),
                ("origin", "https://drops.example.com"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    assert!(
        headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let (status, _, _) = test
        .call(
            Method::POST,
            "/api/auth/login",
            json!({"password":"x".repeat(17000)}),
            "",
            &[("x-tdm-request", "1")],
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn concurrent_settings_edits_conflict_and_failed_writes_preserve_revision() {
    let test = TestApp::new("");
    let revision = test.app.snapshot.read().await.settings.revision.clone();
    let headers = [("x-tdm-request", "1")];
    let (first, second) = tokio::join!(
        test.call(
            Method::POST,
            "/api/settings",
            json!({"revision":revision,"games_to_watch":["Rust"]}),
            "",
            &headers
        ),
        test.call(
            Method::POST,
            "/api/settings",
            json!({"revision":revision,"games_to_watch":["Sea of Thieves"]}),
            "",
            &headers
        ),
    );
    assert!(matches!(
        (first.0, second.0),
        (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
    ));
    let state = test.app.snapshot.read().await;
    let saved = test.app.data.settings().unwrap();
    assert_eq!(saved.games_to_watch, state.settings.values.games_to_watch);
    assert_ne!(revision, state.settings.revision);
    let revision = state.settings.revision.clone();
    drop(state);
    std::fs::remove_file(test.app.data.path.join("settings.json")).unwrap();
    std::fs::create_dir(test.app.data.path.join("settings.json")).unwrap();
    let (status, _, _) = test
        .call(
            Method::POST,
            "/api/settings",
            json!({"revision":revision,"connection_quality":4}),
            "",
            &headers,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(test.app.snapshot.read().await.settings.revision, revision);
    assert_eq!(
        test.app
            .snapshot
            .read()
            .await
            .settings
            .values
            .connection_quality,
        saved.connection_quality
    );
}

#[tokio::test]
async fn invalid_settings_are_rejected_and_retired_credentials_are_not_exposed() {
    let test = TestApp::new("");
    for patch in [
        json!({"proxy":"http://user:secret@example/path"}),
        json!({"connection_quality":7}),
        json!({"inventory_filters":{"show_benefit_badge":"yes"}}),
    ] {
        let (status, _, body) = test
            .call(
                Method::POST,
                "/api/settings",
                patch,
                "",
                &[("x-tdm-request", "1")],
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!String::from_utf8(body).unwrap().contains("secret"));
    }
    let (status, _, body) = test
        .call(
            Method::POST,
            "/api/settings",
            json!({"telegram_bot_token":"secret","web_auth":false}),
            "",
            &[("x-tdm-request", "1")],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!String::from_utf8(body).unwrap().contains("secret"));
    assert!(
        !std::fs::read_to_string(test.app.data.path.join("settings.json"))
            .unwrap()
            .contains("telegram")
    );
}

#[tokio::test]
async fn settings_commit_survives_the_request_future_being_dropped() {
    let test = TestApp::new("");
    let locked = test.app.snapshot.write().await;
    let router = test.router.clone();
    let request = tokio::spawn(async move {
        router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/settings")
                    .header("host", "localhost:8080")
                    .header("x-tdm-request", "1")
                    .body(Body::from(r#"{"games_to_watch":["Rust"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !test.app.has_pending_writes() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    request.abort();
    let _ = request.await;
    drop(locked);
    test.app.drain_writes().await;
    assert_eq!(
        test.app
            .snapshot
            .read()
            .await
            .settings
            .values
            .games_to_watch,
        ["Rust"]
    );
    assert_eq!(test.app.data.settings().unwrap().games_to_watch, ["Rust"]);
}

#[tokio::test]
async fn history_dates_and_cache_clear_use_real_persistence() {
    let test = TestApp::new("");
    test.app.history.lock().await.record(serde_json::from_value(json!({"id":"reward","claimed_at":"2026-01-02T00:15:00Z","game":"Rüst / +","campaign":"Season","drop_name":"Jacket",
        "benefits":["Jacket"],"required_minutes":30,"campaign_id":"campaign"})).unwrap()).unwrap();
    for (since, expected) in [
        ("2026-01-02", 1),
        ("2026-01-02T00%3A15%3A00Z", 1),
        ("2026-01-02T02%3A00%3A00%2B01%3A00", 0),
    ] {
        let (_, _, body) = test
            .call(
                Method::GET,
                &format!("/api/history?since={since}"),
                Value::Null,
                "",
                &[],
            )
            .await;
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            expected
        );
    }
    for route in ["/api/history/export.csv", "/api/history/export.json"] {
        assert_eq!(
            test.call(Method::GET, route, Value::Null, "", &[]).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        test.call(
            Method::DELETE,
            "/api/history",
            json!({}),
            "",
            &[("x-tdm-request", "1")]
        )
        .await
        .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let entry = test
        .app
        .history
        .lock()
        .await
        .entries(&crate::store::HistoryFilter::default())[0]
        .clone();
    let settings = std::fs::read(test.app.data.path.join("settings.json")).ok();
    let session = test.app.data.path.join("twitch_session.json");
    std::fs::write(&session, "fixture credentials stay in place").unwrap();
    assert_eq!(
        test.call(
            Method::POST,
            "/api/cache/clear",
            json!({}),
            "",
            &[("x-tdm-request", "1")]
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut restored = crate::store::History::load(&test.app.data.path);
    assert_eq!(restored.total(), 0);
    assert!(
        !restored.record(entry).unwrap(),
        "cleared claims cannot be reimported after restart"
    );
    assert_eq!(
        std::fs::read(test.app.data.path.join("settings.json")).ok(),
        settings
    );
    assert_eq!(
        std::fs::read_to_string(session).unwrap(),
        "fixture credentials stay in place"
    );
}

#[tokio::test]
async fn cache_clear_fails_closed_for_unreadable_history() {
    let test = TestApp::new("");
    let path = test.app.data.path.join("drop_history.json");
    std::fs::write(&path, "unreadable history fixture").unwrap();
    *test.app.history.lock().await = crate::store::History::load(&test.app.data.path);
    assert_eq!(
        test.call(
            Method::POST,
            "/api/cache/clear",
            json!({}),
            "",
            &[("x-tdm-request", "1")]
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "unreadable history fixture"
    );
}

#[tokio::test]
async fn accepted_cache_clear_survives_client_disconnect() {
    let test = TestApp::new("");
    let mut history = test.app.history.lock().await;
    history.record(serde_json::from_value(json!({"id":"reward","claimed_at":"2026-01-02T00:15:00Z","game":"Rust","campaign":"Season","drop_name":"Jacket","benefits":["Jacket"],"required_minutes":30,"campaign_id":"campaign"})).unwrap()).unwrap();
    let app = test.app.clone();
    let request = tokio::spawn(async move { app.clear_cache().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !test.app.has_pending_writes() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    request.abort();
    let _ = request.await;
    drop(history);
    test.app.drain_writes().await;
    assert_eq!(crate::store::History::load(&test.app.data.path).total(), 0);
}

async fn websocket_text(socket: &mut reqwest_websocket::WebSocket) -> String {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            if text == "2" {
                socket.send(Message::Text("3".into())).await.unwrap();
                continue;
            }
            return text;
        }
    }
}

#[tokio::test]
async fn both_socket_transports_enforce_origin_and_revocation_before_private_events() {
    let test = TestApp::new("https://drops.example.com");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = test.router.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .http1_only()
        .build()
        .unwrap();
    let url = format!("http://{address}/socket.io/?EIO=4&transport=");
    // An anonymous connection must be evicted when password protection is enabled.
    let mut anonymous = client
        .get(format!("{url}websocket"))
        .header("Origin", "https://drops.example.com")
        .upgrade()
        .send()
        .await
        .unwrap()
        .into_websocket()
        .await
        .unwrap();
    assert!(websocket_text(&mut anonymous).await.starts_with('0'));
    anonymous.send(Message::Text("40".into())).await.unwrap();
    let handshake = [
        websocket_text(&mut anonymous).await,
        websocket_text(&mut anonymous).await,
    ];
    assert!(
        handshake.iter().any(|message| message.starts_with("40")),
        "{handshake:?}"
    );
    assert!(
        handshake
            .iter()
            .any(|message| message.contains("initial_state")),
        "{handshake:?}"
    );
    let token = test.enable().await;
    assert_eq!(websocket_text(&mut anonymous).await, "41");
    assert_eq!(
        client
            .get(format!("{url}polling"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for transport in ["polling", "websocket"] {
        let response = client
            .get(format!("{url}{transport}"))
            .header("Cookie", format!("tdm_session={token}"))
            .header("Origin", "https://foreign.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        client
            .get(format!("{url}polling"))
            .header("Cookie", format!("tdm_session={token}"))
            .header("Origin", "https://drops.example.com")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let mut socket = client
        .get(format!("{url}websocket&protocol=2"))
        .header("Cookie", format!("tdm_session={token}"))
        .header("Origin", "https://drops.example.com")
        .upgrade()
        .send()
        .await
        .unwrap()
        .into_websocket()
        .await
        .unwrap();
    assert!(websocket_text(&mut socket).await.starts_with('0'));
    socket.send(Message::Text("40".into())).await.unwrap();
    let handshake = [
        websocket_text(&mut socket).await,
        websocket_text(&mut socket).await,
    ];
    assert!(
        handshake.iter().any(|message| message.starts_with("40")),
        "{handshake:?}"
    );
    assert!(
        handshake
            .iter()
            .any(|message| message.contains("state_snapshot")),
        "{handshake:?}"
    );
    let initial: Value = serde_json::from_str(
        &handshake
            .iter()
            .find(|message| message.starts_with("42"))
            .unwrap()[2..],
    )
    .unwrap();
    let mut revision = initial[1]["revision"].as_u64().unwrap();
    for count in 1..=8 {
        let mut state = test.app.snapshot.write().await;
        state.channels = vec![crate::dto::ChannelView {
            id: 7,
            viewers: Some(count),
            ..Default::default()
        }];
        state.status = format!("published-{count}");
    }
    let latest = test.app.snapshot.read().await.revision;
    while revision < latest {
        let packet = websocket_text(&mut socket).await;
        let packet: Value = serde_json::from_str(&packet[2..]).unwrap();
        assert_eq!(packet[0], "state_patch");
        assert_eq!(packet[1]["base_revision"], revision);
        assert_eq!(packet[1]["instance"], initial[1]["instance"]);
        assert!(packet[1]["changes"].get("campaigns").is_none());
        revision = packet[1]["revision"].as_u64().unwrap();
        if revision == latest {
            assert_eq!(packet[1]["changes"]["channels"][0]["viewers"], 8);
            assert_eq!(packet[1]["changes"]["status"], "published-8");
        }
    }
    socket
        .send(Message::Text("42[\"state_resync\"]".into()))
        .await
        .unwrap();
    let full = websocket_text(&mut socket).await;
    let full: Value = serde_json::from_str(&full[2..]).unwrap();
    assert_eq!(full[0], "state_snapshot");
    assert_eq!(full[1]["revision"], latest);
    test.app.auth.logout(&token).await.unwrap();
    test.app.snapshot.write().await.status = "private-sentinel".into();
    assert_eq!(websocket_text(&mut socket).await, "41");
    test.app.sockets.close().await;
    server.abort();
}

#[tokio::test]
async fn responses_forbid_sniffing_and_documents_restrict_what_they_load() {
    let test = TestApp::new("");
    let asset = super::Assets::iter()
        .find(|name| name.starts_with("assets/") && name.ends_with(".js"))
        .unwrap();
    let asset = format!("/{asset}");
    for path in [
        "/",
        "/api/status",
        "/healthz",
        "/api/missing",
        asset.as_str(),
    ] {
        let (_, headers, _) = test.call(Method::GET, path, Value::Null, "", &[]).await;
        assert_eq!(
            headers.get("x-content-type-options").map(|v| v.as_bytes()),
            Some(&b"nosniff"[..]),
            "{path}"
        );
    }
    let (status, headers, _) = test
        .call(Method::POST, "/api/reload", json!({}), "", &[])
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(headers["x-content-type-options"], "nosniff");

    // Socket.IO is served by a layer inside the same guard.
    let (_, headers, _) = test
        .call(
            Method::GET,
            "/socket.io/?EIO=4&transport=polling",
            Value::Null,
            "",
            &[],
        )
        .await;
    assert_eq!(headers["x-content-type-options"], "nosniff");

    let policy = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; font-src 'self' data:; connect-src 'self'; object-src 'none'; frame-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";
    for path in ["/", "/campaigns", "/history", "/activity", "/settings"] {
        let (status, headers, _) = test.call(Method::GET, path, Value::Null, "", &[]).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(headers[header::CONTENT_SECURITY_POLICY], policy, "{path}");
    }
    let token = test.enable().await;
    // The public login document is protected too, and so are redirects to it.
    let (status, headers, _) = test.call(Method::GET, "/login", Value::Null, "", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], policy);
    assert_eq!(headers["x-content-type-options"], "nosniff");
    let (status, headers, _) = test.call(Method::GET, "/", Value::Null, "", &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers["x-content-type-options"], "nosniff");
    let (status, headers, _) = test
        .call(Method::GET, "/settings", Value::Null, &token, &[])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], policy);
}
