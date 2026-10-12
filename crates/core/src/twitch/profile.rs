use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use super::{TwitchClient, TwitchError};
use crate::dto::{AccountBadge, AccountProfile};

const PROFILE: &str = r#"query AccountProfile($id: ID!) {
  user(id: $id) {
    id login displayName profileImageURL(width: 300) chatColor
    displayBadges { id title description imageURL(size: QUADRUPLE) }
  }
}"#;
const BADGES: &str = r#"query AccountBadges {
  currentUser { id availableBadges { id title description imageURL(size: QUADRUPLE) } }
}"#;

impl TwitchClient {
    // Optional, session-owned metadata. Separate queries keep a denied badge
    // collection from hiding the public identity. Neither request blocks watching.
    pub async fn account_profile(&self) -> Result<Option<AccountProfile>, TwitchError> {
        let (profile, badges) = tokio::join!(
            self.profile_query(
                "AccountProfile",
                PROFILE,
                json!({"id":self.user_id.to_string()})
            ),
            self.profile_query("AccountBadges", BADGES, json!({})),
        );
        let profile = profile?;
        let badges = badges?;
        let Some(mut profile) = profile
            .as_ref()
            .and_then(|value| AccountProfile::parse(&value["data"]["user"], self.user_id))
        else {
            return Ok(None);
        };
        if let Some(user) = badges.as_ref().map(|value| &value["data"]["currentUser"])
            && matches_user(user, self.user_id)
        {
            profile.available_badges = parse_badges(&user["availableBadges"]);
        }
        Ok(Some(profile))
    }

    async fn profile_query(
        &self,
        name: &str,
        query: &str,
        variables: Value,
    ) -> Result<Option<Value>, TwitchError> {
        match tokio::time::timeout(
            Duration::from_secs(10),
            self.gql(json!({"operationName":name, "query":query, "variables":variables})),
        )
        .await
        {
            Ok(Ok(value)) => Ok(Some(value)),
            Ok(Err(error @ (TwitchError::Unauthorized | TwitchError::Cancelled))) => Err(error),
            // Missing fields, optional permissions and network failures leave
            // profile data unavailable. Never renew or discard a login for these.
            _ => Ok(None),
        }
    }
}

impl AccountProfile {
    fn parse(user: &Value, user_id: u64) -> Option<Self> {
        if !matches_user(user, user_id) {
            return None;
        }
        let login = text(&user["login"], 25)?;
        if !login
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return None;
        }
        Some(Self {
            display_name: text(&user["displayName"], 100).unwrap_or_else(|| login.clone()),
            login,
            avatar_url: artwork(&user["profileImageURL"]),
            color: text(&user["chatColor"], 7).filter(|color| {
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..].bytes().all(|c| c.is_ascii_hexdigit())
            }),
            badges: parse_badges(&user["displayBadges"]).unwrap_or_default(),
            available_badges: None,
        })
    }
}

fn matches_user(user: &Value, id: u64) -> bool {
    user["id"].as_str().and_then(|id| id.parse::<u64>().ok()) == Some(id)
}

fn text(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= limit)
        .map(str::to_owned)
}

pub(super) fn artwork(value: &Value) -> Option<String> {
    let url = Url::parse(&text(value, 2048)?).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str() == Some("static-cdn.jtvnw.net"))
    .then(|| url.to_string())
}

