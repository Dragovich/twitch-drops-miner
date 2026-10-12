use std::{collections::HashSet, time::Duration};

use http::{HeaderValue, Method, StatusCode, header};
use serde::Deserialize;
use serde_json::Value;

use super::{RetryPolicy, TwitchClient, TwitchError, diagnostics, success};
use crate::{
    app::commands::{ChannelQuery, GameQuery},
    config::GameMetadata,
    dto::ChannelSearchResult,
};

impl TwitchClient {
    #[tracing::instrument(skip_all, fields(operation = "GameDirectory"))]
    pub async fn games(&self, query: &GameQuery) -> Result<Vec<GameMetadata>, TwitchError> {
        if !query.valid() {
            return Err(TwitchError::InvalidResponse);
        }
        let (path, parameters) = match query {
            GameQuery::Search(query) => (
                "search/categories",
                vec![("query", query.trim()), ("first", "20")],
            ),
            GameQuery::Names(names) => (
                "games",
                names.iter().map(|name| ("name", name.as_str())).collect(),
            ),
        };
        let value = self.helix_read(path, &parameters).await?;
        #[derive(Deserialize)]
        struct Games {
            data: Vec<GameMetadata>,
        }
        let games: Games = serde_json::from_value(value).map_err(|_| {
            diagnostics::invalid(
                "GameDirectory",
                "expected data array with string id/name/box_art_url fields",
                None,
            )
        })?;
        if games.data.len() > 100 || games.data.iter().any(|game| !game.valid()) {
            return Err(diagnostics::invalid(
                "GameDirectory",
                "invalid or oversized game metadata",
                None,
            ));
        }
        let mut seen = HashSet::new();
        Ok(games
            .data
            .into_iter()
            .filter(|game| seen.insert(game.id.clone()))
            .collect())
    }

    #[tracing::instrument(skip_all, fields(operation = "ChannelSearch"))]
    pub async fn search_channels(
        &self,
        query: &ChannelQuery,
    ) -> Result<Vec<ChannelSearchResult>, TwitchError> {
        if !query.valid() {
            return Err(TwitchError::InvalidResponse);
        }
        let value = self
            .helix_read(
                "search/channels",
                &[
                    ("query", query.search.trim()),
                    ("first", "20"),
                    ("live_only", "false"),
                ],
            )
            .await?;
        let invalid = || {
            diagnostics::invalid(
                "ChannelSearch",
                "invalid or oversized channel metadata",
                None,
            )
        };
        let list = value
            .get("data")
            .and_then(Value::as_array)
            .filter(|list| list.len() <= 20)
            .ok_or_else(invalid)?;
        let mut seen = HashSet::new();
        let mut channels = Vec::new();
        for raw in list {
            let id = raw["id"]
                .as_str()
                .and_then(|id| id.parse::<u64>().ok())
                .filter(|id| *id > 0)
                .ok_or_else(invalid)?;
            let login = raw["broadcaster_login"]
                .as_str()
                .filter(|login| {
                    (1..=25).contains(&login.len())
                        && login
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                })
                .ok_or_else(invalid)?
                .to_ascii_lowercase();
            let name = raw["display_name"]
                .as_str()
                .filter(|name| {
                    !name.trim().is_empty()
                        && name.len() <= 256
                        && !name.chars().any(char::is_control)
                })
                .ok_or_else(invalid)?
                .to_owned();
            let game = raw["game_name"]
                .as_str()
                .filter(|game| game.len() <= 1024 && !game.chars().any(char::is_control))
                .ok_or_else(invalid)?;
            let online = raw["is_live"].as_bool().ok_or_else(invalid)?;
            if seen.insert(id) {
                channels.push(ChannelSearchResult {
                    id,
                    login,
                    name,
                    online,
                    avatar_url: super::profile::artwork(&raw["thumbnail_url"]),
                    game: (!game.is_empty()).then(|| game.to_owned()),
                });
            }
        }
        Ok(channels)
    }

