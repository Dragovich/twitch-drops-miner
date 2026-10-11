pub mod records;
use records::StoredCampaign;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    config::{Settings, fold},
    domain::{Campaign, Drop},
    dto::{CampaignView, HistoryEntry},
};

pub struct DataDirectory {
    pub path: PathBuf,
    _lock: File,
}

impl DataDirectory {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        fs::create_dir_all(&path).context("cannot create data directory")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join(".miner.lock"))
            .context("cannot open data directory lock")?;
        lock.try_lock()
            .context("data directory is already in use or cannot be locked")?;
        Ok(Self { path, _lock: lock })
    }

    pub fn settings(&self) -> Result<Settings> {
        read_json(&self.path.join("settings.json"))?
            .map(Settings::from_saved)
            .transpose()
            .context("invalid settings file; original file preserved")
            .map(Option::unwrap_or_default)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        atomic_json(&self.path.join("settings.json"), settings)
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .context("invalid saved data"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("cannot read saved data"),
    }
}

pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .context("saved data requires a parent directory")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).context("cannot prepare saved data")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .context("cannot protect saved data")?;
    }
    serde_json::to_writer_pretty(&mut temporary, value).context("cannot encode saved data")?;
    temporary
        .write_all(b"\n")
        .context("cannot write saved data")?;
    temporary
        .as_file()
        .sync_all()
        .context("cannot sync saved data")?;
    temporary
        .persist(path)
        .map_err(|_| anyhow::anyhow!("cannot replace saved data"))?;
    #[cfg(unix)]
    if File::open(parent)
        .and_then(|directory| directory.sync_all())
        .is_err()
    {
        // Replacement already committed. Keep memory consistent with disk even on
        // filesystems that cannot sync directories; report weaker crash durability.
        tracing::warn!("Saved data was replaced, but the directory could not be synced");
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    entries: Vec<HistoryEntry>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    cleared_ids: BTreeSet<String>,
}

pub struct History {
    path: PathBuf,
    entries: Vec<HistoryEntry>,
    cleared_ids: BTreeSet<String>,
    pub writable: bool,
}

#[derive(Default)]
pub struct HistoryFilter {
    pub game: Option<String>,
    pub campaign_id: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

impl History {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join("drop_history.json");
        let loaded = read_json::<HistoryFile>(&path).and_then(|value| {
            let Some(value) = value else {
                return Ok((vec![], BTreeSet::new()));
            };
            let mut seen = HashSet::new();
            if value.version != 1
                || value.entries.iter().any(|e| {
                    e.id.is_empty() || !seen.insert(&e.id) || value.cleared_ids.contains(&e.id)
                })
                || value.cleared_ids.iter().any(String::is_empty)
            {
                bail!("invalid history");
            }
            Ok((value.entries, value.cleared_ids))
        });
        match loaded {
            Ok((entries, cleared_ids)) => Self {
                path,
                entries,
                cleared_ids,
                writable: true,
            },
            Err(_) => {
                tracing::error!("Cannot read claim history; preserving the original file");
                Self {
                    path,
                    entries: vec![],
                    cleared_ids: BTreeSet::new(),
                    writable: false,
                }
            }
        }
    }

    pub fn record(&mut self, entry: HistoryEntry) -> Result<bool> {
        Ok(self.import_claims(vec![entry])? > 0)
    }

    pub fn import_claims(&mut self, entries: Vec<HistoryEntry>) -> Result<usize> {
        let mut next = self.entries.clone();
        let mut known: HashSet<_> = next.iter().map(|e| e.id.clone()).collect();
        for entry in entries {
            if !self.cleared_ids.contains(&entry.id) && known.insert(entry.id.clone()) {
                next.push(entry);
            }
        }
        let added = next.len() - self.entries.len();
        if added > 0 {
            next.sort_by(|a, b| {
                a.claimed_at
                    .cmp(&b.claimed_at)
                    .then_with(|| a.id.cmp(&b.id))
            });
            self.replace(next, self.cleared_ids.clone())?;
        }
        Ok(added)
    }

    fn replace(&mut self, entries: Vec<HistoryEntry>, cleared_ids: BTreeSet<String>) -> Result<()> {
        if !self.writable {
            bail!("claim history is unreadable; original file preserved");
        }
        let next = HistoryFile {
            version: 1,
            entries,
            cleared_ids,
        };
        atomic_json(&self.path, &next)?;
        self.entries = next.entries;
        self.cleared_ids = next.cleared_ids;
        Ok(())
    }

    pub fn clear(&mut self) -> Result<()> {
        let mut cleared = self.cleared_ids.clone();
        cleared.extend(self.entries.iter().map(|entry| entry.id.clone()));
        self.replace(vec![], cleared)
    }

