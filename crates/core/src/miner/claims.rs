use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClaimOutcome {
    Claimed,
    NotClaimed,
    Disabled,
}

impl Mining {
    pub(super) async fn recover_claims(
        &mut self,
        awards: &HashMap<String, chrono::DateTime<Utc>>,
    ) -> Result<(), TwitchError> {
        let unclaimed: HashSet<_> = self
            .campaigns
            .iter()
            .flat_map(|c| &c.drops)
            .filter(|d| !d.claimed && d.confirmed_at.is_some())
            .map(|d| d.id.clone())
            .collect();
        let confirmed: HashSet<_> = self
            .campaigns
            .iter()
            .flat_map(|c| &c.drops)
            .filter(|d| d.claimed)
            .map(|d| d.id.clone())
            .collect();
        let app = self.app.clone();
        let journal = self.journal.clone();
        let user_id = self.client.user_id;
        let awards = awards.clone();
        let rejected_account_ids = self.rejected_account_ids.clone();
        let (pending, recovered) = tokio::task::spawn_blocking(move || {
            let mut journal = journal.blocking_lock();
            let mut recovered = HashMap::new();
            for pending in journal.pending(user_id).into_iter().filter(|p| {
                p.confirmed
                    || confirmed.contains(&p.entry.id)
                    || (!rejected_account_ids.contains(&p.entry.campaign_id)
                        && !unclaimed.contains(&p.entry.id)
                        && p.confirmed_by(&awards))
            }) {
                record_claim(&app, &pending)?;
                journal.settle(user_id, &pending.entry.id)?;
                recovered.insert(pending.entry.id.clone(), pending.entry.claimed_at);
            }
            journal.prune(Utc::now())?;
            Ok::<_, anyhow::Error>((journal.pending(user_id), recovered))
        })
        .await
        .map_err(|_| TwitchError::Storage)?
        .map_err(|_| TwitchError::Storage)?;
        self.pending_claims = pending;
        for drop in self.campaigns.iter_mut().flat_map(|c| &mut c.drops) {
            if let Some(at) = recovered.get(&drop.id)
                && self
                    .unclaimed_inventory
                    .get(&drop.id)
                    .is_none_or(|confirmed| *confirmed <= *at)
            {
                drop.mark_claimed(*at);
            }
        }
        Ok(())
    }
}

pub(super) async fn claim(
    app: &Arc<App>,
    client: &TwitchClient,
    journal: &Arc<Mutex<ClaimJournal>>,
    pending_claim: PendingClaim,
) -> Result<ClaimOutcome, TwitchError> {
    if pending_claim.user_id != client.user_id {
        return Err(TwitchError::Unauthorized);
    }
    let pending = journal.clone();
    let user_id = client.user_id;
    let pending_claim =
        tokio::task::spawn_blocking(move || pending.blocking_lock().prepare(pending_claim))
            .await
            .map_err(|_| TwitchError::Storage)?
            .map_err(|_| TwitchError::Storage)?;
    if pending_claim.settled {
        return Ok(ClaimOutcome::NotClaimed);
    }
    // A settings save can overtake a queued job or its durable intent write.
    if !pending_claim.confirmed && !app.settings.read().await.auto_claim_drops {
        return Ok(ClaimOutcome::Disabled);
    }
    let claimed = pending_claim.confirmed || client.claim(&pending_claim.instance).await?;
    let app = app.clone();
    let journal = journal.clone();
    tokio::task::spawn_blocking(move || {
        if claimed {
            // Keep the receipt until the owner applies the result or reconciles
            // it after restart. A preclaim publication cannot destroy its evidence.
            journal
                .blocking_lock()
                .confirm(user_id, &pending_claim.entry.id)?;
            app.import_history(vec![pending_claim.entry.clone()])?;
        } else {
            journal
                .blocking_lock()
                .finish(user_id, &pending_claim.entry.id)?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|_| TwitchError::Storage)?
    .map_err(|_| TwitchError::Storage)?;
    Ok(if claimed {
        ClaimOutcome::Claimed
    } else {
        ClaimOutcome::NotClaimed
    })
}

pub(super) fn record_claim(app: &App, claim: &PendingClaim) -> anyhow::Result<()> {
    app.import_history(vec![claim.entry.clone()])?;
    if let Some(completed) = &claim.completed_campaign {
        app.archive
            .blocking_lock()
            .update(&[completed.clone().into()])?;
    }
    Ok(())
}
