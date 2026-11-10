//! Where Quick Recall's data comes from. Views never call the backend: the panel
//! runs these on the background executor. `CoreBackend` asks `AppCore` (every
//! command Quick Recall uses is in app-core now); `fixture::FixtureBackend`
//! serves the parity JSON for goldens and timing tests.

use std::path::PathBuf;
use std::sync::Arc;

use app_core::AppCore;
use app_infra::{MatchBox, RecentQuery, SearchFacet, SearchFacetCounts};
use capture_types::{RecordingSettings, TurnSnapshot};
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::model::{IndexStatus, Preview, SearchResponse, Turn};

pub type Res<T> = Result<T, String>;

/// `ask_ai_availability`.
pub use crate::chat::model::Availability;

/// Synchronous (the panel calls them off the main thread). Every method has a
/// "nothing" default, so a backend only answers what it has.
pub trait Backend: Send + Sync + 'static {
    /// `search_capture` with per-kind limits (operators live in the query text).
    fn search(&self, query: &str, frame_limit: u32, audio_limit: u32) -> Res<SearchResponse>;
    /// `get_frame_scrub_previews` (`max_px` = Quick Look's 1280 stage).
    fn previews(&self, _ids: &[i64], _max_px: Option<u32>) -> Vec<Preview> {
        Vec::new()
    }
    /// `list_frame_summaries_in_range`: `(id, captured ms)`.
    fn frame_times(&self, _start_ms: i64, _end_ms: i64) -> Vec<(i64, i64)> {
        Vec::new()
    }
    /// The newest OCR result's text for a frame.
    fn ocr_text(&self, _frame_id: i64) -> Option<String> {
        None
    }
    fn match_boxes(&self, _frame_id: i64, _query: &str, _by_meaning: bool) -> Vec<MatchBox> {
        Vec::new()
    }
    /// `list_speaker_turns`, empty turns dropped.
    fn turns(&self, _audio_segment_id: i64) -> Vec<Turn> {
        Vec::new()
    }
    /// `list_searchable_apps` names, most recent first (None = failed: retry later).
    fn apps(&self) -> Option<Vec<String>> {
        None
    }
    /// `resolve_app_icons`: id → icon file.
    fn app_icons(&self, _ids: &[String]) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    fn recent(&self) -> Option<Vec<RecentQuery>> {
        None
    }
    fn set_recent(&self, queries: Vec<RecentQuery>) -> Option<Vec<RecentQuery>> {
        Some(queries)
    }
    fn facet(&self, _query: &str, _facet: SearchFacet) -> Option<SearchFacetCounts> {
        None
    }
    fn index_status(&self) -> Option<IndexStatus> {
        None
    }
    fn recording(&self) -> Option<bool> {
        None
    }
    fn availability(&self) -> Availability {
        Availability::default()
    }
    /// `mcp_warm_connectors`.
    fn warm_mcp(&self) {}
    fn settings(&self) -> Option<RecordingSettings> {
        None
    }
    fn ask_start(&self, _id: &str, _question: &str, _title: &str) -> Res<()> {
        Err("ask_ai_unavailable".into())
    }
    fn ask_cancel(&self, _id: &str, _discard: bool) {}
    fn ask_snapshot(&self, _id: &str) -> Option<TurnSnapshot> {
        None
    }
    /// `open_captured_url`: Ok(false) = no openable page.
    fn open_url(&self, _frame_id: i64) -> Res<bool> {
        Ok(false)
    }
}

/// An app-core DTO as the view's serde model (one shape for core and fixture).
pub fn reshape<T: DeserializeOwned>(v: impl Serialize) -> Option<T> {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
}

