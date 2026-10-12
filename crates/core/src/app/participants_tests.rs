use super::*;
use crate::{
    dto::{AllowedChannel, CampaignView},
    twitch::tests::{gql_mock, http, session},
};
use serde_json::{Value, json};
use std::sync::Arc;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

fn query(offset: usize) -> Query {
    Query {
        campaign_id: "campaign".into(),
        offset,
    }
}

async fn setup() -> (tempfile::TempDir, Arc<Application>) {
    let dir = tempfile::tempdir().unwrap();
    let (app, _commands) = Application::open(dir.path().into()).unwrap();
    {
        let mut snapshot = app.snapshot.write().await;
        snapshot.login.user_id = Some(42);
        snapshot.campaigns.push(CampaignView {
            id: "campaign".into(),
            game_id: 1,
            game_name: "Rust".into(),
            active: true,
            ends_at: Utc::now() + chrono::Duration::hours(1),
            allowed_channels: (1..=11)
                .map(|id| AllowedChannel {
                    id,
                    login: format!("streamer{id}"),
                    name: format!("Streamer {id}"),
                })
                .collect(),
            ..Default::default()
        });
    }
    (dir, app)
}

fn response(q: &Value) -> Value {
    if q["operationName"] == "DropsHighlightService_AvailableDrops" {
        return match q["variables"]["channelID"].as_str().unwrap() {
            "6" => json!({"data":{"channel":{}}}),
            "7" => json!({"data":{"channel":{"viewerDropCampaigns":[]}}}),
            _ => json!({"data":{"channel":{"viewerDropCampaigns":[{"id":"campaign"}]}}}),
        };
    }
    assert_eq!(q["operationName"], "VideoPlayerStreamInfoOverlayChannel");
    let id = q["variables"]["channel"]
        .as_str()
        .unwrap()
        .strip_prefix("streamer")
        .unwrap();
    let mut user = json!({"id":id, "displayName":format!("Streamer {id}"), "stream":{"id":"broadcast","viewersCount":null},
        "broadcastSettings":{"game":{"id":"1", "name":"Rust"}}});
    match id {
        "2" => user["stream"] = Value::Null,
        "3" => {
            user.as_object_mut().unwrap().remove("stream");
        }
        "4" => user["broadcastSettings"]["game"] = json!({"id":"2","name":"Other"}),
        "5" => user["broadcastSettings"]["game"] = Value::Null,
        "8" => user["id"] = json!(999),
        "9" => user["stream"] = json!({"id":""}),
        _ => {}
    }
    json!({"data":{"user":user}})
}

#[tokio::test]
async fn participants_are_bounded_cached_and_do_not_infer_offline_or_eligibility() {
    let server = MockServer::start().await;
    gql_mock(&server, response).await;
    let (_dir, app) = setup().await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    let cancel = client.http.cancel.clone();
    let (sender, receiver) = mpsc::channel(4);
    *app.participant_queries.write().await = Some(sender);
    let worker = tokio::spawn(run(client, receiver));
    let page = app.campaign_channels(query(0)).await.unwrap();
    assert_eq!(page.total, 11);
    assert_eq!(page.channels.len(), 10);
    for (index, online, eligible) in [
        (0, Some(true), Some(true)),
        (1, Some(false), Some(false)),
        (2, None, None),
        (3, Some(true), Some(false)),
        (4, Some(true), None),
        (5, Some(true), None),
        (6, Some(true), Some(false)),
        (7, None, None),
        (8, None, None),
    ] {
        assert_eq!(
            (page.channels[index].online, page.channels[index].eligible),
            (online, eligible),
            "{index}"
        );
    }
    // Current mining candidates are empty: detail status is scoped to the real ACL instead.
    assert!(app.snapshot.read().await.channels.is_empty());
    app.snapshot.write().await.campaigns[0].game_id = 509663;
    let special = app.campaign_channels(query(0)).await.unwrap();
    assert_eq!(special.channels[4].eligible, Some(true));
    assert_eq!(special.channels[5].eligible, Some(true));
    assert_eq!(special.channels[6].eligible, Some(true));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .body_json::<Value>()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        20
    );
    let last = app.campaign_channels(query(10)).await.unwrap();
    assert_eq!(last.channels.len(), 1);
    assert_eq!(last.channels[0].login, "streamer11");
    cancel.cancel();
    assert_eq!(worker.await.unwrap(), Err(TwitchError::Cancelled));
}

