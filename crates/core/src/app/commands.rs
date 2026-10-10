use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameQuery {
    Search(String),
    Names(Vec<String>),
}

impl GameQuery {
    pub fn valid(&self) -> bool {
        let valid_name = |name: &str| {
            !name.trim().is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control)
        };
        match self {
            Self::Search(query) => valid_name(query) && query.len() <= 100,
            Self::Names(names) => {
                !names.is_empty()
                    && names.len() <= 100
                    && names.iter().all(|name| valid_name(name))
                    && names.iter().map(String::len).sum::<usize>() <= 4000
            }
        }
    }
}

pub struct GameRequest {
    pub query: GameQuery,
    pub complete:
        oneshot::Sender<Result<Vec<crate::config::GameMetadata>, crate::twitch::TwitchError>>,
}

#[derive(Clone, Debug)]
pub enum Command {
    Refresh { clear_cache: bool },
    SettingsChanged,
    SelectChannel(u64, Option<Duration>),
    MineChannel(String, Option<Duration>),
    ExitManual,
    ConfirmOAuth,
    Logout,
    Shutdown,
}

pub struct CommandRequest {
    pub command: Command,
    pub complete: oneshot::Sender<Result<(), String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Telegram(#[from] crate::notifications::Error),
    #[error("invalid_request")]
    InvalidRequest,
    #[error("invalid_manual_duration")]
    InvalidManualDuration,
    #[error("invalid_channel")]
    InvalidChannel,
    #[error("channel_not_found")]
    ChannelNotFound,
    #[error("shutting_down")]
    ShuttingDown,
    #[error("request_failed")]
    Unavailable,
    #[error("twitch_login_required")]
    LoginRequired,
    #[error("settings_conflict")]
    SettingsConflict,
    #[error("invalid_settings")]
    InvalidSettings,
}
