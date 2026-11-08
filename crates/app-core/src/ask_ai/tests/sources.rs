use super::*;

#[test]
fn ask_ai_source_kind_splits_microphone_from_system_audio() {
    // A system-audio citation is playback, not the user speaking — swapping
    // these two would have the answer card attribute a podcast to the user.
    assert_eq!(ask_ai_source_kind("audio_microphone"), Some("microphone"));
    assert_eq!(ask_ai_source_kind("audio_system"), Some("system"));
    assert_eq!(ask_ai_source_kind("frame"), None);
}

/// "When was Priya talking" is answerable from `timeline` and nowhere else —
/// the tool description now says exactly that — and timeline intervals carry
/// followable opaque ids. Retaining ONLY `search` results leaves every id from
/// that answer unciteable, so the turn renders with no source cards at all.
#[test]
fn timeline_intervals_are_retained_so_the_model_can_cite_them() {
    let response = BrokeredCaptureResponse::Timeline(
        app_infra::brokered_access::BrokerTimelineResponse::page(
            vec![
                app_infra::brokered_access::BrokerTimelineInterval {
                    kind: "audio_microphone".to_string(),
                    started_at: "2026-06-12T09:00:00Z".to_string(),
                    ended_at: Some("2026-06-12T09:05:00Z".to_string()),
                    opaque_id: Some("a2a:gask-ai.deadbeef".to_string()),
                    context: None,
                    turns: Vec::new(),
                },
                // No representative capture to point at: nothing to cite, so
                // nothing may be retained under an empty key either.
                app_infra::brokered_access::BrokerTimelineInterval {
                    kind: "frame".to_string(),
                    started_at: "2026-06-12T10:00:00Z".to_string(),
                    ended_at: Some("2026-06-12T10:05:00Z".to_string()),
                    opaque_id: None,
                    context: None,
                    turns: Vec::new(),
                },
            ],
            8,
            None,
        ),
    );

    let mut metadata: HashMap<String, BrokerSearchResult> = HashMap::new();
    retain_source_metadata(&response, &mut metadata);

    let retained = metadata
        .get("a2a:gask-ai.deadbeef")
        .expect("a timeline interval the model can follow must be citable");
    assert_eq!(retained.kind, "audio_microphone");
    assert_eq!(retained.started_at, "2026-06-12T09:00:00Z");
    assert_eq!(retained.ended_at, "2026-06-12T09:05:00Z");
    assert_eq!(
        metadata.len(),
        1,
        "an interval with no opaque id is not citable"
    );

    // …and the card it produces is still honest about mic-vs-system.
    let (sources, accepted, dropped) =
        build_ask_ai_sources(&["a2a:gask-ai.deadbeef".to_string()], |id| {
            let result = metadata.get(id)?;
            Some(ResolvedAskAiSource {
                kind: "audio".to_string(),
                frame_id: None,
                audio_segment_id: Some(42),
                app_name: None,
                window_title: None,
                url: None,
                snippet: None,
                started_at: result.started_at.clone(),
                ended_at: result.ended_at.clone(),
                span_start_ms: result.span_start_ms,
                aligned_frame_id: result.aligned_frame_id,
                source_kind: ask_ai_source_kind(&result.kind),
            })
        });
    assert_eq!((accepted, dropped), (1, 0));
    assert_eq!(sources[0]["sourceKind"], serde_json::json!("microphone"));
}

