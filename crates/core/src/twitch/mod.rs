mod catalog;
pub mod channels;
mod diagnostics;
mod directory;
pub mod inventory;
pub mod oauth;
pub mod operations;
pub(crate) mod playback;
mod profile;
pub mod pubsub;
#[cfg(test)]
pub(crate) mod tests;

use std::{collections::VecDeque, sync::Arc, time::Duration};

use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use reqwest::cookie::{CookieStore, Jar};
use serde_json::Value;
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{config::Settings, random_hex};

pub const CLIENT_ID: &str = "ue6666qo983tsx6so1t0vnawi233wa";
pub const CLIENT_ORIGIN: &str = "https://android.tv.twitch.tv";
pub const USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 7.1; Smart Box C1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryPolicy {
    Never,
    // Keep the existing send/429/5xx retries for operations with side effects.
    Transport,
    // Also retry an interrupted successful response for reads that are safe to replay.
    Replay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TwitchError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("Twitch login is required")]
    Unauthorized,
    #[error("Twitch returned HTTP {0}")]
    Status(u16),
    #[error("Twitch connection failed")]
    Network,
    #[error("Twitch returned an invalid response")]
    InvalidResponse,
    #[error("Twitch GraphQL request failed")]
    GraphQl,
    #[error("device authorization expired")]
    Expired,
    #[error("device authorization was denied")]
    Denied,
    #[error("saved Twitch session could not be read or written")]
    Storage,
    #[error("invalid connection configuration")]
    Configuration,
}

#[derive(Clone)]
pub(crate) struct Endpoints {
    pub oauth: Url,
    pub gql: Url,
    pub tv: Url,
    pub web: Url,
    pub pubsub: Url,
    pub catalog: Url,
    pub helix: Url,
    pub usher: Url,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            oauth: Url::parse("https://id.twitch.tv/").unwrap(),
            gql: Url::parse("https://gql.twitch.tv/gql").unwrap(),
            tv: Url::parse(CLIENT_ORIGIN).unwrap(),
            web: Url::parse("https://www.twitch.tv/").unwrap(),
            pubsub: Url::parse("wss://pubsub-edge.twitch.tv/v1").unwrap(),
            catalog: Url::parse("https://twitch-drops-api.sunkwi.com/v2/drops").unwrap(),
            helix: Url::parse("https://api.twitch.tv/helix/").unwrap(),
            usher: Url::parse("https://usher.ttvnw.net/").unwrap(),
        }
    }
}

#[cfg(test)]
impl Endpoints {
    pub(crate) fn mock(base: &str) -> Self {
        let base = Url::parse(&format!("{base}/")).unwrap();
        let mut pubsub = base.join("pubsub").unwrap();
        pubsub.set_scheme("ws").unwrap();
        Self {
            oauth: base.clone(),
            gql: base.join("gql").unwrap(),
            tv: base.join("tv").unwrap(),
            catalog: base.join("catalog").unwrap(),
            helix: base.join("helix/").unwrap(),
            usher: base.clone(),
            web: base,
            pubsub,
        }
    }
}

#[derive(Clone)]
pub struct TwitchHttp {
    pub(crate) client: reqwest::Client,
    catalog_client: reqwest::Client,
    pub(crate) endpoints: Endpoints,
    jar: Arc<Jar>,
    pub device_id: String,
    pub cancel: CancellationToken,
    rate: Arc<Mutex<VecDeque<Instant>>>,
    concurrent: Arc<Semaphore>,
    diagnostics: diagnostics::Capture,
}

