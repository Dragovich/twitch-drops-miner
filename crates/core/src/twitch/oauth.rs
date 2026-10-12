use std::{path::Path, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{sync::Notify, time::Instant};
use twitch_oauth2::{
    AccessToken, ClientId, DeviceUserTokenBuilder, RefreshToken, TwitchToken, UserToken,
    client::Client,
    tokens::errors::{
        DeviceUserTokenExchangeError, RefreshTokenError, RetrieveTokenError, ValidationError,
    },
};

use super::{CLIENT_ID, TwitchError, TwitchHttp, diagnostics};
use crate::{
    dto::OAuthCode,
    store::{atomic_json, read_json},
};

const SESSION_FILE: &str = "twitch_session.json";

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    version: u32,
    client_id: String,
    pub user_id: u64,
    pub device_id: String,
    pub(crate) access_token: String,
    refresh_token: Option<String>,
}

impl Session {
    pub fn load(directory: &Path) -> Result<Option<Self>, TwitchError> {
        let saved = match read_json::<Self>(&directory.join(SESSION_FILE)) {
            Ok(saved) => saved,
            Err(error) if error.downcast_ref::<serde_json::Error>().is_some() => {
                Self::preserve_invalid(directory)?;
                return Ok(None);
            }
            Err(_) => return Err(TwitchError::Storage),
        };
        if let Some(value) = &saved
            && (value.version != 1
                || value.client_id != CLIENT_ID
                || value.user_id == 0
                || [&value.access_token, &value.device_id].iter().any(|v| {
                    v.is_empty() || v.len() > 4096 || !v.bytes().all(|b| b.is_ascii_alphanumeric())
                }))
        {
            Self::preserve_invalid(directory)?;
            return Ok(None);
        }
        Ok(saved)
    }
    fn preserve_invalid(directory: &Path) -> Result<(), TwitchError> {
        let suffix = crate::random_hex::<16>().map_err(|_| TwitchError::Storage)?;
        std::fs::rename(
            directory.join(SESSION_FILE),
            directory.join(format!("twitch_session.invalid-{suffix}.json")),
        )
        .map_err(|_| TwitchError::Storage)?;
        tracing::warn!(
            "Unreadable Twitch session preserved as a separate file; fresh device authorization required"
        );
        Ok(())
    }
    pub fn save(&self, directory: &Path) -> Result<(), TwitchError> {
        atomic_json(&directory.join(SESSION_FILE), self).map_err(|_| TwitchError::Storage)
    }
    pub fn remove(directory: &Path) -> Result<(), TwitchError> {
        match std::fs::remove_file(directory.join(SESSION_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(TwitchError::Storage),
        }
    }
    pub fn from_token(token: &UserToken, http: &TwitchHttp) -> Result<Self, TwitchError> {
        if token.client_id().as_str() != CLIENT_ID {
            return Err(TwitchError::Unauthorized);
        }
        Ok(Self {
            version: 1,
            client_id: CLIENT_ID.into(),
            user_id: token.user_id.as_str().parse().map_err(|_| {
                diagnostics::invalid("OAuthValidation", "user ID must fit u64", None)
            })?,
            device_id: http.device_id.clone(),
            access_token: token.access_token.secret().to_owned(),
            refresh_token: token.refresh_token.as_ref().map(|v| v.secret().to_owned()),
        })
    }
    // None requests renewal; identity mismatches remain fatal.
    pub(crate) async fn validate(&self, http: &TwitchHttp) -> Result<Option<Self>, TwitchError> {
        let access = AccessToken::new(self.access_token.clone());
        let validated = match access.validate_token(http).await {
            Ok(validated) => UserToken::new(
                access,
                self.refresh_token.clone().map(RefreshToken::new),
                validated,
                None,
            )
            .map_err(|_| {
                diagnostics::invalid(
                    "OAuthValidation",
                    "validated token cannot construct user session",
                    None,
                )
            })?,
            // Interrupted validation error bodies preserve 401/403 as a request error.
            Err(
                ValidationError::NotAuthorized
                | ValidationError::Request(TwitchError::Unauthorized),
            ) => return Ok(None),
            Err(error) => return Err(validation_error(error)),
        };
        self.same_account(&validated, http).map(Some)
    }
    pub async fn restore(&self, http: &TwitchHttp) -> Result<Self, TwitchError> {
        if let Some(validated) = self.validate(http).await? {
            return Ok(validated);
        }
        let refresh = self
            .refresh_token
            .clone()
            .ok_or(TwitchError::Unauthorized)?;
        let refreshed =
            UserToken::from_refresh_token(
                http,
                RefreshToken::new(refresh),
                ClientId::new(CLIENT_ID.into()),
                None,
            )
            .await
            .map_err(|error| match error {
                RetrieveTokenError::ValidationError { error, .. } => validation_error(error),
                RetrieveTokenError::RefreshTokenError {
                    error: RefreshTokenError::RequestError(error),
                    ..
                } => error,
                RetrieveTokenError::RefreshTokenError {
                    error:
                        RefreshTokenError::RequestParseError(
                            twitch_oauth2::RequestParseError::TwitchError(error),
                        ),
                    ..
                } if matches!(error.status.as_u16(), 400 | 401 | 403) => TwitchError::Unauthorized,
                _ => diagnostics::invalid(
                    "OAuthRefresh",
                    "token refresh response rejected by OAuth library",
                    None,
                ),
            })?;
        self.same_account(&refreshed, http)
    }
    fn same_account(&self, token: &UserToken, http: &TwitchHttp) -> Result<Self, TwitchError> {
        let restored = Self::from_token(token, http)?;
        if restored.user_id != self.user_id {
            return Err(TwitchError::Unauthorized);
        }
        Ok(restored)
    }
}

fn validation_error(error: ValidationError<TwitchError>) -> TwitchError {
    match error {
        ValidationError::NotAuthorized => TwitchError::Unauthorized,
        ValidationError::Request(error) => error,
        _ => diagnostics::invalid(
            "OAuthValidation",
            "validation response rejected by OAuth library",
            None,
        ),
    }
}

pub struct DeviceLogin {
    builder: DeviceUserTokenBuilder,
    pub code: OAuthCode,
    expires_at: Instant,
    interval: Duration,
    http: Arc<TwitchHttp>,
}

impl DeviceLogin {
    pub async fn start(http: Arc<TwitchHttp>) -> Result<Self, TwitchError> {
        let mut builder = DeviceUserTokenBuilder::new(CLIENT_ID, vec![]);
        let response = http.req(builder.get_exchange_device_code_request()).await?;
        if !response.status().is_success() {
            return Err(TwitchError::Status(response.status().as_u16()));
        }
        let code = builder
            .parse_exchange_device_code_response(response)
            .map_err(|_| {
                diagnostics::invalid(
                    "OAuthDeviceCode",
                    "device response rejected by OAuth library",
                    None,
                )
            })?;
        let url = url::Url::parse(&code.verification_uri).map_err(|_| {
            diagnostics::invalid("OAuthDeviceCode", "verification URI is malformed", None)
        })?;
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("www.twitch.tv" | "twitch.tv"))
            || !url.username().is_empty()
            || url.password().is_some()
            || code.expires_in == 0
            || code.expires_in > 86400
            || code.user_code.is_empty()
            || code.interval > 3600
        {
            return Err(diagnostics::invalid(
                "OAuthDeviceCode",
                "verification URI, expiry, user code or polling interval failed validation",
                None,
            ));
        }
        let expires_at = Instant::now() + Duration::from_secs(code.expires_in);
        let interval = Duration::from_secs(code.interval.max(1));
        let code = OAuthCode {
            url: url.into(),
            code: code.user_code.clone(),
        };
        Ok(Self {
            builder,
            code,
            expires_at,
            interval,
            http,
        })
    }

