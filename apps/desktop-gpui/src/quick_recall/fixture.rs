//! A [`Backend`] over the P0 parity map (`apps/desktop/parity/fixtures/
//! quick-recall/<state>.json`, command → JSON): what the Svelte stub serves, so
//! goldens and timing tests show the same moments. Records every call.

use std::path::PathBuf;
use std::sync::Mutex;

use app_infra::{MatchBox, RecentQuery, SearchFacet, SearchFacetCounts};
use capture_types::{RecordingSettings, TurnSnapshot};
use serde::Deserialize;
use serde_json::Value;

use super::data::{dedupe_names, Availability, Backend, Res};
use super::model::{IndexStatus, Preview, SearchResponse, Turn};
use crate::shell::fixture::{get, load};

/// The golden / parity states, in `surfaces.ts` order.
pub const STATES: [&str; 7] = [
    "idle",
    "results",
    "quicklook",
    "ask-streaming",
    "ask-done",
    "recent",
    "filter",
];

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Search(String),
    Previews(Vec<i64>),
    Facet(String),
    AskStart(String),
    AskCancel(String, bool),
    SetRecent(usize),
}

#[derive(Default)]
pub struct FixtureBackend {
    map: Value,
    pub calls: Mutex<Vec<Call>>,
    /// Make `search_capture` fail.
    pub fail_search: Mutex<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRow {
    frame_id: i64,
    preview: Option<PreviewFile>,
    missing_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewFile {
    file_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnRow {
    start_ms: u64,
    end_ms: u64,
    speaker_label: String,
    transcript_text: Option<String>,
}

impl FixtureBackend {
    pub fn load(state: &str) -> anyhow::Result<Self> {
        Ok(Self {
            map: load("quick-recall", state)?,
            ..Default::default()
        })
    }

    /// The query the state types in (`take_quick_recall_query`).
    pub fn summon_query(&self) -> Option<String> {
        self.get(&["take_quick_recall_query"])
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &[&str]) -> Option<T> {
        get(&self.map, path).ok().flatten()
    }

    fn record(&self, c: Call) {
        self.calls.lock().unwrap().push(c);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl Backend for FixtureBackend {
    fn search(&self, query: &str, _: u32, _: u32) -> Res<SearchResponse> {
        self.record(Call::Search(query.into()));
        if *self.fail_search.lock().unwrap() {
            return Err("failed to search captured content: fixture".into());
        }
        Ok(self.get(&["search_capture"]).unwrap_or_default())
    }

    fn previews(&self, ids: &[i64], _: Option<u32>) -> Vec<Preview> {
        self.record(Call::Previews(ids.to_vec()));
        let rows: Vec<PreviewRow> = self
            .get(&["get_frame_scrub_previews", "previews"])
            .unwrap_or_default();
        // One picture per id: the fixture lists a few; every other id reuses the first.
        ids.iter()
            .filter_map(|id| {
                let row = rows.iter().find(|r| r.frame_id == *id).or(rows.first())?;
                Some(Preview {
                    frame_id: *id,
                    path: row.preview.as_ref().map(|p| PathBuf::from(&p.file_path)),
                    missing: row.missing_reason.clone(),
                })
            })
            .collect()
    }

    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Vec<(i64, i64)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Row {
            id: i64,
            captured_at: String,
        }
        let rows: Vec<Row> = self
            .get(&["list_frame_summaries_in_range"])
            .unwrap_or_default();
        rows.into_iter()
            .map(|r| (r.id, super::model::ms(&r.captured_at)))
            .filter(|r| r.1 >= start_ms && r.1 <= end_ms)
            .collect()
    }

    fn ocr_text(&self, _: i64) -> Option<String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Row {
            processor: String,
            result_text: Option<String>,
        }
        let rows: Vec<Row> = self.get(&["list_processing_results"]).unwrap_or_default();
        rows.into_iter()
            .find(|r| r.processor == "ocr")
            .and_then(|r| r.result_text)
    }

    fn match_boxes(&self, _: i64, _: &str, _: bool) -> Vec<MatchBox> {
        self.get(&["get_frame_match_boxes"]).unwrap_or_default()
    }

    fn turns(&self, _: i64) -> Vec<Turn> {
        let rows: Vec<TurnRow> = self.get(&["list_speaker_turns"]).unwrap_or_default();
        rows.into_iter()
            .filter_map(|t| {
                let text = t.transcript_text?.trim().to_string();
                (!text.is_empty()).then_some(Turn {
                    start_ms: t.start_ms,
                    end_ms: t.end_ms,
                    speaker: t.speaker_label,
                    text,
                })
            })
            .collect()
    }

    fn apps(&self) -> Option<Vec<String>> {
        let apps: Vec<app_infra::SearchableApp> =
            self.get(&["list_searchable_apps"]).unwrap_or_default();
        Some(dedupe_names(apps.into_iter().filter_map(|a| a.name)))
    }

    fn app_icons(&self, _: &[String]) -> Vec<(String, PathBuf)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Row {
            bundle_id: String,
            icon_path: Option<String>,
        }
        let rows: Vec<Row> = self.get(&["resolve_app_icons"]).unwrap_or_default();
        rows.into_iter()
            .filter_map(|r| Some((r.bundle_id, PathBuf::from(r.icon_path?))))
            .collect()
    }

    fn recent(&self) -> Option<Vec<RecentQuery>> {
        Some(
            self.get(&["get_quick_recall_recent_queries"])
                .unwrap_or_default(),
        )
    }

    fn set_recent(&self, queries: Vec<RecentQuery>) -> Option<Vec<RecentQuery>> {
        self.record(Call::SetRecent(queries.len()));
        Some(queries)
    }

    fn facet(&self, query: &str, facet: SearchFacet) -> Option<SearchFacetCounts> {
        self.record(Call::Facet(query.into()));
        // One counts object (what the Svelte stub answers every call) or a list of them.
        let all: Vec<SearchFacetCounts> = self
            .get(&["count_search_facet"])
            .or_else(|| self.get(&["count_search_facet"]).map(|c| vec![c]))
            .unwrap_or_default();
        all.into_iter().find(|c| c.facet == facet)
    }

    fn index_status(&self) -> Option<IndexStatus> {
        self.get(&["get_quick_recall_index_status"])
    }

    fn recording(&self) -> Option<bool> {
        self.get::<Value>(&["get_capture_permissions", "session", "isRunning"])
            .and_then(|v| v.as_bool())
    }

    fn availability(&self) -> Availability {
        self.get(&["ask_ai_availability"]).unwrap_or_default()
    }

    fn settings(&self) -> Option<RecordingSettings> {
        self.get(&["get_recording_settings"])
    }

    fn ask_start(&self, id: &str, _: &str, _: &str) -> Res<()> {
        self.record(Call::AskStart(id.into()));
        Ok(())
    }

    fn ask_cancel(&self, id: &str, discard: bool) {
        self.record(Call::AskCancel(id.into(), discard));
    }

    fn ask_snapshot(&self, _: &str) -> Option<TurnSnapshot> {
        self.get(&["ask_ai_snapshot"])
    }
}
