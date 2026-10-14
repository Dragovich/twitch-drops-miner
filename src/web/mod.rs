mod releases;
pub mod socket;
#[cfg(test)]
mod tests;

use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use socketioxide::SocketIo;
use tokio::sync::mpsc;

use crate::{
    auth::{
        AuthAction, AuthError, AuthSettingsRequest, LoginRequest, WebAuth, session_cookie,
        token_from_headers, unix_now,
    },
    dto::SettingsView,
    origin::DashboardOrigin,
};
use socket::SocketHub;

use crate::app::{
    AppError, Application,
    api::{ChannelSelection, HistoryQuery},
    catalog,
};
pub use crate::app::{Command, CommandRequest, message};

pub struct WebState {
    pub application: Arc<Application>,
    pub auth: Arc<WebAuth>,
    pub origin: DashboardOrigin,
    pub sockets: SocketHub,
    releases: releases::Releases,
    #[cfg(feature = "dashboard-fixture")]
    pub fixture: bool,
}

// Routes compose the application with transport security; the miner owns no HTTP state.
impl std::ops::Deref for WebState {
    type Target = Arc<Application>;
    fn deref(&self) -> &Self::Target {
        &self.application
    }
}

pub type App = WebState;

impl WebState {
    pub fn open(
        directory: PathBuf,
        public_base_url: &str,
    ) -> Result<(Arc<Self>, mpsc::Receiver<CommandRequest>)> {
        let (application, receiver) = Application::open(directory)?;
        let auth = Arc::new(WebAuth::open(&application.data.path)?);
        Ok((
            Arc::new(Self {
                application,
                sockets: SocketHub::new(auth.clone()),
                auth,
                origin: DashboardOrigin::new(public_base_url)?,
                releases: releases::Releases::new()?,
                #[cfg(feature = "dashboard-fixture")]
                fixture: false,
            }),
            receiver,
        ))
    }
}

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str);

async fn games(
    State(app): State<Arc<App>>,
    Json(query): Json<crate::app::commands::GameQuery>,
) -> Result<Json<Vec<crate::config::GameMetadata>>, ApiError> {
    Ok(Json(app.games(query).await?))
}
async fn search_channels(
    State(app): State<Arc<App>>,
    Json(query): Json<crate::app::commands::ChannelQuery>,
) -> Result<Json<Vec<crate::dto::ChannelSearchResult>>, ApiError> {
    Ok(Json(app.search_channels(query).await?))
}

impl ApiError {
    fn invalid() -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request")
    }
    fn unavailable() -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "request_failed")
    }
}
impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Telegram(error) => Self(
                if error == twitch_drops_miner_core::notifications::Error::Conflict {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::BAD_REQUEST
                },
                error.code(),
            ),
            AppError::InvalidRequest => Self::invalid(),
            AppError::InvalidManualDuration => {
                Self(StatusCode::BAD_REQUEST, "invalid_manual_duration")
            }
            AppError::InvalidChannel => Self(StatusCode::BAD_REQUEST, "invalid_channel"),
            AppError::ChannelNotFound => Self(StatusCode::NOT_FOUND, "channel_not_found"),
            AppError::ShuttingDown => Self(StatusCode::CONFLICT, "shutting_down"),
            AppError::LoginRequired => Self(StatusCode::CONFLICT, "twitch_login_required"),
            AppError::SettingsConflict => Self(StatusCode::CONFLICT, "settings_conflict"),
            AppError::InvalidSettings => Self(StatusCode::BAD_REQUEST, "invalid_settings"),
            AppError::Unavailable => Self::unavailable(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"detail":self.1}))).into_response()
    }
}
impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut response =
            (self.status(), Json(json!({"detail":self.to_string()}))).into_response();
        if self == Self::RateLimited {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        response
    }
}

pub fn router(app: Arc<App>) -> Router {
    let (layer, io) = SocketIo::builder().max_payload(1024 * 1024).build_layer();
    SocketHub::attach(&app, io);
    let mut routes = Router::new()
        .route("/healthz", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/", get(index))
        .route("/campaigns", get(index))
        .route("/history", get(index))
        .route("/activity", get(index))
        .route("/settings", get(index))
        .route("/login", get(login_page))
        .route("/assets/{*path}", get(asset))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/settings", post(auth_settings))
        .route("/api/status", get(status))
        .route("/api/channels", get(channels))
        .route("/api/channels/select", post(select_channel))
        .route("/api/campaigns", get(campaigns))
        .route("/api/console", get(console))
        .route("/api/settings", get(settings).post(update_settings))
        .route("/api/telegram", get(telegram).post(save_telegram))
        .route("/api/telegram/bot", get(telegram_bot))
        .route("/api/telegram/test", post(test_telegram))
        .route("/api/games", post(games))
        .route("/api/channels/search", post(search_channels))
        .route("/api/settings/verify-proxy", post(verify_proxy))
        .route("/api/version", get(version))
        .route("/api/history", get(history))
        .route("/api/history/stats", get(history_stats))
        .route("/api/twitch/logout", post(logout_twitch))
        .route("/api/oauth/confirm", post(confirm_oauth))
        .route("/api/reload", post(reload))
        .route("/api/cache/clear", post(clear_cache))
        .route("/api/mode/exit-manual", post(exit_manual))
        .route("/api/close", post(close));
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        routes = super::fixture::routes(routes);
    }
    routes = routes
        .layer(layer)
        .layer(middleware::from_fn_with_state(app.clone(), guard));
    routes.with_state(app)
}