    pub async fn finish(mut self, confirmed: &Notify) -> Result<Session, TwitchError> {
        tokio::select! {biased;
            _=self.http.cancel.cancelled()=>return Err(TwitchError::Cancelled),
            _=tokio::time::sleep_until(self.expires_at)=>return Err(TwitchError::Expired),
            _=confirmed.notified()=>{},
        }
        loop {
            tokio::select! {biased;
                _=self.http.cancel.cancelled()=>return Err(TwitchError::Cancelled),
                _=tokio::time::sleep_until(self.expires_at)=>return Err(TwitchError::Expired),
                _=tokio::time::sleep(self.interval)=>{},
            }
            let result = tokio::select! {biased;
                _=self.http.cancel.cancelled()=>return Err(TwitchError::Cancelled),
                _=tokio::time::sleep_until(self.expires_at)=>return Err(TwitchError::Expired),
                result=self.builder.try_finish(&*self.http)=>result,
            };
            match result {
                Ok(token) => return Session::from_token(&token, &self.http),
                Err(error) if error.is_pending() => {}
                Err(DeviceUserTokenExchangeError::TokenParseError(
                    twitch_oauth2::RequestParseError::TwitchError(error),
                )) => match error.message.as_str() {
                    "slow_down" => self.interval += Duration::from_secs(5),
                    "expired_token" | "invalid_device_code" => return Err(TwitchError::Expired),
                    "access_denied" => return Err(TwitchError::Denied),
                    _ => return Err(TwitchError::Status(error.status.as_u16())),
                },
                Err(DeviceUserTokenExchangeError::TokenRequestError(error)) => return Err(error),
                Err(DeviceUserTokenExchangeError::ValidationError(error)) => {
                    return Err(validation_error(error));
                }
                Err(_) => {
                    return Err(diagnostics::invalid(
                        "OAuthDeviceCode",
                        "token exchange response rejected by OAuth library",
                        None,
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::twitch::tests::{http, session, validation};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    async fn device(server: &MockServer, expires: u64) {
        Mock::given(method("POST")).and(path("/oauth2/device")).and(body_string_contains("scopes="))
            .and(body_string_contains(format!("client_id={CLIENT_ID}")))
            .respond_with(ResponseTemplate::new(200).insert_header("Content-Type","application/json; charset=utf-8")
                .set_body_raw(serde_json::to_vec(&json!({"device_code":"privatecode","user_code":"ABCD1234","verification_uri":"https://www.twitch.tv/activate?device-code=ABCD1234","interval":1,"expires_in":expires})).unwrap(),"application/json; charset=utf-8"))
            .mount(server).await;
    }

    async fn valid_token(server: &MockServer, token: &str) {
        Mock::given(method("GET"))
            .and(path("/oauth2/validate"))
            .and(header("Authorization", format!("OAuth {token}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(validation()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn device_code_confirmation_pending_and_restart_use_safe_library_requests() {
        let server = MockServer::start().await;
        device(&server, 60).await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"status":400,"message":"authorization_pending"})),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/oauth2/token")).and(body_string_contains("device_code=privatecode"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"testtoken","refresh_token":"testrefresh","token_type":"bearer"})))
            .mount(&server).await;
        valid_token(&server, "testtoken").await;
        let http = Arc::new(http(&server));
        let login = DeviceLogin::start(http.clone()).await.unwrap();
        assert_eq!(login.code.code, "ABCD1234");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let confirmation = Notify::new();
        confirmation.notify_one();
        let session = login.finish(&confirmation).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("cookies.jar"),
            b"existing credential backup",
        )
        .unwrap();
        session.save(directory.path()).unwrap();
        let restored = Session::load(directory.path())
            .unwrap()
            .unwrap()
            .restore(&http)
            .await
            .unwrap();
        assert_eq!(restored.user_id, 42);
        assert_eq!(restored.device_id, "testdevice");
        for request in server.received_requests().await.unwrap() {
            assert!(request.url.query().is_none_or(str::is_empty));
            assert!(!request.url.as_str().contains("privatecode"));
            assert_eq!(request.headers.get("client-id").unwrap(), CLIENT_ID);
        }
        Session::remove(directory.path()).unwrap();
        assert!(Session::load(directory.path()).unwrap().is_none());
        assert_eq!(
            std::fs::read(directory.path().join("cookies.jar")).unwrap(),
            b"existing credential backup"
        );
    }

    #[tokio::test]
    async fn expired_access_tokens_refresh_without_requiring_a_client_secret() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/validate"))
            .and(header("Authorization", "OAuth testtoken"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(json!({"status":401,"message":"invalid token"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=testrefresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token":"newtoken","refresh_token":"newrefresh","expires_in":3600}),
            ))
            .mount(&server)
            .await;
        valid_token(&server, "newtoken").await;
        let restored = session().restore(&http(&server)).await.unwrap();
        assert_eq!(restored.access_token, "newtoken");
        assert_eq!(restored.refresh_token.as_deref(), Some("newrefresh"));
    }

    #[tokio::test]
    async fn device_flow_expires_without_confirmation_and_cancellation_is_immediate() {
        let server = MockServer::start().await;
        device(&server, 1).await;
        let http = Arc::new(http(&server));
        let login = DeviceLogin::start(http.clone()).await.unwrap();
        assert!(matches!(
            login.finish(&Notify::new()).await,
            Err(TwitchError::Expired)
        ));
        let login = DeviceLogin::start(http.clone()).await.unwrap();
        http.cancel.cancel();
        assert!(matches!(
            login.finish(&Notify::new()).await,
            Err(TwitchError::Cancelled)
        ));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.url.path() == "/oauth2/device")
        );
    }

    #[tokio::test]
    async fn oauth_rejects_wrong_clients_and_reports_status_without_upstream_secrets() {
        let server = MockServer::start().await;
        let mut validated = validation();
        validated["client_id"] = "wrongclient".into();
        Mock::given(method("GET"))
            .and(path("/oauth2/validate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(validated))
            .mount(&server)
            .await;
        let http = Arc::new(http(&server));
        assert!(matches!(
            session().restore(&http).await,
            Err(TwitchError::Unauthorized)
        ));
        Mock::given(method("POST"))
            .and(path("/oauth2/device"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({"secret":"do-not-echo"})))
            .mount(&server)
            .await;
        assert!(matches!(
            DeviceLogin::start(http).await,
            Err(TwitchError::Status(400))
        ));
    }

    #[tokio::test]
    async fn slow_down_increases_poll_interval_before_retry() {
        let server = MockServer::start().await;
        device(&server, 60).await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"status":400,"message":"slow_down"})),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"status":400,"message":"access_denied"})),
            )
            .mount(&server)
            .await;
        let login = DeviceLogin::start(Arc::new(http(&server))).await.unwrap();
        let confirmation = Notify::new();
        confirmation.notify_one();
        let started = Instant::now();
        assert!(matches!(
            login.finish(&confirmation).await,
            Err(TwitchError::Denied)
        ));
        assert!(started.elapsed() >= Duration::from_secs(7));
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == "/oauth2/token")
                .count(),
            2
        );
    }
}
