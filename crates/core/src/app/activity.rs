use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{config::Language, dto::Snapshot};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Account,
    #[default]
    Mining,
    Inventory,
    Claims,
    Connection,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ActivityEvent {
    pub id: u64,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub category: Category,
    pub severity: Severity,
    pub code: String,
    pub args: BTreeMap<String, String>,
    pub message: String,
    pub campaign_id: Option<String>,
    pub drop_id: Option<String>,
    pub channel_id: Option<u64>,
    pub count: u32,
    pub recovered: bool,
}

impl ActivityEvent {
    pub fn new(code: &str, category: Category, severity: Severity, args: &[(&str, &str)]) -> Self {
        Self::new_in(Language::En, code, category, severity, args)
    }

    pub fn new_in(
        language: Language,
        code: &str,
        category: Category,
        severity: Severity,
        args: &[(&str, &str)],
    ) -> Self {
        let now = Utc::now();
        Self {
            id: 0,
            first_at: now,
            last_at: now,
            category,
            severity,
            code: code.to_owned(),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            message: super::message_in(language, code, args),
            campaign_id: None,
            drop_id: None,
            channel_id: None,
            count: 1,
            recovered: false,
        }
    }

    pub fn message(message: String) -> Self {
        Self {
            message,
            ..Self::new("status.message", Category::Mining, Severity::Info, &[])
        }
    }
}

// Returns whether this is a new row, so server logs can suppress adjacent repeats too.
pub fn record(state: &mut Snapshot, mut event: ActivityEvent) -> bool {
    if let Some(previous) = state.activity.last_mut()
        && !previous.recovered
        && previous.category == event.category
        && previous.severity == event.severity
        && previous.code == event.code
        && previous.args == event.args
        && previous.message == event.message
        && previous.campaign_id == event.campaign_id
        && previous.drop_id == event.drop_id
        && previous.channel_id == event.channel_id
    {
        previous.count = previous.count.saturating_add(1);
        previous.last_at = event.last_at;
        return false;
    }
    event.id = state.activity.last().map_or(1, |last| last.id + 1);
    state.console.push(format!(
        "[{}] | {}",
        event.last_at.format("%Y-%m-%d %H:%M:%S"),
        event.message
    ));
    state.activity.push(event);
    if state.activity.len() > 1000 {
        state.activity.remove(0);
    }
    if state.console.len() > 1000 {
        state.console.remove(0);
    }
    true
}

pub fn recover(state: &mut Snapshot, scope: &ActivityEvent) {
    for previous in &mut state.activity {
        if previous.severity != Severity::Info
            && previous.category == scope.category
            && previous.campaign_id == scope.campaign_id
            && previous.drop_id == scope.drop_id
            && previous.channel_id == scope.channel_id
            && previous.args.get("operation") == scope.args.get("operation")
        {
            previous.recovered = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_failures_are_counted_and_buffer_is_bounded() {
        let mut state = Snapshot::default();
        let mut failure = ActivityEvent::new(
            "gui.backend.twitch_error",
            Category::Connection,
            Severity::Warning,
            &[],
        );
        failure.channel_id = Some(7);
        assert!(record(&mut state, failure.clone()));
        assert!(!record(&mut state, failure));
        assert_eq!(state.activity.len(), 1);
        assert_eq!(state.activity[0].count, 2);
        for i in 0..1001 {
            assert!(record(&mut state, ActivityEvent::message(i.to_string())));
        }
        assert_eq!(state.activity.len(), 1000);
        assert_eq!(state.console.len(), 1000);
    }
    #[test]
    fn recovery_requires_the_same_operation_and_entity() {
        let mut state = Snapshot::default();
        let mut failure = ActivityEvent::new(
            "gui.backend.twitch_error",
            Category::Mining,
            Severity::Warning,
            &[("operation", "watch")],
        );
        failure.channel_id = Some(7);
        record(&mut state, failure.clone());
        record(
            &mut state,
            ActivityEvent::message("Watching another channel".into()),
        );
        assert!(!state.activity[0].recovered);
        let mut unrelated = failure.clone();
        unrelated.channel_id = Some(8);
        recover(&mut state, &unrelated);
        unrelated.channel_id = Some(7);
        unrelated.args.insert("operation".into(), "progress".into());
        recover(&mut state, &unrelated);
        assert!(!state.activity[0].recovered);
        recover(&mut state, &failure);
        assert!(state.activity[0].recovered);
        record(&mut state, failure);
        assert!(!state.activity.last().unwrap().recovered);
    }
}