    #[cfg(feature = "dashboard-fixture")]
    pub fn reset_fixture(&mut self) -> Result<()> {
        self.replace(vec![], BTreeSet::new())
    }
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn entries(&self, filter: &HistoryFilter) -> Vec<HistoryEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|e| {
                filter
                    .game
                    .as_ref()
                    .is_none_or(|game| fold(game) == fold(&e.game))
                    && filter
                        .campaign_id
                        .as_ref()
                        .is_none_or(|id| id == &e.campaign_id)
                    && filter.since.is_none_or(|since| since <= e.claimed_at)
            })
            .take(filter.limit.unwrap_or(usize::MAX))
            .cloned()
            .collect()
    }

    pub fn stats(&self) -> serde_json::Value {
        let mut games = BTreeMap::<String, usize>::new();
        let mut months = BTreeMap::<String, usize>::new();
        for entry in &self.entries {
            *games.entry(entry.game.clone()).or_default() += 1;
            *months
                .entry(entry.claimed_at.format("%Y-%m").to_string())
                .or_default() += 1;
        }
        serde_json::json!({"total_drops": self.total(), "by_game": games, "by_month": months})
    }
}

#[derive(Serialize, Deserialize)]
struct ArchiveFile {
    version: u32,
    campaigns: Vec<StoredCampaign>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingClaim {
    pub user_id: u64,
    pub entry: HistoryEntry,
    pub instance: String,
    pub benefits: Vec<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub retry_until: DateTime<Utc>,
    pub completed_campaign: Option<StoredCampaign>,
    pub confirmed: bool,
    #[serde(default)]
    pub settled: bool,
    #[serde(default)]
    pub linked: Option<bool>,
}

impl PendingClaim {
    pub fn new(user_id: u64, campaign: &Campaign, drop: &Drop, settings: &Settings) -> Self {
        let now = Utc::now();
        let mut completed = campaign.clone();
        completed
            .drops
            .iter_mut()
            .find(|d| d.id == drop.id)
            .unwrap()
            .mark_claimed(now);
        let completed = completed.view(settings, now);
        Self {
            user_id,
            entry: campaign.history_entry(drop, now),
            instance: drop.claim_id.clone().expect("account claim ID required"),
            benefits: drop.benefits.iter().map(|b| b.id.clone()).collect(),
            starts_at: drop.starts_at,
            ends_at: drop.ends_at,
            retry_until: campaign.ends_at + chrono::Duration::hours(24),
            completed_campaign: completed.finished.then(|| completed.into()),
            confirmed: false,
            settled: false,
            linked: campaign.linked,
        }
    }

