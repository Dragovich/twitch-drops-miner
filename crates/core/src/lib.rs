pub mod app;
pub mod config;
pub mod domain;
pub mod dto;
pub mod miner;
pub mod notifications;
pub mod policy;
pub mod store;
pub mod twitch;

pub fn random_hex<const N: usize>() -> anyhow::Result<String> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("secure random generator failed"))?;
    Ok(hex::encode(bytes))
}

pub mod logging;
pub mod runtime;