/// A `search` result is authoritative: it alone carries the Audio Search
/// Result Anchor, so a later timeline interval for the same recording must not
/// flatten it back to the segment start.
#[test]
fn a_timeline_interval_never_overwrites_a_search_result_anchor() {
    let mut result = sample_result();
    result.opaque_id = "a2a:gask-ai.deadbeef".to_string();
    result.kind = "audio_microphone".to_string();
    result.span_start_ms = Some(3_000);
    result.aligned_frame_id = Some(99);

    let mut metadata: HashMap<String, BrokerSearchResult> = HashMap::new();
    retain_source_metadata(
        &BrokeredCaptureResponse::Search(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![result],
            limit: 8,
            scope_clamped: false,
            required_scope: None,
            next_cursor: None,
            speaker_coverage: None,
        }),
        &mut metadata,
    );
    retain_source_metadata(
        &BrokeredCaptureResponse::Timeline(
            app_infra::brokered_access::BrokerTimelineResponse::page(
                vec![app_infra::brokered_access::BrokerTimelineInterval {
                    kind: "audio_microphone".to_string(),
                    started_at: "2026-06-12T09:00:00Z".to_string(),
                    ended_at: Some("2026-06-12T09:05:00Z".to_string()),
                    opaque_id: Some("a2a:gask-ai.deadbeef".to_string()),
                    context: None,
                    turns: Vec::new(),
                }],
                8,
                None,
            ),
        ),
        &mut metadata,
    );

    let retained = metadata.get("a2a:gask-ai.deadbeef").expect("retained");
    assert_eq!(retained.span_start_ms, Some(3_000));
    assert_eq!(retained.aligned_frame_id, Some(99));
}

#[test]
fn build_ask_ai_sources_caps_frames_and_audio() {
    let mut ids: Vec<String> = (0..8).map(|i| format!("frame-{i}")).collect();
    ids.extend((0..6).map(|i| format!("audio-{i}")));

    let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |id| {
        if id.starts_with("frame-") {
            Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
        } else {
            Some(audio_source("2026-01-01T11:00:00Z", "2026-01-01T11:05:00Z"))
        }
    });

    assert_eq!(accepted, 10);
    assert_eq!(dropped, ids.len() - 10);
    let frame_total = sources
        .iter()
        .filter(|s| s["kind"] == serde_json::json!("frame"))
        .count();
    let audio_total = sources
        .iter()
        .filter(|s| s["kind"] == serde_json::json!("audio"))
        .count();
    assert_eq!(frame_total, 6);
    assert_eq!(audio_total, 4);
    assert_eq!(sources[0]["kind"], serde_json::json!("frame"));
    assert_eq!(sources[6]["kind"], serde_json::json!("audio"));
}

#[test]
fn build_ask_ai_sources_drops_invalid_ids() {
    let ids = vec![
        "good-1".to_string(),
        "bad-1".to_string(),
        "good-2".to_string(),
        "bad-2".to_string(),
    ];

    let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |id| {
        if id.starts_with("good-") {
            Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
        } else {
            None
        }
    });

    assert_eq!(accepted, 2);
    assert_eq!(dropped, 2);
    assert_eq!(sources.len(), 2);
}

#[test]
fn build_ask_ai_sources_dedupes() {
    let ids = vec![
        "dup".to_string(),
        "dup".to_string(),
        "other".to_string(),
        "dup".to_string(),
    ];

    let (sources, accepted, dropped) = build_ask_ai_sources(&ids, |_id| {
        Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
    });

    assert_eq!(accepted, 2);
    assert_eq!(dropped, 2);
    assert_eq!(sources.len(), 2);
}