    pub fn confirmed_by(&self, awards: &std::collections::HashMap<String, DateTime<Utc>>) -> bool {
        !self.benefits.is_empty()
            && self.benefits.iter().all(|id| {
                awards
                    .get(id)
                    .is_some_and(|at| self.starts_at <= *at && *at < self.ends_at)
            })
    }
}

pub struct ClaimJournal {
    path: PathBuf,
    entries: Vec<PendingClaim>,
}

impl ClaimJournal {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("pending_claims.json");
        let entries = read_json::<Vec<PendingClaim>>(&path)?.unwrap_or_default();
        let mut ids = std::collections::HashSet::new();
        let mut pending_ids = std::collections::HashSet::new();
        if entries.iter().any(|claim| {
            claim.user_id == 0
                || claim.entry.id.is_empty()
                || claim.instance.is_empty()
                || claim.starts_at >= claim.ends_at
                || claim.completed_campaign.as_ref().is_some_and(|c| {
                    c.id != claim.entry.campaign_id || !CampaignArchive::valid(&c.clone().into())
                })
                || (!claim.settled && !pending_ids.insert((claim.user_id, &claim.entry.id)))
                || (claim.settled && !claim.confirmed)
                || !ids.insert((
                    claim.user_id,
                    &claim.entry.id,
                    &claim.instance,
                    claim.linked,
                ))
        }) {
            bail!("pending claims are unreadable; original file preserved");
        }
        Ok(Self { path, entries })
    }
    pub fn pending(&self, user_id: u64) -> Vec<PendingClaim> {
        self.entries
            .iter()
            .filter(|e| e.user_id == user_id && !e.settled)
            .cloned()
            .collect()
    }
    pub fn prepare(&mut self, claim: PendingClaim) -> Result<PendingClaim> {
        if let Some(existing) = self.entries.iter().find(|e| {
            e.user_id == claim.user_id
                && e.entry.id == claim.entry.id
                && (!e.settled
                    || (e.instance == claim.instance
                        && !(e.linked == Some(false) && claim.linked == Some(true))))
        }) {
            return Ok(existing.clone());
        }
        let mut entries = self.entries.clone();
        entries.push(claim.clone());
        atomic_json(&self.path, &entries)?;
        self.entries = entries;
        Ok(claim)
    }
    pub fn confirm(&mut self, user_id: u64, id: &str) -> Result<()> {
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|p| p.user_id == user_id && p.entry.id == id && !p.settled)
            .context("claim intent is missing")?;
        entry.confirmed = true;
        atomic_json(&self.path, &entries)?;
        self.entries = entries;
        Ok(())
    }
    pub fn acknowledged(&self, user_id: u64, drop: &Drop, linked: Option<bool>) -> bool {
        self.entries.iter().any(|e| {
            e.user_id == user_id
                && e.entry.id == drop.id
                && e.settled
                && drop.claim_id.as_deref() == Some(e.instance.as_str())
                && !(e.linked == Some(false) && linked == Some(true))
        })
    }
    pub fn settled(&self, user_id: u64, id: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.user_id == user_id && e.entry.id == id && e.settled)
    }
    pub fn settle(&mut self, user_id: u64, id: &str) -> Result<()> {
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|e| e.user_id == user_id && e.entry.id == id && !e.settled)
            .context("claim intent is missing")?;
        entry.confirmed = true;
        entry.settled = true;
        atomic_json(&self.path, &entries)?;
        self.entries = entries;
        Ok(())
    }
    pub fn prune(&mut self, now: DateTime<Utc>) -> Result<()> {
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|e| !e.settled || now < e.retry_until)
            .cloned()
            .collect();
        if entries.len() != self.entries.len() {
            atomic_json(&self.path, &entries)?;
            self.entries = entries;
        }
        Ok(())
    }
    pub fn finish(&mut self, user_id: u64, id: &str) -> Result<()> {
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.user_id != user_id || e.entry.id != id || e.settled)
            .cloned()
            .collect();
        if entries.len() == self.entries.len() {
            return Ok(());
        }
        atomic_json(&self.path, &entries)?;
        self.entries = entries;
        Ok(())
    }
}

pub struct CampaignArchive {
    path: PathBuf,
    campaigns: BTreeMap<String, CampaignView>,
    pub writable: bool,
}