impl TwitchHttp {
    pub fn new(
        settings: &Settings,
        device_id: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<Self, TwitchError> {
        Self::build(settings, device_id, cancel, Endpoints::default())
    }

    pub(crate) fn build(
        settings: &Settings,
        device_id: Option<&str>,
        cancel: CancellationToken,
        endpoints: Endpoints,
    ) -> Result<Self, TwitchError> {
        crate::config::validate_proxy(&settings.proxy).map_err(|_| TwitchError::Configuration)?;
        let device_id = device_id
            .map(str::to_owned)
            .unwrap_or(random_hex::<16>().map_err(|_| TwitchError::Configuration)?);
        let session_id = random_hex::<16>().map_err(|_| TwitchError::Configuration)?;
        let jar = Arc::new(Jar::default());
        jar.add_cookie_str(&format!("unique_id={device_id}; Path=/"), &endpoints.tv);
        let headers = HeaderMap::from_iter([
            (header::USER_AGENT, HeaderValue::from_static(USER_AGENT)),
            (header::ACCEPT_LANGUAGE, HeaderValue::from_static("en-US")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            (header::PRAGMA, HeaderValue::from_static("no-cache")),
            (
                http::HeaderName::from_static("client-id"),
                HeaderValue::from_static(CLIENT_ID),
            ),
            (
                http::HeaderName::from_static("client-session-id"),
                HeaderValue::from_str(&session_id).map_err(|_| TwitchError::Configuration)?,
            ),
        ]);
        let quality = u64::from(settings.connection_quality.clamp(1, 6));
        let builder = || {
            let mut builder = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .pool_max_idle_per_host(50)
                .pool_idle_timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(5 * quality))
                .timeout(Duration::from_secs(10 * quality));
            if !settings.proxy.is_empty() {
                builder = builder.proxy(
                    reqwest::Proxy::all(&settings.proxy).map_err(|_| TwitchError::Configuration)?,
                );
            }
            Ok::<_, TwitchError>(builder)
        };
        let client = builder()?
            .cookie_provider(jar.clone())
            .default_headers(headers)
            .build()
            .map_err(|_| TwitchError::Configuration)?;
        // Public metadata must never share Twitch headers, identifiers or cookies.
        let catalog_client = builder()?
            .user_agent(concat!("TwitchDropsMiner/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| TwitchError::Configuration)?;
        let diagnostics = diagnostics::Capture::new(&settings.proxy, &device_id);
        Ok(Self {
            client,
            catalog_client,
            endpoints,
            jar,
            device_id,
            cancel,
            rate: Arc::new(Mutex::new(VecDeque::new())),
            concurrent: Arc::new(Semaphore::new(5)),
            diagnostics,
        })
    }

    pub async fn discover_device(&mut self) -> Result<(), TwitchError> {
        let response = self
            .execute(
                self.request(Method::GET, self.endpoints.tv.clone()),
                RetryPolicy::Replay,
            )
            .await?;
        success(response.status())?;
        if let Some(cookie) = self
            .jar
            .cookies(&self.endpoints.tv)
            .and_then(|v| v.to_str().ok().map(str::to_owned))
        {
            for cookie in cookie::Cookie::split_parse(cookie).flatten() {
                if cookie.name() == "unique_id"
                    && !cookie.value().is_empty()
                    && cookie.value().len() <= 128
                    && cookie.value().bytes().all(|b| b.is_ascii_alphanumeric())
                {
                    self.device_id = cookie.value().to_owned();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("X-Device-Id", &self.device_id)
    }

    pub(crate) async fn sleep(&self, duration: Duration) -> Result<(), TwitchError> {
        tokio::select! { biased;
            _=self.cancel.cancelled()=>Err(TwitchError::Cancelled),
            _=tokio::time::sleep(duration)=>Ok(()),
        }
    }

    async fn acquire(&self) -> Result<(), TwitchError> {
        loop {
            let mut slots = self.rate.lock().await;
            let now = Instant::now();
            while slots
                .front()
                .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(1))
            {
                slots.pop_front();
            }
            if slots.len() < 5 {
                slots.push_back(now);
                return Ok(());
            }
            let wait = Duration::from_secs(1).saturating_sub(now.duration_since(slots[0]));
            drop(slots);
            self.sleep(wait).await?;
        }
    }

    pub(crate) async fn execute(
        &self,
        request: reqwest::RequestBuilder,
        retry: RetryPolicy,
    ) -> Result<http::Response<Vec<u8>>, TwitchError> {
        self.execute_with(&self.client, request, retry).await
    }

    #[tracing::instrument(skip_all, fields(endpoint, diagnostic_id, method))]
    async fn execute_with(
        &self,
        client: &reqwest::Client,
        request: reqwest::RequestBuilder,
        retry: RetryPolicy,
    ) -> Result<http::Response<Vec<u8>>, TwitchError> {
        let request = request.build().map_err(|_| TwitchError::Configuration)?;
        let endpoint = if request.url() == &self.endpoints.gql {
            "twitch_graphql"
        } else if request.url() == &self.endpoints.catalog {
            "public_catalog"
        } else if request
            .url()
            .as_str()
            .starts_with(self.endpoints.helix.as_str())
        {
            "twitch_directory"
        } else if request.url().origin() == self.endpoints.oauth.origin() {
            "twitch_oauth"
        } else if request.method() == Method::POST {
            "twitch_beacon"
        } else if request.url().origin() == self.endpoints.web.origin() {
            "twitch_page"
        } else {
            "twitch_settings_script"
        };
        tracing::Span::current().record("endpoint", endpoint);
        if diagnostics::enabled() {
            tracing::Span::current().record("diagnostic_id", diagnostics::request_id());
            tracing::Span::current().record("method", request.method().as_str());
        }
        for attempt in 0..5 {
            let started = Instant::now();
            // A retry may use cookies received by the previous attempt.
            let capture = self
                .diagnostics
                .request(&request, self.jar.cookies(request.url()));
            let sending = request.try_clone().ok_or(TwitchError::Configuration)?;
            let result = tokio::select! { biased;
                _=self.cancel.cancelled()=>return Err(TwitchError::Cancelled),
                result=client.execute(sending)=>result,
            };
            let response = match result {
                Ok(response) => response,
                Err(error) => {
                    capture.network(&error, "send", attempt + 1, started.elapsed().as_millis());
                    if retry != RetryPolicy::Never && attempt < 4 {
                        self.sleep(Duration::from_secs(1 << attempt)).await?;
                        continue;
                    }
                    return Err(TwitchError::Network);
                }
            };
            if !response.status().is_success() {
                tracing::warn!(
                    status = response.status().as_u16(),
                    attempt = attempt + 1,
                    "Upstream HTTP response is unsuccessful"
                );
            }
            if retry != RetryPolicy::Never
                && attempt < 4
                && (response.status().is_server_error()
                    || response.status() == StatusCode::TOO_MANY_REQUESTS)
            {
                let delay = response
                    .headers()
                    .get(header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok()?.parse::<u64>().ok())
                    .unwrap_or(1 << attempt)
                    .clamp(1, 60);
                if diagnostics::enabled() {
                    // Diagnostic body failures must not replace the existing retry decision.
                    if tokio::time::timeout(
                        Duration::from_secs(1),
                        self.read_response(response, attempt + 1, &capture, started),
                    )
                    .await
                    .is_err()
                    {
                        tracing::debug!(target: "tdm_diagnostics", attempt = attempt + 1, "Retry response capture exceeded one-second budget; body omitted");
                    }
                }
                self.sleep(Duration::from_secs(delay)).await?;
                continue;
            }
            let status = response.status();
            let result = self
                .read_response(response, attempt + 1, &capture, started)
                .await;
            match result {
                // A broken error body cannot erase the status already received.
                Err(TwitchError::Network | TwitchError::InvalidResponse)
                    if !status.is_success() =>
                {
                    let authenticated = request.url() == &self.endpoints.gql
                        || request
                            .url()
                            .as_str()
                            .starts_with(self.endpoints.helix.as_str())
                        || request.url().origin() == self.endpoints.oauth.origin()
                            && request.url().path().starts_with("/oauth2/");
                    return Err(
                        if authenticated
                            && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                        {
                            TwitchError::Unauthorized
                        } else {
                            TwitchError::Status(status.as_u16())
                        },
                    );
                }
                Err(TwitchError::Network)
                    if retry == RetryPolicy::Replay
                        && status != StatusCode::NO_CONTENT
                        && attempt < 4 =>
                {
                    self.sleep(Duration::from_secs(1 << attempt)).await?;
                }
                result => return result,
            }
        }
        Err(TwitchError::Network)
    }

    async fn read_response(
        &self,
        mut response: reqwest::Response,
        attempt: usize,
        capture: &diagnostics::Capture,
        started: Instant,
    ) -> Result<http::Response<Vec<u8>>, TwitchError> {
        let status = response.status();
        let version = response.version();
        let mut headers = response.headers().clone();
        // The OAuth library checks the exact JSON media type. Parameters do not
        // change the media type and must not reject Twitch's UTF-8 responses.
        if headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            })
        {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! { biased;
                _=self.cancel.cancelled()=>return Err(TwitchError::Cancelled),
                chunk=response.chunk()=>chunk.map_err(|error| {
                    capture.network(&error, "read_body", attempt, started.elapsed().as_millis());
                    TwitchError::Network
                })?,
            };
            let Some(chunk) = chunk else { break };
            if bytes.len() + chunk.len() > MAX_BODY {
                tracing::warn!(
                    status = status.as_u16(),
                    limit = MAX_BODY,
                    "Upstream response exceeds body limit"
                );
                return Err(TwitchError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut response = http::Response::builder()
            .status(status)
            .version(version)
            .body(bytes)
            .map_err(|_| TwitchError::InvalidResponse)?;
        *response.headers_mut() = headers;
        capture.response(&response, attempt, started.elapsed().as_millis());
        Ok(response)
    }
}

impl twitch_oauth2::client::Client for TwitchHttp {
    type Error = TwitchError;
    fn req(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> impl std::future::Future<Output = Result<http::Response<Vec<u8>>, Self::Error>> + Send + use<>
    {
        let owned = self.clone();
        async move { owned.oauth_request(request).await }
    }
}

impl TwitchHttp {
    async fn oauth_request(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, TwitchError> {
        let (parts, body) = request.into_parts();
        let mut original =
            Url::parse(&parts.uri.to_string()).map_err(|_| TwitchError::Configuration)?;
        if original.host_str() != Some("id.twitch.tv") || !original.path().starts_with("/oauth2/") {
            return Err(TwitchError::Configuration);
        }
        let mut body = body;
        let mut headers = parts.headers;
        if parts.method == Method::POST {
            // The library constructs OAuth parameters in the URL. Send those same
            // parameters as form data so credentials never appear in request URLs.
            let mut pairs: Vec<(String, String)> = original
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            pairs.extend(
                url::form_urlencoded::parse(&body).map(|(k, v)| (k.into_owned(), v.into_owned())),
            );
            if original.path() == "/oauth2/device" && !pairs.iter().any(|(k, _)| k == "scopes") {
                pairs.push(("scopes".into(), String::new()));
            }
            body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs)
                .finish()
                .into_bytes();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            );
            original.set_query(None);
        }
        let mut url = self
            .endpoints
            .oauth
            .join(original.path())
            .map_err(|_| TwitchError::Configuration)?;
        url.set_query(original.query());
        self.execute(
            self.request(parts.method, url)
                .headers(headers)
                .header(header::ORIGIN, CLIENT_ORIGIN)
                .header(header::REFERER, CLIENT_ORIGIN)
                .body(body),
            // A successful single-use OAuth exchange cannot safely be replayed.
            RetryPolicy::Transport,
        )
        .await
    }
}

pub(crate) fn success(status: StatusCode) -> Result<(), TwitchError> {
    match status {
        status if status.is_success() => Ok(()),
        status => Err(TwitchError::Status(status.as_u16())),
    }
}

#[derive(Clone)]
pub struct TwitchClient {
    pub http: Arc<TwitchHttp>,
    pub user_id: u64,
    access_token: String,
}

impl TwitchClient {
    pub fn new(http: Arc<TwitchHttp>, session: &oauth::Session) -> Self {
        Self {
            http,
            user_id: session.user_id,
            access_token: session.access_token.clone(),
        }
    }
    pub(crate) fn authorization(&self) -> Result<HeaderValue, TwitchError> {
        let mut value = HeaderValue::from_str(&format!("OAuth {}", self.access_token))
            .map_err(|_| TwitchError::Configuration)?;
        value.set_sensitive(true);
        Ok(value)
    }
    pub(crate) fn token(&self) -> &str {
        &self.access_token
    }

    #[tracing::instrument(skip_all, fields(operation = diagnostics::operation(&operation)))]
    pub async fn gql(&self, operation: Value) -> Result<Value, TwitchError> {
        let _permit = tokio::select! {biased;
            _=self.http.cancel.cancelled()=>return Err(TwitchError::Cancelled),
            permit=self.http.concurrent.acquire()=>permit.map_err(|_|TwitchError::Cancelled)?,
        };
        for attempt in 0..5 {
            self.http.acquire().await?;
            let response = self
                .http
                .execute(
                    self.http
                        .request(Method::POST, self.http.endpoints.gql.clone())
                        .header(header::AUTHORIZATION, self.authorization()?)
                        .header(header::ORIGIN, CLIENT_ORIGIN)
                        .header(header::REFERER, CLIENT_ORIGIN)
                        .json(&operation),
                    if operations::can_replay_response(&operation) {
                        RetryPolicy::Replay
                    } else {
                        RetryPolicy::Transport
                    },
                )
                .await?;
            // Only authenticated API responses can invalidate the saved session.
            // Public channel pages and settings scripts also return 401/403.
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) {
                return Err(TwitchError::Unauthorized);
            }
            success(response.status())?;
            let mut response = diagnostics::json(response.body(), response.status().as_u16())?;
            let retry = if let Some(batch) = response.as_array_mut() {
                let mut retry = false;
                let mut reported = 0;
                for (index, item) in batch.iter_mut().enumerate() {
                    if item.get("error").is_some()
                        || item["errors"]
                            .as_array()
                            .is_some_and(|errors| !errors.is_empty())
                    {
                        if reported < 4 {
                            diagnostics::graphql(
                                item,
                                diagnostics::operation(&operation[index]),
                                attempt,
                                index,
                            );
                        }
                        reported += 1;
                    }
                    retry |= gql_errors(item, attempt)?;
                }
                if reported > 4 {
                    tracing::warn!(
                        omitted = reported - 4,
                        "Additional GraphQL batch diagnostics omitted"
                    );
                }
                retry
            } else {
                diagnostics::graphql(&response, diagnostics::operation(&operation), attempt, 0);
                gql_errors(&mut response, attempt)?
            };
            if !retry {
                return Ok(response);
            }
            self.http
                .sleep(Duration::from_secs((1 << attempt).max(5)))
                .await?;
        }
        Err(TwitchError::GraphQl)
    }
}

fn gql_errors(response: &mut Value, attempt: u32) -> Result<bool, TwitchError> {
    if !response.is_object() {
        return Err(diagnostics::invalid(
            "GraphQL",
            "expected response object",
            Some(response),
        ));
    }
    if response.get("error").is_some() {
        return Err(TwitchError::GraphQl);
    }
    let errors = response["errors"].as_array().cloned().unwrap_or_default();
    let mut retry = false;
    for error in errors {
        match error["message"].as_str().unwrap_or_default() {
            "Unauthorized" | "unauthorized" | "authentication required" | "invalid oauth token" => {
                return Err(TwitchError::Unauthorized);
            }
            "service error" | "PersistedQueryNotFound" if attempt == 0 => retry = true,
            "service timeout" | "service unavailable" | "context deadline exceeded"
                if attempt < 4 =>
            {
                retry = true
            }
            "server error" => {
                let mut target = &mut response["data"];
                for part in error["path"].as_array().into_iter().flatten() {
                    // GraphQL null propagation may already have nulled a parent
                    // before reporting a deeper resolver path.
                    if target.is_null() {
                        break;
                    }
                    let child = if let Some(key) = part.as_str() {
                        target.get_mut(key)
                    } else if let Some(index) = part.as_u64().and_then(|v| usize::try_from(v).ok())
                    {
                        target.get_mut(index)
                    } else {
                        None
                    };
                    let Some(child) = child else {
                        return Err(TwitchError::GraphQl);
                    };
                    target = child;
                }
                *target = Value::Null;
            }
            _ => return Err(TwitchError::GraphQl),
        }
    }
    Ok(retry)
}