#[test]
fn build_ask_ai_sources_source_shape() {
    let ids = vec!["frame-1".to_string(), "audio-1".to_string()];
    let (sources, _accepted, _dropped) = build_ask_ai_sources(&ids, |id| {
        if id.starts_with("frame-") {
            Some(frame_source("2026-01-01T10:00:00Z", "2026-01-01T10:01:00Z"))
        } else {
            Some(audio_source("2026-01-01T11:00:00Z", "2026-01-01T11:05:00Z"))
        }
    });

    let frame = &sources[0];
    assert_eq!(frame["kind"], serde_json::json!("frame"));
    assert_eq!(frame["frameId"], serde_json::json!(42));
    assert_eq!(frame["audioSegmentId"], serde_json::Value::Null);
    assert_eq!(frame["appName"], serde_json::json!("Xcode"));
    assert_eq!(frame["windowTitle"], serde_json::json!("ContentView.swift"));
    assert_eq!(
        frame["startedAt"],
        serde_json::json!("2026-01-01T10:00:00Z")
    );
    assert_eq!(frame["endedAt"], serde_json::json!("2026-01-01T10:01:00Z"));
    assert!(frame.as_object().unwrap().contains_key("sourceKind"));
    assert_eq!(frame["sourceKind"], serde_json::Value::Null);
    assert_eq!(frame["spanStartMs"], serde_json::Value::Null);
    assert_eq!(frame["alignedFrameId"], serde_json::Value::Null);
    // Frame sources carry the guarded host+path; audio sources null it.
    assert_eq!(frame["url"], serde_json::json!("github.com/owner/repo"));
    // The text match rides along for the evidence row's second line.
    assert_eq!(
        frame["snippet"],
        serde_json::json!("the <mark>pricing</mark> page")
    );

    let audio = &sources[1];
    assert_eq!(audio["kind"], serde_json::json!("audio"));
    assert_eq!(audio["frameId"], serde_json::Value::Null);
    assert_eq!(audio["audioSegmentId"], serde_json::json!(7));
    assert_eq!(audio["windowTitle"], serde_json::Value::Null);
    assert_eq!(audio["url"], serde_json::Value::Null);
    assert_eq!(audio["snippet"], serde_json::Value::Null);
    // Mic-vs-system now rides the search result's kind, not a DB post-pass.
    assert_eq!(audio["sourceKind"], serde_json::json!("microphone"));
    assert_eq!(audio["spanStartMs"], serde_json::json!(3_000));
    assert_eq!(audio["alignedFrameId"], serde_json::json!(99));
}

#[test]
fn broker_response_to_tool_value_serializes_search() {
    let response = BrokeredCaptureResponse::Search(BrokerSearchResponse {
        results: vec![sample_result()],
        limit: 8,
        scope_clamped: false,
        required_scope: None,
        next_cursor: None,
        speaker_coverage: None,
    });

    let value = broker_response_to_tool_value(response).expect("search serializes");
    assert_eq!(value["limit"], serde_json::json!(8));
    assert_eq!(value["results"][0]["opaqueId"], serde_json::json!("op-1"));
}

#[test]
fn search_tool_schema_lets_the_model_use_the_cursor_it_is_handed() {
    // The whole `BrokerSearchResponse` is serialized straight to the model, so
    // once the broker started minting a real `nextCursor` the Ask AI `search`
    // tool started ADVERTISING a next page. The schema is
    // `additionalProperties: false`, so unless it declares `cursor` the model
    // literally cannot ask for that page — it is told the walk is unfinished
    // and given no way to finish it. (The CLI's MCP schema in
    // `crates/cli/src/mcp.rs` was updated for exactly this; this door was not.)
    let response = BrokeredCaptureResponse::Search(BrokerSearchResponse {
        results: vec![sample_result()],
        limit: 8,
        scope_clamped: false,
        required_scope: None,
        next_cursor: Some("v1:42:1:0".to_string()),
        speaker_coverage: None,
    });
    let value = broker_response_to_tool_value(response).expect("search serializes");
    assert_eq!(value["nextCursor"], serde_json::json!("v1:42:1:0"));

    let schema = search_tool_schema();
    assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    assert!(
        schema["properties"].get("cursor").is_some(),
        "search tool schema must expose `cursor` so the model can page: {schema}"
    );
}