impl CampaignArchive {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join("completed_campaigns.json");
        let loaded = read_json::<ArchiveFile>(&path).and_then(|value| {
            let Some(value) = value else {
                return Ok(BTreeMap::new());
            };
            let mut campaigns = BTreeMap::new();
            if value.version != 1 {
                bail!("unknown campaign archive version");
            }
            for campaign in value.campaigns {
                let campaign: CampaignView = campaign.into();
                if !Self::valid(&campaign)
                    || campaigns.insert(campaign.id.clone(), campaign).is_some()
                {
                    bail!("invalid campaign archive");
                }
            }
            Ok(campaigns)
        });
        match loaded {
            Ok(campaigns) => Self {
                path,
                campaigns,
                writable: true,
            },
            Err(_) => {
                tracing::error!("Cannot read completed campaigns; preserving the original file");
                Self {
                    path,
                    campaigns: BTreeMap::new(),
                    writable: false,
                }
            }
        }
    }

    fn valid(c: &CampaignView) -> bool {
        let mut seen = HashSet::new();
        c.finished
            && !c.id.is_empty()
            && !c.drops.is_empty()
            && c.total_drops == c.drops.len()
            && c.claimed_drops == c.drops.len()
            && c.drops.iter().all(|d| {
                d.is_claimed && d.required_minutes > 0 && !d.id.is_empty() && seen.insert(&d.id)
            })
    }

    pub fn update(&mut self, live: &[CampaignView]) -> Result<()> {
        let mut next = self.campaigns.clone();
        for campaign in live {
            if campaign.finished && Self::valid(campaign) {
                next.insert(campaign.id.clone(), campaign.clone());
            } else if let Some(previous) = next.get(&campaign.id) {
                let ids =
                    |c: &CampaignView| c.drops.iter().map(|d| d.id.clone()).collect::<HashSet<_>>();
                if campaign
                    .drops
                    .iter()
                    .any(|d| !d.is_claimed && d.confirmed_at.is_some())
                    || ids(campaign) != ids(previous)
                {
                    next.remove(&campaign.id);
                }
            }
        }
        if next != self.campaigns {
            if !self.writable {
                bail!("campaign archive is unreadable; original file preserved");
            }
            atomic_json(
                &self.path,
                &ArchiveFile {
                    version: 1,
                    campaigns: next.values().cloned().map(Into::into).collect(),
                },
            )?;
            self.campaigns = next;
        }
        Ok(())
    }

    pub fn merge(&self, live: Vec<CampaignView>, now: DateTime<Utc>) -> Vec<CampaignView> {
        let mut combined: BTreeMap<_, _> = live.into_iter().map(|c| (c.id.clone(), c)).collect();
        for (id, archived) in &self.campaigns {
            let mut archived = archived.clone();
            archived.active = archived.starts_at <= now && now < archived.ends_at;
            archived.upcoming = now < archived.starts_at;
            archived.expired = archived.ends_at <= now;
            combined.insert(id.clone(), archived);
        }
        combined.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str) -> HistoryEntry {
        serde_json::from_value(
            json!({"id":id, "claimed_at":"2026-01-01T23:45:00-01:00", "game":"Rust",
            "campaign":"Winter", "drop_name":"Coat, warm", "benefits":["Coat", "Boots"],
            "required_minutes": 30, "campaign_id":"c"}),
        )
        .unwrap()
    }

    #[test]
    fn settings_and_exclusive_lock_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDirectory::open(dir.path()).unwrap();
        assert!(DataDirectory::open(dir.path()).is_err());
        let settings = Settings::default()
            .patched(&json!({"games_to_watch":["Rust"],"connection_quality": 3}))
            .unwrap();
        data.save_settings(&settings).unwrap();
        assert_eq!(data.settings().unwrap().games_to_watch, ["Rust"]);
        drop(data);
        assert_eq!(
            DataDirectory::open(dir.path())
                .unwrap()
                .settings()
                .unwrap()
                .connection_quality,
            3
        );
    }

    #[test]
    fn history_preserves_old_artless_entries_and_filters_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::load(dir.path());
        assert!(history.record(entry("a")).unwrap());
        assert!(!history.record(entry("a")).unwrap());
        history.record(entry("b")).unwrap();
        let restored = History::load(dir.path());
        let filter = HistoryFilter {
            game: Some("RUST".into()),
            since: Some("2026-01-02T00:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            restored
                .entries(&filter)
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(restored.entries(&filter)[0].image_url.is_empty());
        history.clear().unwrap();
        assert_eq!(History::load(dir.path()).total(), 0);
    }

    #[test]
    fn imported_claims_are_deduplicated_sorted_and_clearing_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::load(dir.path());
        let recent = entry("recent");
        let mut old = entry("older");
        old.claimed_at -= chrono::Duration::days(1);
        old.claimed_at_is_observed = true;
        history.record(recent.clone()).unwrap();
        assert_eq!(
            history
                .import_claims(vec![old.clone(), recent.clone(), old.clone()])
                .unwrap(),
            1
        );
        let mut history = History::load(dir.path());
        let entries = history.entries(&HistoryFilter::default());
        assert_eq!(entries[0].id, "recent");
        assert!(entries[1].claimed_at_is_observed);
        history.clear().unwrap();
        let mut history = History::load(dir.path());
        assert!(
            !history.record(recent.clone()).unwrap(),
            "late claim recovery must respect clearing too"
        );
        assert_eq!(history.import_claims(vec![old, recent]).unwrap(), 0);
        assert_eq!(history.total(), 0);
        assert_eq!(history.import_claims(vec![entry("new")]).unwrap(), 1);
        assert_eq!(History::load(dir.path()).total(), 1);
    }

    #[test]
    fn unreadable_files_survive_mutations() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "drop_history.json",
            "completed_campaigns.json",
            "settings.json",
        ] {
            fs::write(dir.path().join(name), b"{broken").unwrap();
        }
        let mut history = History::load(dir.path());
        assert!(!history.writable);
        assert!(history.record(entry("a")).is_err());
        assert!(history.import_claims(vec![entry("b")]).is_err());
        assert!(history.clear().is_err());
        assert!(!CampaignArchive::load(dir.path()).writable);
        assert!(DataDirectory::open(dir.path()).unwrap().settings().is_err());
        for name in [
            "drop_history.json",
            "completed_campaigns.json",
            "settings.json",
        ] {
            assert_eq!(fs::read(dir.path().join(name)).unwrap(), b"{broken");
        }
    }

    #[test]
    fn failed_replace_does_not_change_in_memory_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::load(dir.path());
        history.record(entry("a")).unwrap();
        history.path = dir.path().join("missing").join("history.json");
        assert!(history.record(entry("b")).is_err());
        assert!(history.import_claims(vec![entry("b"), entry("c")]).is_err());
        assert!(history.clear().is_err());
        assert!(history.cleared_ids.is_empty());
        assert_eq!(history.total(), 1);
        assert_eq!(History::load(dir.path()).total(), 1);
    }
}
