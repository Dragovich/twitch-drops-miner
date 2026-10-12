use super::*;
use wiremock::matchers::{body_string_contains, header};

pub(super) async fn calls(server: &MockServer, operation: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| {
            request.url.path() == operation
                || request
                    .body_json::<serde_json::Value>()
                    .ok()
                    .is_some_and(|body| body["operationName"] == operation)
        })
        .count()
}

async fn wait_calls(server: &MockServer, operation: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while calls(server, operation).await < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

pub(super) async fn wait_for(app: &App, condition: impl Fn(&crate::dto::Snapshot) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if condition(&*app.snapshot.read().await) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn advance(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}

async fn owner(
    server: &MockServer,
    paused: bool,
) -> (
    tempfile::TempDir,
    Arc<App>,
    JoinHandle<Result<(), TwitchError>>,
) {
    gql_mock(server, |q| match q["operationName"].as_str().unwrap() {
        "Inventory" => json!({"data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[],"gameEventDrops":[]}}}}),
        "AccountProfile" | "AccountBadges" => json!({"data":null}),
        other => panic!("unexpected operation {other}"),
    }).await;
    let dir = tempfile::tempdir().unwrap();
    let (app, commands) = App::open(dir.path().to_owned()).unwrap();
    {
        let mut settings = app.settings.write().await;
        settings.minimum_refresh_interval_minutes = 90;
        settings.mining_paused = paused;
    }
    session().save(dir.path()).unwrap();
    let worker = tokio::spawn(
        Miner {
            app: app.clone(),
            commands,
            endpoints: Endpoints::mock(&server.uri()),
        }
        .run(),
    );
    wait_for(&app, |state| state.inventory_status.checked_at.is_some()).await;
    (dir, app, worker)
}

async fn stop(app: &App, worker: JoinHandle<Result<(), TwitchError>>) {
    app.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn valid(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn idle_and_paused_validation_leave_routine_and_manual_refresh_independent() {
    for paused in [false, true] {
        let server = MockServer::start().await;
        valid(&server).await;
        let (_dir, app, worker) = owner(&server, paused).await;
        let checked = app.snapshot.read().await.inventory_status.checked_at;
        advance(3601).await;
        wait_calls(&server, "AccountProfile", 2).await;
        assert_eq!(calls(&server, "/oauth2/validate").await, 2);
        assert_eq!(calls(&server, "Inventory").await, 1);
        assert_eq!(calls(&server, "/catalog").await, 1);
        assert_eq!(
            app.snapshot.read().await.inventory_status.checked_at,
            checked
        );
        // The original 90-minute deadline still applies after the hourly check.
        advance(1800).await;
        wait_for(&app, |state| state.inventory_status.checked_at > checked).await;
        assert_eq!(calls(&server, "Inventory").await, 2);
        let checked = app.snapshot.read().await.inventory_status.checked_at;
        app.command(Command::Refresh { clear_cache: false })
            .await
            .unwrap();
        wait_for(&app, |state| state.inventory_status.checked_at > checked).await;
        assert_eq!(calls(&server, "Inventory").await, 3);
        assert_eq!(calls(&server, "/oauth2/validate").await, 2);
        stop(&app, worker).await;
    }
}

#[tokio::test]
async fn validation_failure_retries_without_resetting_inventory_or_credentials() {
    let server = MockServer::start().await;
    valid(&server).await;
    let (dir, app, worker) = owner(&server, false).await;
    let credentials = std::fs::read(dir.path().join("twitch_session.json")).unwrap();
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_string("interrupted validation"))
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&server)
        .await;
    advance(3601).await;
    wait_for(&app, |state| {
        state.activity.iter().any(|event| {
            event
                .args
                .get("operation")
                .is_some_and(|name| name == "validation")
                && !event.recovered
        })
    })
    .await;
    assert_eq!(calls(&server, "Inventory").await, 1);
    assert_eq!(calls(&server, "AccountProfile").await, 1);
    assert_eq!(calls(&server, "/oauth2/token").await, 0);
    assert_eq!(app.snapshot.read().await.login.user_id, Some(42));
    advance(59).await;
    assert_eq!(calls(&server, "/oauth2/validate").await, 2);
    advance(2).await;
    wait_calls(&server, "AccountProfile", 2).await;
    wait_for(&app, |state| {
        state.activity.iter().any(|event| {
            event
                .args
                .get("operation")
                .is_some_and(|name| name == "validation")
                && event.recovered
        })
    })
    .await;
    assert_eq!(calls(&server, "/oauth2/validate").await, 3);
    assert_eq!(calls(&server, "Inventory").await, 1);
    assert_eq!(
        std::fs::read(dir.path().join("twitch_session.json")).unwrap(),
        credentials
    );
    stop(&app, worker).await;
}

#[tokio::test]
async fn rejected_hourly_token_renews_through_existing_authentication() {
    use crate::app::activity::{ActivityEvent, Category, Severity};
    let server = MockServer::start().await;
    valid(&server).await;
    let (dir, app, worker) = owner(&server, false).await;
    app.record_activity(ActivityEvent::new(
        "gui.backend.twitch_error",
        Category::Connection,
        Severity::Warning,
        &[
            ("operation", "validation"),
            ("error", "Twitch connection failed"),
        ],
    ))
    .await;
    let checked = app.snapshot.read().await.inventory_status.checked_at;
    Mock::given(method("GET"))
        .and(path("/oauth2/validate"))
        .and(header("Authorization", "OAuth testtoken"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"status":401,"message":"invalid token"})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=testrefresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token":"newtoken", "refresh_token":"newrefresh", "expires_in":3600
        })))
        .expect(1)
        .mount(&server)
        .await;
    advance(3601).await;
    wait_for(&app, |state| state.inventory_status.checked_at > checked).await;
    assert_eq!(
        Session::load(dir.path()).unwrap().unwrap().access_token,
        "newtoken"
    );
    assert_eq!(app.snapshot.read().await.login.user_id, Some(42));
    assert!(app.snapshot.read().await.activity.iter().any(|event| {
        event
            .args
            .get("operation")
            .is_some_and(|operation| operation == "validation")
            && event.recovered
    }));
    assert_eq!(calls(&server, "/oauth2/device").await, 0);
    let requests = server.received_requests().await.unwrap();
    let renewed = requests
        .iter()
        .rposition(|r| r.url.path() == "/oauth2/token")
        .unwrap();
    assert!(
        requests[renewed + 1..]
            .iter()
            .filter(|r| r.url.path() == "/gql")
            .all(|r| r.headers["authorization"] == "OAuth newtoken")
    );
    stop(&app, worker).await;
}