async fn guard(State(app): State<Arc<App>>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let secure_transport = request.uri().scheme_str() == Some("https");
    if !app
        .origin
        .permits(request.method(), &path, request.headers(), secure_transport)
    {
        return private(ApiError(StatusCode::FORBIDDEN, "forbidden").into_response());
    }
    let asset =
        path.starts_with("/assets/") && matches!(*request.method(), Method::GET | Method::HEAD);
    let public = asset
        || matches!(
            path.as_str(),
            "/login" | "/healthz" | "/api/auth/status" | "/api/auth/login"
        );
    #[cfg(feature = "dashboard-fixture")]
    let public = public
        || app.fixture
            && matches!(
                path.as_str(),
                "/__test/health" | "/__test/reset" | "/__test/event"
            );
    if !public
        && !app
            .auth
            .state
            .lock()
            .await
            .allowed(&token_from_headers(request.headers()), unix_now())
    {
        let response = if matches!(
            path.as_str(),
            "/" | "/campaigns" | "/history" | "/activity" | "/settings"
        ) {
            Redirect::to("/login").into_response()
        } else {
            ApiError(StatusCode::UNAUTHORIZED, "authentication_required").into_response()
        };
        return private(response);
    }
    // Bound every submitted document and suppress extractor errors that could echo secrets.
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) && !path.starts_with("/socket.io")
    {
        let limit = if path.starts_with("/api/auth/") {
            16384
        } else {
            1024 * 1024
        };
        let (parts, body) = request.into_parts();
        let Ok(body) = to_bytes(body, limit).await else {
            return private(
                ApiError(StatusCode::PAYLOAD_TOO_LARGE, "invalid_request").into_response(),
            );
        };
        request = Request::from_parts(parts, Body::from(body));
    }
    let response = next.run(request).await;
    if asset && response.status().is_success() {
        no_sniff(response)
    } else {
        private(response)
    }
}

// The browser counterpart of the desktop window policy: only bundled code runs, connections
// stay same-origin, and remote content is limited to HTTPS images such as Twitch artwork.
const DOCUMENT_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; font-src 'self' data:; connect-src 'self'; object-src 'none'; frame-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

fn no_sniff(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
}

fn private(response: Response) -> Response {
    let mut response = no_sniff(response);
    if !response.headers().contains_key(header::CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
        .headers_mut()
        .insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "same-origin".parse().unwrap());
    response
}

#[derive(rust_embed::RustEmbed)]
#[folder = "web/"]
struct Assets;

async fn index() -> Response {
    let mut response = serve_asset("index.html", false);
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        DOCUMENT_POLICY.parse().unwrap(),
    );
    response
}
async fn login_page(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if app
        .auth
        .state
        .lock()
        .await
        .allowed(&token_from_headers(&headers), unix_now())
    {
        Redirect::to("/").into_response()
    } else {
        index().await
    }
}
async fn asset(Path(path): Path<String>) -> Response {
    serve_asset(&format!("assets/{path}"), true)
}
fn serve_asset(path: &str, immutable: bool) -> Response {
    if path
        .split('/')
        .any(|part| matches!(part, "." | "..") || part.contains('\\'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(asset) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (
                header::CONTENT_TYPE,
                mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
            ),
            (
                header::CACHE_CONTROL,
                if immutable {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }
                .to_owned(),
            ),
        ],
        asset.data.into_owned(),
    )
        .into_response()
}

async fn document<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, ApiError> {
    let bytes = to_bytes(request.into_body(), 1024 * 1024)
        .await
        .map_err(|_| ApiError::invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::invalid())
}