fn parse_badges(value: &Value) -> Option<Vec<AccountBadge>> {
    let list = value.as_array().filter(|list| list.len() <= 256)?;
    let mut result = Vec::<AccountBadge>::new();
    for badge in list {
        let id = text(&badge["id"], 256)?;
        let title = text(&badge["title"], 200)?;
        if !result.iter().any(|badge| badge.id == id) {
            result.push(AccountBadge {
                id,
                title,
                description: text(&badge["description"], 2000),
                image_url: artwork(&badge["imageURL"]),
            });
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::tests::{gql_mock, http, session};
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn user() -> Value {
        json!({"id":"42", "login":"miner", "displayName":"Miner", "chatColor":"#008000",
            "profileImageURL":"https://static-cdn.jtvnw.net/avatar.png",
            "displayBadges":[{"id":"event", "title":"Event badge", "description":"Earned during an event", "imageURL":"https://static-cdn.jtvnw.net/badge.png"}]})
    }

    #[tokio::test]
    async fn profile_loads_identity_and_own_badge_collection_without_extra_scopes() {
        let server = MockServer::start().await;
        gql_mock(&server, |request| match request["operationName"].as_str().unwrap() {
            "AccountProfile" => {
                assert_eq!(request["variables"]["id"], "42");
                json!({"data":{"user":user()}})
            },
            "AccountBadges" => json!({"data":{"currentUser":{"id":"42", "availableBadges":user()["displayBadges"]}}}),
            _ => unreachable!(),
        }).await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        let profile = client.account_profile().await.unwrap().unwrap();
        assert_eq!(profile.login, "miner");
        assert_eq!(profile.display_name, "Miner");
        assert_eq!(profile.color.as_deref(), Some("#008000"));
        assert_eq!(profile.available_badges, Some(profile.badges));
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn profile_keeps_equipped_badges_when_collection_is_denied_missing_or_wrong_account() {
        for response in [
            json!({"errors":[{"message":"unauthenticated"}], "data":{"currentUser":null}}),
            json!({"data":{"currentUser":{"id":"42","availableBadges":null}}}),
            json!({"data":{"currentUser":{"id":"99","availableBadges":[]}}}),
        ] {
            let server = MockServer::start().await;
            gql_mock(&server, move |request| {
                if request["operationName"] == "AccountProfile" {
                    json!({"data":{"user":user()}})
                } else {
                    response.clone()
                }
            })
            .await;
            let client = TwitchClient::new(Arc::new(http(&server)), &session());
            let profile = client.account_profile().await.unwrap().unwrap();
            assert_eq!(profile.badges.len(), 1);
            assert!(profile.available_badges.is_none());
        }
    }

    #[tokio::test]
    async fn profile_is_optional_but_retains_authentication_and_cancellation_failures() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        let client = TwitchClient::new(Arc::new(http(&server)), &session());
        assert_eq!(client.account_profile().await, Ok(None));
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert_eq!(
            client.account_profile().await,
            Err(TwitchError::Unauthorized)
        );
        client.http.cancel.cancel();
        assert_eq!(client.account_profile().await, Err(TwitchError::Cancelled));
    }

    #[test]
    fn profile_validates_identity_artwork_color_and_collection_completeness() {
        let mut value = user();
        value["displayName"] = json!("表示名");
        let profile = AccountProfile::parse(&value, 42).unwrap();
        assert_eq!(profile.login, "miner");
        assert_eq!(profile.display_name, "表示名");
        let mut legacy = serde_json::to_value(&profile).unwrap();
        legacy.as_object_mut().unwrap().remove("login");
        assert!(
            serde_json::from_value::<AccountProfile>(legacy)
                .unwrap()
                .login
                .is_empty()
        );
        assert!(AccountProfile::parse(&value, 99).is_none());
        value["login"] = json!("../bad");
        assert!(AccountProfile::parse(&value, 42).is_none());
        value["login"] = json!("miner");
        value["chatColor"] = json!("red;background:url(example)");
        value["profileImageURL"] = json!("https://static-cdn.jtvnw.net.evil.test/image.png");
        let profile = AccountProfile::parse(&value, 42).unwrap();
        assert!(profile.color.is_none() && profile.avatar_url.is_none());
        for url in [
            "javascript:alert(1)",
            "http://static-cdn.jtvnw.net/image.png",
            "https://user:secret@static-cdn.jtvnw.net/image.png",
        ] {
            value["profileImageURL"] = json!(url);
            assert!(
                AccountProfile::parse(&value, 42)
                    .unwrap()
                    .avatar_url
                    .is_none()
            );
        }
        let badge = user()["displayBadges"][0].clone();
        assert_eq!(
            parse_badges(&json!([badge.clone(), badge.clone()]))
                .unwrap()
                .len(),
            1
        );
        assert!(parse_badges(&json!([badge, null])).is_none());
        assert_eq!(parse_badges(&json!([])), Some(vec![]));
        assert!(parse_badges(&json!(vec![json!({}); 257])).is_none());
    }
}