#[tokio::test]
async fn hourly_validation_rejects_revoked_sessions_and_identity_changes() {
    for failure in ["revoked", "user_id", "client_id"] {
        let server = MockServer::start().await;
        valid(&server).await;
        let (dir, app, worker) = owner(&server, false).await;
        let mut response = validation();
        let status = if failure == "revoked" {
            response = json!({"status":401,"message":"invalid token"});
            401
        } else {
            response[failure] = "999".into();
            200
        };
        Mock::given(method("GET"))
            .and(path("/oauth2/validate"))
            .respond_with(ResponseTemplate::new(status).set_body_json(response))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"status":400,"message":"invalid refresh token"})),
            )
            .mount(&server)
            .await;
        advance(3601).await;
        wait_for(&app, |state| state.login.user_id.is_none()).await;
        assert!(Session::load(dir.path()).unwrap().is_none());
        assert_eq!(
            calls(&server, "/oauth2/token").await,
            usize::from(failure == "revoked")
        );
        assert_eq!(calls(&server, "Inventory").await, 1);
        stop(&app, worker).await;
    }
}

#[tokio::test]
async fn logout_and_shutdown_cancel_inflight_validation_without_late_credential_writes() {
    for logout in [false, true] {
        let server = MockServer::start().await;
        valid(&server).await;
        let (dir, app, worker) = owner(&server, false).await;
        Mock::given(method("GET"))
            .and(path("/oauth2/validate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(validation())
                    .set_delay(Duration::from_secs(30)),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        advance(3601).await;
        wait_calls(&server, "/oauth2/validate", 2).await;
        // A pending check must not hold up mining's command/inventory loop.
        let checked = app.snapshot.read().await.inventory_status.checked_at;
        app.command(Command::Refresh { clear_cache: false })
            .await
            .unwrap();
        wait_for(&app, |state| state.inventory_status.checked_at > checked).await;
        assert_eq!(calls(&server, "AccountProfile").await, 1);
        if logout {
            tokio::time::timeout(Duration::from_secs(3), app.command(Command::Logout))
                .await
                .unwrap()
                .unwrap();
        }
        stop(&app, worker).await;
        assert_eq!(Session::load(dir.path()).unwrap().is_none(), logout);
        assert_eq!(calls(&server, "/oauth2/token").await, 0);
        assert_eq!(calls(&server, "Inventory").await, 2);
    }
}

#[tokio::test]
async fn validation_and_shutdown_wait_for_inflight_history_writes() {
    for renew in [false, true] {
        let server = MockServer::start().await;
        valid(&server).await;
        let (dir, app, worker) = owner(&server, false).await;
        let mut campaign = campaign_json("claimed");
        campaign["timeBasedDrops"][0]["self"]["isClaimed"] = true.into();
        Mock::given(method("POST")).and(path("/gql"))
            .and(body_string_contains("Inventory"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":{"currentUser":{"inventory":{"dropCampaignsInProgress":[campaign],"gameEventDrops":[]}}}
            }))).with_priority(1).mount(&server).await;
        let history = app.history.lock().await;
        app.command(Command::Refresh { clear_cache: false })
            .await
            .unwrap();
        wait_calls(&server, "Inventory", 2).await;
        // Let the local response reach the blocking history transaction held above.
        tokio::time::sleep(Duration::from_millis(100)).await;
        if renew {
            Mock::given(method("GET"))
                .and(path("/oauth2/validate"))
                .respond_with(
                    ResponseTemplate::new(401)
                        .set_body_json(json!({"status":401,"message":"invalid token"})),
                )
                .with_priority(1)
                .mount(&server)
                .await;
            advance(3601).await;
            wait_calls(&server, "/oauth2/validate", 2).await;
        } else {
            app.shutdown.cancel();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let exited_early = worker.is_finished();
        let renewed_early = calls(&server, "/oauth2/validate").await > 2;
        drop(history);
        stop(&app, worker).await;
        assert!(!exited_early, "shutdown detached a durable write");
        assert!(
            !renewed_early,
            "token renewal started before the prior write finished"
        );
        assert_eq!(History::load(dir.path()).total(), 1);
    }
}