async fn auth_status(
    State(app): State<Arc<App>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Json<Value> {
    // An explicit choice wins; otherwise the browser language localizes the login screen.
    let language = query
        .get("lang")
        .map(|code| crate::config::Language::from_code(code))
        .unwrap_or_else(|| {
            headers
                .get(header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .map(crate::config::Language::from_accept_language)
                .unwrap_or_default()
        });
    let state = app.auth.state.lock().await;
    Json(
        json!({"enabled":state.enabled(),"authenticated":state.allowed(&token_from_headers(&headers),unix_now()),"translations":catalog(language)["gui"]["auth"]}),
    )
}

fn peer(request: &Request) -> Option<std::net::IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0.ip())
}
fn cookie_secure(app: &App, request: &Request) -> bool {
    app.origin
        .secure_cookie(request.uri().scheme_str() == Some("https"))
}
async fn auth_response(
    app: &App,
    token: &str,
    remember: bool,
    secure: bool,
    disconnect_all: bool,
) -> Response {
    app.sockets.prune(disconnect_all).await;
    let enabled = app.auth.state.lock().await.enabled();
    (
        [(header::SET_COOKIE, session_cookie(token, remember, secure))],
        Json(json!({"success":true,"enabled":enabled})),
    )
        .into_response()
}
async fn auth_login(State(app): State<Arc<App>>, request: Request) -> Response {
    let previous = token_from_headers(request.headers());
    let peer = peer(&request);
    let secure = cookie_secure(&app, &request);
    let data = match document::<LoginRequest>(request).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let remember = data.remember;
    match app.auth.login(data, &previous, peer).await {
        Ok(token) => auth_response(&app, &token, remember, secure, false).await,
        Err(e) => e.into_response(),
    }
}
async fn auth_logout(State(app): State<Arc<App>>, request: Request) -> Response {
    let token = token_from_headers(request.headers());
    let secure = cookie_secure(&app, &request);
    match app.auth.logout(&token).await {
        Ok(()) => auth_response(&app, "", false, secure, false).await,
        Err(e) => e.into_response(),
    }
}
async fn auth_settings(State(app): State<Arc<App>>, request: Request) -> Response {
    let token = token_from_headers(request.headers());
    let peer = peer(&request);
    let secure = cookie_secure(&app, &request);
    let data = match document::<AuthSettingsRequest>(request).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let disable = data.action == AuthAction::Disable;
    match app.auth.configure(data, &token, peer).await {
        Ok(token) => auth_response(&app, &token, false, secure, disable).await,
        Err(e) => e.into_response(),
    }
}

async fn status(State(app): State<Arc<App>>) -> Json<Value> {
    let state = app.snapshot.read().await;
    Json(json!({"status":state.status,"login":state.login,"manual_mode":state.manual_mode}))
}
async fn channels(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"channels":app.snapshot.read().await.channels}))
}
async fn campaigns(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"campaigns":app.snapshot.read().await.campaigns}))
}
async fn console(State(app): State<Arc<App>>) -> Json<Value> {
    Json(json!({"lines":app.snapshot.read().await.console}))
}
async fn settings(State(app): State<Arc<App>>) -> Json<SettingsView> {
    Json(app.snapshot.read().await.settings.clone())
}

async fn update_settings(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let patch: Value = document(request).await?;
    let settings = app.application.update_settings(patch).await?;
    Ok(Json(json!({"success":true,"settings":settings})))
}

async fn select_channel(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let selection: ChannelSelection = document(request).await?;
    app.application.select_channel(selection).await?;
    Ok(Json(json!({"success":true})))
}

async fn history(State(app): State<Arc<App>>, Query(query): Query<HistoryQuery>) -> Json<Value> {
    Json(app.application.history(query).await)
}

async fn history_stats(State(app): State<Arc<App>>) -> Json<Value> {
    Json(app.history.lock().await.stats())
}
async fn run_command(app: &App, command: Command) -> Result<Json<Value>, ApiError> {
    app.command(command).await?;
    Ok(Json(json!({"success":true})))
}
async fn reload(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    app.refresh_inventory().await?;
    Ok(Json(json!({"success":true})))
}
async fn clear_cache(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    app.clear_cache().await?;
    Ok(Json(json!({"success":true})))
}
async fn logout_twitch(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::Logout).await
}
async fn confirm_oauth(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::ConfirmOAuth).await
}
async fn exit_manual(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::ExitManual).await
}
async fn close(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    run_command(&app, Command::Shutdown).await
}

async fn verify_proxy(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    #[derive(Deserialize)]
    struct Proxy {
        proxy: String,
    }
    let Proxy { proxy } = document(request).await?;
    crate::config::validate_proxy(&proxy).map_err(|_| ApiError::invalid())?;
    if proxy.is_empty() {
        return Ok(Json(
            json!({"success":false,"message":app.application.text("gui.backend.proxy_empty",&[]).await}),
        ));
    }
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        return Ok(Json(json!({"success":true})));
    }
    Ok(Json(app.application.verify_proxy(&proxy).await?))
}

async fn version(State(app): State<Arc<App>>) -> Json<releases::ReleaseInfo> {
    #[cfg(feature = "dashboard-fixture")]
    if app.fixture {
        return Json(releases::ReleaseInfo::default());
    }
    Json(app.releases.check(&app.shutdown).await)
}

async fn telegram(
    State(app): State<Arc<App>>,
) -> Json<twitch_drops_miner_core::notifications::View> {
    Json(app.telegram.view())
}
async fn save_telegram(
    State(app): State<Arc<App>>,
    request: Request,
) -> Result<Json<twitch_drops_miner_core::notifications::View>, ApiError> {
    let update = document(request).await?;
    Ok(Json(app.application.save_telegram(update).await?))
}
async fn telegram_bot(
    State(app): State<Arc<App>>,
) -> Json<twitch_drops_miner_core::notifications::BotView> {
    Json(app.telegram_bot().await)
}
async fn test_telegram(State(app): State<Arc<App>>) -> Result<Json<Value>, ApiError> {
    app.test_telegram().await?;
    Ok(Json(json!({"success":true})))
}