    async fn helix_read(
        &self,
        path: &str,
        parameters: &[(&str, &str)],
    ) -> Result<Value, TwitchError> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let _permit = tokio::select! { biased;
                _ = self.http.cancel.cancelled() => return Err(TwitchError::Cancelled),
                permit = self.http.concurrent.acquire() => permit.map_err(|_| TwitchError::Cancelled)?,
            };
            self.http.acquire().await?;
            let mut url = self.http.endpoints.helix.join(path).map_err(|_| TwitchError::Configuration)?;
            url.query_pairs_mut().extend_pairs(parameters.iter().copied());
            let mut authorization = HeaderValue::from_str(&format!("Bearer {}", self.token()))
                .map_err(|_| TwitchError::Configuration)?;
            authorization.set_sensitive(true);
            let response = self.http.execute(
                self.http.request(Method::GET, url).header(header::AUTHORIZATION, authorization),
                RetryPolicy::Replay,
            ).await?;
            if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(TwitchError::Unauthorized);
            }
            success(response.status())?;
            diagnostics::json(response.body(), response.status().as_u16())
        }).await.map_err(|_| TwitchError::Network)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::tests::{http, session};
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    #[tokio::test]
    async fn game_directory_uses_the_existing_token_and_returns_official_metadata() {
        let server = MockServer::start().await;
        let session = session();
        let client = TwitchClient::new(Arc::new(http(&server)), &session);
        let game = json!({"id":"33214", "name":"Fortnite", "box_art_url":"https://static-cdn.jtvnw.net/ttv-boxart/33214-52x72.jpg"});
        for (endpoint, field, value) in [
            ("/helix/search/categories", "query", "fort"),
            ("/helix/games", "name", "Fortnite"),
        ] {
            Mock::given(method("GET"))
                .and(path(endpoint))
                .and(query_param(field, value))
                .and(header("client-id", super::super::CLIENT_ID))
                .and(header(
                    "authorization",
                    format!("Bearer {}", session.access_token),
                ))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"data":[game.clone()]})),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        assert_eq!(
            client
                .games(&GameQuery::Search("fort".into()))
                .await
                .unwrap()[0]
                .name,
            "Fortnite"
        );
        assert_eq!(
            client
                .games(&GameQuery::Names(vec!["Fortnite".into()]))
                .await
                .unwrap()[0]
                .id,
            "33214"
        );
        assert!(client.games(&GameQuery::Search(" ".into())).await.is_err());
        assert!(!GameQuery::Names(vec!["name".into(); 101]).valid());
        client.http.cancel.cancel();
        assert_eq!(
            client
                .games(&GameQuery::Search("fort".into()))
                .await
                .unwrap_err(),
            TwitchError::Cancelled
        );
    }

    #[tokio::test]
    async fn game_directory_rejects_bad_metadata_and_preserves_auth_errors() {
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        for body in [
            json!({}),
            json!({"data":null}),
            json!({"data":[{"id":"1", "name":"Game", "box_art_url":"javascript:alert(1)"}]}),
        ] {
            server.reset().await;
            Mock::given(path("/helix/search/categories"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                client
                    .games(&GameQuery::Search("game".into()))
                    .await
                    .unwrap_err(),
                TwitchError::InvalidResponse
            );
        }
        for status in [401, 403] {
            server.reset().await;
            Mock::given(path("/helix/search/categories"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            assert_eq!(
                client
                    .games(&GameQuery::Search("game".into()))
                    .await
                    .unwrap_err(),
                TwitchError::Unauthorized
            );
        }
    }
    fn search_query() -> ChannelQuery {
        ChannelQuery {
            search: " streamer ".into(),
        }
    }

    fn search_channel() -> Value {
        json!({"id":"123", "broadcaster_login":"streamer", "display_name":"Streamer",
            "is_live":false, "game_name":"Rust",
            "thumbnail_url":"https://static-cdn.jtvnw.net/channel.png"})
    }

    #[tokio::test]
    async fn channel_search_includes_offline_identities_and_validated_avatars_without_extra_requests()
     {
        let server = MockServer::start().await;
        let session = session();
        let client = TwitchClient::new(Arc::new(http(&server)), &session);
        let mut live = search_channel();
        live["id"] = "456".into();
        live["broadcaster_login"] = "live_streamer".into();
        live["is_live"] = true.into();
        live["thumbnail_url"] = "https://untrusted.example/avatar.png".into();
        Mock::given(method("GET"))
            .and(path("/helix/search/channels"))
            .and(query_param("query", "streamer"))
            .and(query_param("first", "20"))
            .and(query_param("live_only", "false"))
            .and(header("client-id", super::super::CLIENT_ID))
            .and(header(
                "authorization",
                format!("Bearer {}", session.access_token),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[search_channel(), search_channel(), live]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let channels = client.search_channels(&search_query()).await.unwrap();
        assert_eq!(channels.len(), 2);
        assert!(!channels[0].online);
        assert_eq!(
            channels[0].avatar_url.as_deref(),
            Some("https://static-cdn.jtvnw.net/channel.png")
        );
        assert_eq!(channels[1].login, "live_streamer");
        assert!(channels[1].online);
        assert!(channels[1].avatar_url.is_none());
        for query in ["".to_owned(), "x".repeat(101), "streamer\n".into()] {
            assert_eq!(
                client
                    .search_channels(&ChannelQuery { search: query })
                    .await
                    .unwrap_err(),
                TwitchError::InvalidResponse
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        client.http.cancel.cancel();
        assert_eq!(
            client.search_channels(&search_query()).await.unwrap_err(),
            TwitchError::Cancelled
        );
    }

    #[tokio::test]
    async fn channel_search_rejects_malformed_status_and_oversized_results_and_propagates_auth() {
        let server = MockServer::start().await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let mut malformed = vec![
            json!({}),
            json!({"data":null}),
            json!({"data":vec![search_channel(); 21]}),
        ];
        for (key, value) in [
            ("id", json!("0")),
            ("broadcaster_login", json!("../other")),
            ("is_live", json!(null)),
            ("display_name", json!("bad\nname")),
            ("game_name", json!(42)),
        ] {
            let mut channel = search_channel();
            channel[key] = value;
            malformed.push(json!({"data":[channel]}));
        }
        for body in malformed {
            server.reset().await;
            Mock::given(path("/helix/search/channels"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            assert_eq!(
                client.search_channels(&search_query()).await.unwrap_err(),
                TwitchError::InvalidResponse
            );
        }
        for status in [401, 403] {
            server.reset().await;
            Mock::given(path("/helix/search/channels"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            assert_eq!(
                client.search_channels(&search_query()).await.unwrap_err(),
                TwitchError::Unauthorized
            );
        }
    }
}
