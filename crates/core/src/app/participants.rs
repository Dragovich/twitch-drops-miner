use super::{AppError, Application};
use crate::{
    domain::{ChannelIdentity, Game},
    dto::Snapshot,
    twitch::{
        TwitchClient, TwitchError,
        channels::{ChannelStatus, channel_login},
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

const PAGE_SIZE: usize = 10;
const FRESH: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub campaign_id: String,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub campaign_id: String,
    pub offset: usize,
    pub total: usize,
    pub channels: Vec<Participant>,
}

#[derive(Debug, Serialize)]
pub struct Participant {
    pub id: u64,
    pub login: String,
    pub name: String,
    pub online: Option<bool>,
    pub eligible: Option<bool>,
    pub game: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    account: u64,
    game: Game,
    channels: Vec<ChannelIdentity>,
    total: usize,
}

impl Query {
    fn scope(&self, snapshot: &Snapshot) -> Result<Scope, AppError> {
        if self.campaign_id.is_empty()
            || self.campaign_id.len() > 256
            || !self.offset.is_multiple_of(PAGE_SIZE)
        {
            return Err(AppError::InvalidRequest);
        }
        let account = snapshot.login.user_id.ok_or(AppError::LoginRequired)?;
        let campaign = snapshot
            .campaigns
            .iter()
            .find(|c| c.id == self.campaign_id && !c.finished && c.ends_at > Utc::now())
            .ok_or(AppError::InvalidRequest)?;
        if campaign.game_id == 0 || self.offset >= campaign.allowed_channels.len() {
            return Err(AppError::InvalidRequest);
        }
        let channels = campaign
            .allowed_channels
            .iter()
            .skip(self.offset)
            .take(PAGE_SIZE)
            .map(|channel| {
                let login = channel_login(&channel.login).ok_or(AppError::InvalidRequest)?;
                if channel.id == 0 {
                    return Err(AppError::InvalidRequest);
                }
                Ok(ChannelIdentity {
                    id: channel.id,
                    login,
                    name: channel.name.clone(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Scope {
            account,
            game: Game {
                id: campaign.game_id,
                name: campaign.game_name.clone(),
                slug: String::new(),
                image_url: String::new(),
            },
            channels,
            total: campaign.allowed_channels.len(),
        })
    }
}

type Statuses = Vec<(ChannelStatus, DateTime<Utc>)>;

pub(crate) struct Request {
    channels: Vec<ChannelIdentity>,
    complete: oneshot::Sender<Result<Statuses, TwitchError>>,
}

impl Application {
    pub async fn campaign_channels(&self, query: Query) -> Result<Page, AppError> {
        let scope = query.scope(&*self.snapshot.read().await)?;
        let sender = self
            .participant_queries
            .read()
            .await
            .clone()
            .ok_or(AppError::Unavailable)?;
        let (complete, result) = oneshot::channel();
        sender
            .try_send(Request {
                channels: scope.channels.clone(),
                complete,
            })
            .map_err(|_| AppError::Unavailable)?;
        let statuses = tokio::select! { biased;
            _ = self.shutdown.cancelled() => return Err(AppError::ShuttingDown),
            result = tokio::time::timeout(Duration::from_secs(12), result) => result.map_err(|_| AppError::Unavailable)?
                .map_err(|_| AppError::Unavailable)?.map_err(|_| AppError::Unavailable)?,
        };
        if query.scope(&*self.snapshot.read().await)? != scope {
            return Err(AppError::Unavailable);
        }
        let channels = statuses
            .into_iter()
            .map(|(status, checked_at)| {
                let channel = &status.channel;
                let eligible = match status.online {
                    Some(false) => Some(false),
                    Some(true)
                        if scope.game.special() || status.drops_known && channel.game.is_some() =>
                    {
                        Some(channel.matches_campaign(&scope.game, &scope.channels))
                    }
                    _ => None,
                };
                Participant {
                    id: channel.identity.id,
                    login: channel.identity.login.clone(),
                    name: channel.identity.name.clone(),
                    online: status.online,
                    eligible,
                    game: channel.game.as_ref().map(|g| g.name.clone()),
                    checked_at,
                }
            })
            .collect();
        Ok(Page {
            campaign_id: query.campaign_id,
            offset: query.offset,
            total: scope.total,
            channels,
        })
    }
}

// One generation-owned reader keeps detail lookups independent of watching and discovery.
pub(crate) async fn run(
    client: TwitchClient,
    mut requests: mpsc::Receiver<Request>,
) -> Result<(), TwitchError> {
    let mut cache = HashMap::<(u64, String), (Instant, DateTime<Utc>, ChannelStatus)>::new();
    let mut next_read = Instant::now();
    loop {
        let mut request = tokio::select! { biased;
            _ = client.http.cancel.cancelled() => return Err(TwitchError::Cancelled),
            request = requests.recv() => request.ok_or(TwitchError::Cancelled)?,
        };
        let read = async {
            cache.retain(|_, (at, _, _)| at.elapsed() < FRESH);
            if cache.len() + request.channels.len() > 200 {
                cache.clear();
            }
            let missing: Vec<_> = request
                .channels
                .iter()
                .filter(|c| !cache.contains_key(&(c.id, c.login.clone())))
                .cloned()
                .collect();
            if !missing.is_empty() {
                tokio::time::sleep_until(next_read).await;
                next_read = Instant::now() + Duration::from_secs(5);
                let statuses =
                    tokio::time::timeout(Duration::from_secs(8), client.channel_statuses(&missing))
                        .await
                        .map_err(|_| TwitchError::Network)??;
                let now = Utc::now();
                for status in statuses {
                    cache.insert(
                        (
                            status.channel.identity.id,
                            status.channel.identity.login.clone(),
                        ),
                        (Instant::now(), now, status),
                    );
                }
            }
            request
                .channels
                .iter()
                .map(|c| {
                    cache
                        .get(&(c.id, c.login.clone()))
                        .map(|(_, at, status)| (status.clone(), *at))
                        .ok_or(TwitchError::InvalidResponse)
                })
                .collect()
        };
        let result = tokio::select! { biased;
            _ = client.http.cancel.cancelled() => return Err(TwitchError::Cancelled),
            _ = request.complete.closed() => continue,
            result = read => result,
        };
        let fatal = match result {
            Err(error @ (TwitchError::Unauthorized | TwitchError::Cancelled)) => Some(error),
            _ => None,
        };
        let _ = request.complete.send(result);
        if let Some(error) = fatal {
            return Err(error);
        }
    }
}

#[cfg(test)]
#[path = "participants_tests.rs"]
mod tests;