/// A speaker filter can only match through DETECTED speaker turns, so an empty
/// filtered result is routinely "this audio could not be checked", not "they
/// said nothing". `speakerCoverage` is the only thing that tells the two apart,
/// and the whole response is serialized straight to the model — so the moment
/// the Ask AI door started offering `speaker`, it started handing the model two
/// bare counts with nothing anywhere that says what they mean. The CLI door
/// publishes the rule (`.agents/skills/mnema-data/SKILL.md`: "Either count being
/// non-zero makes the answer partial"); this door did not, and the answer the
/// user reads is the one that states silence as fact.
#[test]
fn speaker_filtered_tools_tell_the_model_an_empty_result_is_not_silence() {
    // The model really is handed the counts, with zero rows beside them.
    let response = BrokeredCaptureResponse::Search(BrokerSearchResponse {
        results: Vec::new(),
        limit: 8,
        scope_clamped: false,
        required_scope: None,
        next_cursor: None,
        speaker_coverage: Some(app_infra::brokered_access::BrokerSpeakerCoverage {
            recordings_with_unnamed_voices: 12,
            recordings_without_speaker_data: 30,
        }),
    });
    let value = broker_response_to_tool_value(response).expect("search serializes");
    assert_eq!(
        value["speakerCoverage"]["recordingsWithUnnamedVoices"],
        serde_json::json!(12)
    );
    assert_eq!(
        value["speakerCoverage"]["recordingsWithoutSpeakerData"],
        serde_json::json!(30)
    );

    let tools = build_ask_ai_tools(false, Vec::new());
    for name in ["search", "timeline"] {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("`{name}` must be offered"));
        assert!(
            tool.description.contains("speakerCoverage"),
            "`{name}` hands the model `speakerCoverage` but never names it: {}",
            tool.description
        );
        assert!(
            tool.description.to_lowercase().contains("partial"),
            "`{name}` must say a non-zero coverage count makes the answer partial: {}",
            tool.description
        );
    }
}

#[test]
fn broker_response_to_tool_value_strips_internal_ids_for_model() {
    // An audio result carries the internal frame DB id + sub-segment timing.
    // The model-facing value must NOT include them (they egress off-device for
    // a cloud engine and the model never uses them — source cards read them
    // locally from `search_metadata`).
    let mut audio = sample_result();
    audio.kind = "audio".to_string();
    audio.span_start_ms = Some(3_000);
    audio.span_end_ms = Some(5_000);
    audio.aligned_frame_id = Some(99);
    let response = BrokeredCaptureResponse::Search(BrokerSearchResponse {
        results: vec![audio],
        limit: 8,
        scope_clamped: false,
        required_scope: None,
        next_cursor: None,
        speaker_coverage: None,
    });

    let value = broker_response_to_tool_value(response).expect("search serializes");
    let result = &value["results"][0];
    // `skip_serializing_if = "Option::is_none"` drops them entirely once None.
    assert!(result.get("spanStartMs").is_none(), "spanStartMs stripped");
    assert!(result.get("spanEndMs").is_none(), "spanEndMs stripped");
    assert!(
        result.get("alignedFrameId").is_none(),
        "alignedFrameId stripped"
    );
    // The model-facing fields it DOES use survive.
    assert_eq!(result["opaqueId"], serde_json::json!("op-1"));
    assert_eq!(result["snippet"], serde_json::json!("build passed"));
}

#[test]
fn broker_response_to_tool_value_serializes_show_text() {
    let response = BrokeredCaptureResponse::ShowText(BrokerShowTextResponse {
        opaque_id: "op-1".to_string(),
        kind: "frame".to_string(),
        text: "full redacted text".to_string(),
        speakers: Vec::new(),
        turns: Vec::new(),
    });

    let value = broker_response_to_tool_value(response).expect("show_text serializes");
    assert_eq!(value["opaqueId"], serde_json::json!("op-1"));
    assert_eq!(value["text"], serde_json::json!("full redacted text"));
}

#[test]
fn broker_response_to_tool_value_error_returns_message() {
    let response = BrokeredCaptureResponse::Error(BrokerErrorResponse {
        error: BrokerAuthStatusKind::AuthorizationRequired,
        message: "result is unavailable or outside the grant scope".to_string(),
    });

    let error =
        broker_response_to_tool_value(response).expect_err("error envelope should become Err");
    assert_eq!(error, "result is unavailable or outside the grant scope");
}