#[tokio::test]
async fn queries_reject_non_acl_targets_and_stale_account_or_campaign_replies() {
    let (_dir, app) = setup().await;
    for invalid in [
        query(1),
        query(20),
        Query {
            campaign_id: "x".repeat(257),
            offset: 0,
        },
    ] {
        assert!(matches!(
            app.campaign_channels(invalid).await,
            Err(AppError::InvalidRequest)
        ));
    }
    assert!(
        serde_json::from_value::<Query>(json!({"campaign_id":"campaign","channels":["outside"]}))
            .is_err()
    );
    for mutation in 0..3 {
        let (_dir, app) = setup().await;
        let (sender, mut receiver) = mpsc::channel(4);
        *app.participant_queries.write().await = Some(sender);
        let reply = async {
            let request = receiver.recv().await.unwrap();
            match mutation {
                0 => app.snapshot.write().await.login.user_id = None,
                1 => app.snapshot.write().await.campaigns[0]
                    .allowed_channels
                    .clear(),
                _ => app.snapshot.write().await.campaigns[0].game_id = 2,
            }
            request.complete.send(Ok(vec![])).unwrap();
        };
        let (result, _) = tokio::join!(app.campaign_channels(query(0)), reply);
        assert!(result.is_err());
    }
    app.snapshot.write().await.campaigns[0].finished = true;
    assert!(matches!(
        app.campaign_channels(query(0)).await,
        Err(AppError::InvalidRequest)
    ));
}

#[tokio::test]
async fn abandoned_reads_cancel_and_authentication_failure_reaches_the_session_owner() {
    let server = MockServer::start().await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    let cancel = client.http.cancel.clone();
    let (_dir, app) = setup().await;
    let scope = query(0).scope(&*app.snapshot.read().await).unwrap();
    let (sender, receiver) = mpsc::channel(4);
    let (complete, abandoned) = oneshot::channel();
    sender
        .send(Request {
            channels: scope.channels.clone(),
            complete,
        })
        .await
        .unwrap();
    drop(abandoned);
    let worker = tokio::spawn(run(client, receiver));
    Mock::given(path("/gql"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    let (complete, result) = oneshot::channel();
    sender
        .send(Request {
            channels: scope.channels,
            complete,
        })
        .await
        .unwrap();
    assert_eq!(
        result.await.unwrap().unwrap_err(),
        TwitchError::Unauthorized
    );
    assert_eq!(worker.await.unwrap(), Err(TwitchError::Unauthorized));
    cancel.cancel();
}

#[tokio::test]
async fn closing_a_pending_detail_drops_its_read_and_generation_shutdown_drains_the_worker() {
    let server = MockServer::start().await;
    Mock::given(path("/gql"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(20)))
        .mount(&server)
        .await;
    let client = TwitchClient::new(Arc::new(http(&server)), &session());
    let cancel = client.http.cancel.clone();
    let (_dir, app) = setup().await;
    let (sender, receiver) = mpsc::channel(4);
    *app.participant_queries.write().await = Some(sender);
    let worker = tokio::spawn(run(client, receiver));
    let caller = {
        let app = app.clone();
        tokio::spawn(async move { app.campaign_channels(query(0)).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    cancel.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(200), worker)
            .await
            .unwrap()
            .unwrap(),
        Err(TwitchError::Cancelled)
    );
}
