//! Setup's download fold (`lib/onboarding/model-readiness.ts` + the sequential
//! driver in `SetupScreen.svelte`). One download at a time, in the work list's
//! order; bytes are monotonic; a failure stays at its item with the real reason.
//! Nothing here ever gates Continue.

use std::collections::HashMap;

use serde_json::Value;

use super::flow::{Features, WorkItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DlState {
    Queued,
    Downloading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct Downloads {
    pub states: HashMap<String, DlState>,
    pub received: HashMap<String, u64>,
    pub errors: HashMap<String, String>,
    /// Items started once; a retry / "turn it back on" removes the id.
    pub attempted: Vec<String>,
    /// The one item whose start has been issued and not yet settled.
    pub in_flight: Option<String>,
    /// The user chose to download anyway after a measured shortfall.
    pub disk_override: bool,
}

pub fn id(item: &WorkItem) -> String {
    format!("{:?}:{}:{}", item.subsystem, item.provider, item.model_id)
}

impl Downloads {
    pub fn state(&self, item: &WorkItem) -> DlState {
        self.states
            .get(&id(item))
            .copied()
            .unwrap_or(DlState::Queued)
    }

    pub fn received(&self, item: &WorkItem) -> u64 {
        self.received.get(&id(item)).copied().unwrap_or(0)
    }

    /// Fold one `*_model_download_progress` event (any of the four families).
    /// Returns the id it touched, if it was one of ours.
    pub fn apply(&mut self, event: &Value, items: &[WorkItem]) -> Option<String> {
        let provider = event["provider"].as_str()?;
        let model = event["modelId"].as_str()?;
        let item = items
            .iter()
            .find(|i| i.provider == provider && i.model_id == model)?;
        let key = id(item);
        let got = event["downloadedBytes"].as_u64().unwrap_or(0);
        let r = self.received.entry(key.clone()).or_default();
        *r = (*r).max(got);
        let state = match event["status"].as_str().unwrap_or("") {
            "completed" => DlState::Ready,
            "failed" => DlState::Failed,
            "cancelled" => DlState::Queued,
            _ => DlState::Downloading,
        };
        if state == DlState::Ready {
            *r = (*r).max(item.bytes);
        }
        if state == DlState::Failed {
            let msg = event["message"]
                .as_str()
                .unwrap_or("Download failed.")
                .to_string();
            self.errors.insert(key.clone(), msg);
        }
        self.states.insert(key.clone(), state);
        if state != DlState::Downloading && self.in_flight.as_deref() == Some(key.as_str()) {
            self.in_flight = None;
        }
        Some(key)
    }

    /// A start that failed synchronously never reaches the progress stream.
    pub fn failed_to_start(&mut self, item: &WorkItem, message: String) {
        let key = id(item);
        self.states.insert(key.clone(), DlState::Failed);
        self.errors.insert(key.clone(), message);
        if self.in_flight.as_deref() == Some(key.as_str()) {
            self.in_flight = None;
        }
    }

    /// The sequential driver: the next item to start, if any (marks it in flight).
    pub fn next_to_start(
        &mut self,
        items: &[WorkItem],
        f: &Features,
        may_start: bool,
    ) -> Option<WorkItem> {
        if !may_start {
            return None;
        }
        let active: Vec<&WorkItem> = items.iter().filter(|i| f.get(i.feature)).collect();
        if let Some(busy) = &self.in_flight {
            if active.iter().any(|i| &id(i) == busy) {
                return None;
            }
        }
        let next = active
            .into_iter()
            .find(|i| self.state(i) != DlState::Ready && !self.attempted.contains(&id(i)))?
            .clone();
        self.attempted.push(id(&next));
        self.in_flight = Some(id(&next));
        self.states.insert(id(&next), DlState::Downloading);
        Some(next)
    }

    pub fn retry(&mut self, item: &WorkItem) {
        let key = id(item);
        self.attempted.retain(|a| a != &key);
        self.errors.remove(&key);
        self.states.insert(key, DlState::Queued);
    }

    /// Byte-weighted over the active subset, so a cancel shrinks the total.
    pub fn percent(&self, items: &[WorkItem], f: &Features) -> u32 {
        let active: Vec<&WorkItem> = items.iter().filter(|i| f.get(i.feature)).collect();
        let total: u64 = active.iter().map(|i| i.bytes).sum();
        if total == 0 {
            return 100;
        }
        let got: u64 = active.iter().map(|i| self.received(i).min(i.bytes)).sum();
        ((got as f64 / total as f64) * 100.).floor().min(100.) as u32
    }

    pub fn all_ready(&self, items: &[WorkItem], f: &Features) -> bool {
        items
            .iter()
            .filter(|i| f.get(i.feature))
            .all(|i| self.state(i) == DlState::Ready)
    }
}