fn iso(ms: i64) -> String {
    let t = time::OffsetDateTime::UNIX_EPOCH + time::Duration::milliseconds(ms);
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The real app.
pub struct CoreBackend(pub Arc<AppCore>);

impl CoreBackend {
    fn infra(&self) -> Option<Arc<app_core::AppInfraState>> {
        self.0.try_state::<app_core::AppInfraState>()
    }
}

impl Backend for CoreBackend {
    fn search(&self, query: &str, frame_limit: u32, audio_limit: u32) -> Res<SearchResponse> {
        use app_core::app_infra::{search_capture, SearchCaptureRequest};
        let infra = self.infra().ok_or("search isn't ready yet")?;
        let embedder = self
            .0
            .try_state::<app_core::semantic_search_query::SemanticQueryEmbedderState>()
            .ok_or("search isn't ready yet")?;
        let request = SearchCaptureRequest {
            query: query.into(),
            frame_limit: Some(frame_limit),
            frame_offset: Some(0),
            audio_limit: Some(audio_limit),
            audio_offset: Some(0),
            snapshot_document_id: None,
            refinements: Some(Default::default()),
        };
        let res = self
            .0
            .block_on(search_capture(request, &self.0, &infra, &embedder))?;
        reshape(res).ok_or_else(|| "unreadable search response".into())
    }

    fn previews(&self, ids: &[i64], max_px: Option<u32>) -> Vec<Preview> {
        use app_core::app_infra::frame_preview::{
            get_frame_scrub_previews, GetFrameScrubPreviewsRequest,
        };
        let request = GetFrameScrubPreviewsRequest {
            frame_ids: ids.to_vec(),
            max_pixel_size: max_px,
        };
        let Ok(res) = self.0.block_on(get_frame_scrub_previews(&self.0, request)) else {
            return Vec::new();
        };
        res.previews
            .into_iter()
            .map(|p| Preview {
                frame_id: p.frame_id,
                path: p.preview.map(|v| PathBuf::from(v.file_path)),
                missing: p
                    .missing_reason
                    .and_then(|r| serde_json::to_value(r).ok())
                    .and_then(|v| v.as_str().map(String::from)),
            })
            .collect()
    }

    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Vec<(i64, i64)> {
        use app_core::app_infra::{list_frame_summaries_in_range, FrameCapturedAtRangeRequest};
        let Some(infra) = self.infra() else {
            return Vec::new();
        };
        let request = FrameCapturedAtRangeRequest {
            captured_at_start: iso(start_ms),
            captured_at_end: iso(end_ms),
            exclude_lock_screen: false,
        };
        self.0
            .block_on(list_frame_summaries_in_range(request, &infra))
            .unwrap_or_default()
            .into_iter()
            .map(|f| (f.id, super::model::ms(&f.captured_at)))
            .filter(|f| f.1 > 0)
            .collect()
    }

    fn ocr_text(&self, frame_id: i64) -> Option<String> {
        use app_core::app_infra::{list_processing_results, ListProcessingResultsRequest};
        let infra = self.infra()?;
        let request = ListProcessingResultsRequest {
            subject_type: "frame".into(),
            subject_id: frame_id,
        };
        let mut rows = self
            .0
            .block_on(list_processing_results(request, &infra))
            .ok()?;
        rows.retain(|r| r.processor == "ocr" && r.result_text.is_some());
        rows.sort_by_key(|r| std::cmp::Reverse(r.id));
        let text = rows.first()?.result_text.as_deref()?.trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn match_boxes(&self, frame_id: i64, query: &str, by_meaning: bool) -> Vec<MatchBox> {
        use app_core::app_infra::quick_recall::{get_frame_match_boxes, GetFrameMatchBoxesRequest};
        let Some(infra) = self.infra() else {
            return Vec::new();
        };
        let request = GetFrameMatchBoxesRequest {
            frame_id,
            query: query.into(),
            by_meaning,
        };
        self.0
            .block_on(get_frame_match_boxes(request, &infra))
            .unwrap_or_default()
    }

    fn turns(&self, audio_segment_id: i64) -> Vec<Turn> {
        use app_core::app_infra::{list_speaker_turns, ListSpeakerTurnsRequest};
        let Some(infra) = self.infra() else {
            return Vec::new();
        };
        let request = ListSpeakerTurnsRequest { audio_segment_id };
        self.0
            .block_on(list_speaker_turns(request, &infra))
            .unwrap_or_default()
            .into_iter()
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
        let infra = self.infra()?;
        let apps = self
            .0
            .block_on(app_core::app_infra::list_searchable_apps(&infra))
            .ok()?;
        Some(dedupe_names(apps.into_iter().filter_map(|a| a.name)))
    }

    fn app_icons(&self, ids: &[String]) -> Vec<(String, PathBuf)> {
        use app_core::native_capture::app_candidates::{resolve_app_icons, ResolveAppIconsRequest};
        let request = ResolveAppIconsRequest {
            bundle_ids: ids.to_vec(),
        };
        self.0
            .block_on(resolve_app_icons(&self.0, request))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| Some((r.bundle_id, PathBuf::from(r.icon_path?))))
            .collect()
    }

    fn recent(&self) -> Option<Vec<RecentQuery>> {
        let infra = self.infra()?;
        self.0
            .block_on(app_core::app_infra::quick_recall::get_quick_recall_recent_queries(&infra))
            .ok()
    }

    fn set_recent(&self, queries: Vec<RecentQuery>) -> Option<Vec<RecentQuery>> {
        let infra = self.infra()?;
        self.0
            .block_on(
                app_core::app_infra::quick_recall::set_quick_recall_recent_queries(queries, &infra),
            )
            .ok()
    }

    fn facet(&self, query: &str, facet: SearchFacet) -> Option<SearchFacetCounts> {
        use app_core::app_infra::quick_recall::{count_search_facet, CountSearchFacetRequest};
        let infra = self.infra()?;
        let embedder = self
            .0
            .try_state::<app_core::semantic_search_query::SemanticQueryEmbedderState>()?;
        let request = CountSearchFacetRequest {
            query: query.into(),
            refinements: None,
            facet,
        };
        self.0
            .block_on(count_search_facet(request, &self.0, &infra, &embedder))
            .ok()
    }

    fn index_status(&self) -> Option<IndexStatus> {
        let infra = self.infra()?;
        let s = self
            .0
            .block_on(
                app_core::app_infra::quick_recall::get_quick_recall_index_status(&self.0, &infra),
            )
            .ok()?;
        reshape(s)
    }

    fn recording(&self) -> Option<bool> {
        self.0
            .try_state::<app_core::native_capture::lifecycle::NativeCaptureState>()?;
        Some(app_core::native_capture::control::current_native_capture_session(&self.0).is_running)
    }

    fn availability(&self) -> Availability {
        match self
            .0
            .block_on(app_core::ask_ai::ask_ai_availability(&self.0))
        {
            Ok(a) => reshape(a).unwrap_or_default(),
            Err(e) => Availability {
                available: false,
                reason: Some(e),
            },
        }
    }

    fn warm_mcp(&self) {
        crate::chat::data::warm_mcp(&self.0)
    }

    fn settings(&self) -> Option<RecordingSettings> {
        let s = self
            .0
            .try_state::<app_core::recording_settings::RecordingSettingsState>()?;
        Some(app_core::recording_settings::current_recording_settings(&s))
    }

    fn ask_start(&self, id: &str, question: &str, title: &str) -> Res<()> {
        let offset = crate::native::sys::local_offset().whole_minutes() as i32;
        let request: app_core::ask_ai::AskAiStartRequest =
            serde_json::from_value(serde_json::json!({
                "conversationId": id,
                "question": question,
                "origin": "quick_recall",
                "title": title,
                "utcOffsetMinutes": offset,
            }))
            .map_err(|e| e.to_string())?;
        self.0
            .block_on(app_core::ask_ai::ask_ai_start(&self.0, request))
    }

    fn ask_cancel(&self, id: &str, discard: bool) {
        let request = app_core::ask_ai::AskAiCancelRequest {
            conversation_id: id.into(),
            discard,
        };
        let _ = self.0.block_on(app_core::ask_ai::ask_ai_cancel(request));
    }

    fn ask_snapshot(&self, id: &str) -> Option<TurnSnapshot> {
        let request = app_core::ask_ai::AskAiSnapshotRequest {
            conversation_id: id.into(),
        };
        self.0
            .block_on(app_core::ask_ai::ask_ai_snapshot(request))
            .ok()
            .flatten()
    }

    fn open_url(&self, frame_id: i64) -> Res<bool> {
        let infra = self.infra().ok_or("not ready")?;
        self.0.block_on(app_core::app_infra::open_captured_url(
            frame_id, &self.0, &infra,
        ))
    }
}

/// Deduped by name (case-insensitive), first seen first.
pub fn dedupe_names(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && seen.insert(n.to_lowercase()))
        .collect()
}
