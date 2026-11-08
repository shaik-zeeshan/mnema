//! Ask AI's tests, grouped by what they pin. Shared fixtures live here.

use super::*;
use app_infra::brokered_access::{
    BrokerAuthStatusKind, BrokerErrorResponse, BrokerSearchResponse, BrokerSearchResultContext,
    BrokerShowTextResponse,
};

mod broker;
mod host_events;
mod live_view;
mod requests;
mod sources;
mod turn_and_tools;

fn sample_result() -> BrokerSearchResult {
    BrokerSearchResult {
        opaque_id: "op-1".to_string(),
        kind: "frame".to_string(),
        snippet: "build passed".to_string(),
        started_at: "2026-01-01T10:00:00Z".to_string(),
        ended_at: "2026-01-01T10:01:00Z".to_string(),
        context: Some(BrokerSearchResultContext {
            app_bundle_id: Some("com.apple.dt.Xcode".to_string()),
            app_name: Some("Xcode".to_string()),
            window_title: Some("ContentView.swift".to_string()),
            url: None,
        }),
        span_start_ms: None,
        span_end_ms: None,
        aligned_frame_id: None,
        turns: Vec::new(),
    }
}

fn frame_source(started_at: &str, ended_at: &str) -> ResolvedAskAiSource {
    ResolvedAskAiSource {
        kind: "frame".to_string(),
        frame_id: Some(42),
        audio_segment_id: None,
        app_name: Some("Xcode".to_string()),
        window_title: Some("ContentView.swift".to_string()),
        url: Some("github.com/owner/repo".to_string()),
        snippet: Some("the <mark>pricing</mark> page".to_string()),
        started_at: started_at.to_string(),
        ended_at: ended_at.to_string(),
        span_start_ms: None,
        aligned_frame_id: None,
        source_kind: None,
    }
}

fn audio_source(started_at: &str, ended_at: &str) -> ResolvedAskAiSource {
    ResolvedAskAiSource {
        kind: "audio".to_string(),
        frame_id: None,
        audio_segment_id: Some(7),
        app_name: Some("Zoom".to_string()),
        window_title: None,
        url: None,
        snippet: None,
        started_at: started_at.to_string(),
        ended_at: ended_at.to_string(),
        span_start_ms: Some(3_000),
        aligned_frame_id: Some(99),
        source_kind: Some("microphone"),
    }
}

/// A fresh thinking-phase view for a turn, matching the shape `run_ask_ai_turn`
/// registers at step 4.
fn fresh_view(turn_index: i64) -> TurnView {
    TurnView {
        turn_index,
        question: "q".to_string(),
        phase: "thinking".to_string(),
        blocks: vec![],
        reasoning: None,
        tool_activities: vec![],
        live_activity: None,
        sources: serde_json::json!([]),
        error_message: None,
        provider: None,
        model: None,
        context_tokens: None,
    }
}
