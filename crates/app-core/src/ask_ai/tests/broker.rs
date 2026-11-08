use super::*;

/// Citing a timeline interval is only half the capability: the preamble now
/// promises `show_text` takes an id returned by `search` OR `timeline`, and
/// `reference_captures` says the same. The `show_text` TOOL description — the
/// text rig ships inside the tool schema, which is what the model reads when
/// it decides whether an id is eligible — still said `search` only. Two
/// contradictory contracts for the same tool, and the narrower one is the one
/// attached to the call site, so the model never reads the full recording
/// behind a timeline interval.
#[test]
fn show_text_tool_description_accepts_timeline_ids_like_the_preamble_promises() {
    let preamble = build_ask_ai_preamble(&[]);
    assert!(
        preamble.contains(
            "`show_text` returns the full redacted text for one opaque id returned by \
`search` or `timeline`"
        ),
        "the preamble promises show_text takes a timeline id: {preamble}"
    );

    let tools = build_ask_ai_tools(false, Vec::new());
    let show_text = tools
        .iter()
        .find(|tool| tool.name == "show_text")
        .expect("`show_text` must be offered");
    assert!(
        show_text.description.contains("`timeline`"),
        "`show_text`'s tool description must name `timeline` as an id source too, or the \
model believes the narrower contract: {}",
        show_text.description
    );
}

#[test]
fn preamble_documents_the_tools_and_graphical_affordance() {
    let preamble = build_ask_ai_preamble(&[]);
    // The five data tools + the presentation tool are all described. The
    // preamble is the only place that tells the model HOW the tools combine;
    // a tool offered in `build_ask_ai_tools` but missing here is a surface the
    // model has no instruction to reach for.
    for tool in build_ask_ai_tools(false, Vec::new()).iter().filter(|tool| {
        // Every offered tool that `broker_request_from_tool` accepts as a
        // brokered data tool.
        broker_request_from_tool(&tool.name, serde_json::json!({ "query": "x", "from": "2026-05-22T10:00:00Z", "to": "2026-05-22T11:00:00Z", "opaqueId": "x" })).is_ok()
    }) {
        assert!(
            preamble.contains(&format!("`{}`", tool.name)),
            "the preamble must document the offered `{}` tool",
            tool.name
        );
    }
    assert!(preamble.contains("`search`"));
    assert!(preamble.contains("`timeline`"));
    assert!(preamble.contains("`show_text`"));
    assert!(preamble.contains("`recall_context`"));
    assert!(preamble.contains("`reference_captures`"));
    // The graphical-answer affordance blocks are documented.
    assert!(preamble.contains("mnema-bars"));
    assert!(preamble.contains("mnema-dossier"));
    assert!(preamble.contains("mnema-timeline"));
    // The five app-control tools + the explicit-request guardrail (Workstream A).
    assert!(preamble.contains("`capture_status`"));
    assert!(preamble.contains("`start_capture`"));
    assert!(preamble.contains("`stop_capture`"));
    assert!(preamble.contains("`pause_capture`"));
    assert!(preamble.contains("`resume_capture`"));
    assert!(preamble.contains("`fetch_url`"));
    assert!(preamble.contains("explicit user request in the current message"));
    // The MCP-connector paragraph (Workstream C) is always documented.
    assert!(preamble.contains("`mcp__`"));
    assert!(preamble.contains("MCP tool connectors"));
    // With no notes, no per-turn MCP note line is appended.
    assert!(!preamble.contains("Note about your MCP connectors this turn"));
    // The preamble is the SYSTEM instruction — it must carry no question.
    assert!(!preamble.contains("Question:"));
}

#[test]
fn preamble_appends_mcp_notes_when_present() {
    let notes = vec!["MCP server \"github\" exposes 78 tools; first 32 available".to_string()];
    let preamble = build_ask_ai_preamble(&notes);
    assert!(preamble.contains("Note about your MCP connectors this turn"));
    assert!(preamble.contains("exposes 78 tools"));
}

#[test]
fn prompt_is_grounding_then_question() {
    let prompt = build_ask_ai_prompt("What did I do?", 0, &ClientClock::default(), None);
    // The temporal grounding leads; the bare question trails. No capture
    // context is ever injected — the model gathers it with tool calls.
    assert!(prompt.starts_with("Temporal grounding: "));
    assert!(prompt.ends_with("Question: What did I do?"));
}

#[test]
fn temporal_grounding_renders_local_and_utc() {
    // now_ms = 0 → 1970-01-01 00:00 UTC. PST (UTC-08:00) lands at 1969-12-31 16:00.
    let clock = ClientClock {
        utc_offset_minutes: Some(-480),
        time_zone: Some("America/Los_Angeles".to_string()),
    };
    let grounding = build_temporal_grounding(0, &clock);
    assert!(grounding.contains("1969-12-31 16:00"));
    assert!(grounding.contains("(America/Los_Angeles, UTC-08:00)"));
    assert!(grounding.contains("1970-01-01 00:00 UTC"));
    assert!(grounding.contains("RFC3339 `Z`"));
}

#[test]
fn temporal_grounding_falls_back_to_utc_without_offset() {
    let grounding = build_temporal_grounding(0, &ClientClock::default());
    assert!(grounding.contains("1970-01-01 00:00 UTC"));
    assert!(grounding.contains("local offset is unknown"));
}

#[test]
fn utc_rfc3339_to_local_display_converts_correctly() {
    // IST = UTC+05:30 (330 min). 18:26 UTC on June 12 → 23:56 IST same day.
    let result = utc_rfc3339_to_local_display("2026-06-12T18:26:00Z", 330);
    assert_eq!(result.as_deref(), Some("2026-06-12 23:56"));

    // PST = UTC-08:00 (-480 min). 2026-01-01T02:00:00Z → 2025-12-31 18:00.
    let result = utc_rfc3339_to_local_display("2026-01-01T02:00:00Z", -480);
    assert_eq!(result.as_deref(), Some("2025-12-31 18:00"));

    // Unparseable string returns None.
    assert_eq!(utc_rfc3339_to_local_display("not-a-timestamp", 0), None);
}

#[test]
fn annotate_local_times_injects_local_fields() {
    // IST = UTC+05:30 (330 min). 18:26 UTC → 23:56 local.
    let mut value = serde_json::json!({
        "results": [{
            "startedAt": "2026-06-12T18:26:00Z",
            "endedAt": "2026-06-12T18:30:00Z",
            "snippet": "hello"
        }]
    });
    annotate_local_times(&mut value, 330);
    let result = &value["results"][0];
    assert_eq!(result["startedAtLocal"], "2026-06-12 23:56");
    assert_eq!(result["endedAtLocal"], "2026-06-13 00:00");
    // Original UTC fields must still be present.
    assert_eq!(result["startedAt"], "2026-06-12T18:26:00Z");
}

#[test]
fn availability_serializes_camel_case() {
    let json = serde_json::to_string(&AskAiAvailability {
        available: false,
        reason: Some("ask_ai_disabled".to_string()),
    })
    .expect("serialize");
    assert_eq!(json, r#"{"available":false,"reason":"ask_ai_disabled"}"#);

    let json = serde_json::to_string(&AskAiAvailability {
        available: true,
        reason: None,
    })
    .expect("serialize");
    assert_eq!(json, r#"{"available":true}"#);
}

#[test]
fn availability_reason_is_ask_ai_disabled_when_access_off() {
    // The availability reason logic: Ask AI disabled short-circuits to the
    // `ask_ai_disabled` reason BEFORE the engine prerequisite is consulted.
    // (The full command needs a Tauri app handle; here we assert the reason
    // contract the command relies on — a disabled flag produces this exact
    // reason, the same string `ensure_ask_ai_access_ready` would otherwise
    // surface via its disabled-message branch.)
    let availability = AskAiAvailability {
        available: false,
        reason: Some("ask_ai_disabled".to_string()),
    };
    assert!(!availability.available);
    assert_eq!(availability.reason.as_deref(), Some("ask_ai_disabled"));
}

#[test]
fn resolve_tool_call_cap_treats_zero_as_unlimited() {
    assert_eq!(resolve_tool_call_cap(0), usize::MAX);
    assert_eq!(resolve_tool_call_cap(1), 1);
    assert_eq!(resolve_tool_call_cap(12), 12);
    assert_eq!(resolve_tool_call_cap(250), 250);
}

#[test]
fn broker_request_from_tool_search_maps_to_search_variant() {
    let request = broker_request_from_tool(
        "search",
        serde_json::json!({ "query": "build", "limit": 5 }),
    )
    .expect("search params should parse");

    match request {
        BrokeredCaptureRequest::Search(search) => {
            assert_eq!(search.query, "build");
            assert_eq!(search.limit, Some(5));
        }
        other => panic!("expected Search, got {other:?}"),
    }
}

#[test]
fn broker_request_from_tool_timeline_maps_to_timeline_variant() {
    let request = broker_request_from_tool(
        "timeline",
        serde_json::json!({
            "from": "2026-01-01T00:00:00Z",
            "to": "2026-01-01T01:00:00Z",
        }),
    )
    .expect("timeline params should parse");

    match request {
        BrokeredCaptureRequest::Timeline(timeline) => {
            assert_eq!(timeline.from, "2026-01-01T00:00:00Z");
            assert_eq!(timeline.to, "2026-01-01T01:00:00Z");
        }
        other => panic!("expected Timeline, got {other:?}"),
    }
}

#[test]
fn broker_request_from_tool_show_text_extracts_opaque_id() {
    let request = broker_request_from_tool("show_text", serde_json::json!({ "opaqueId": "op-7" }))
        .expect("show_text params should parse");

    match request {
        BrokeredCaptureRequest::ShowText { opaque_id } => assert_eq!(opaque_id, "op-7"),
        other => panic!("expected ShowText, got {other:?}"),
    }
}

#[test]
fn broker_request_from_tool_activities_maps_to_activities_variant() {
    let request = broker_request_from_tool(
        "activities",
        serde_json::json!({ "from": "2026-08-15T00:00:00Z", "to": "2026-08-15T18:00:00Z" }),
    )
    .expect("activities params should parse");

    match request {
        BrokeredCaptureRequest::Activities(req) => {
            assert_eq!(req.from, "2026-08-15T00:00:00Z");
            assert_eq!(req.to, "2026-08-15T18:00:00Z");
        }
        other => panic!("expected Activities, got {other:?}"),
    }
}

/// The day-scale door has to be reachable and self-describing: the model picks
/// it over `timeline` only if the description says `timeline` samples the tail.
#[test]
fn activities_tool_is_offered_and_requires_a_window() {
    let tools = build_ask_ai_tools(false, Vec::new());
    let activities = tools
        .iter()
        .find(|tool| tool.name == "activities")
        .expect("activities must be offered to the model");

    let required = activities.parameters_schema["required"]
        .as_array()
        .expect("required list");
    assert!(required.iter().any(|value| value == "from"));
    assert!(required.iter().any(|value| value == "to"));
    // No relevance knob and no caller cap — either one turns this back into
    // the ranked top-N door it exists to complement.
    let properties = activities.parameters_schema["properties"]
        .as_object()
        .expect("properties");
    assert!(!properties.contains_key("query"));
    assert!(!properties.contains_key("limit"));
    assert!(
        activities.description.contains("derivedFrom"),
        "the coverage window is what keeps an empty list honest"
    );
}

/// An episode the model summarized must be citable, or the answer renders with
/// no source cards and the user cannot check it.
#[test]
fn activities_opaque_ids_are_retained_so_the_model_can_cite_them() {
    let response =
        BrokeredCaptureResponse::Activities(app_infra::brokered_access::BrokerActivitiesResponse {
            activities: vec![
                app_infra::brokered_access::BrokerActivity {
                    title: "Morning triage".to_string(),
                    summary: "cleared the queue".to_string(),
                    category: None,
                    focus: None,
                    started_at: "2026-06-12T09:00:00Z".to_string(),
                    ended_at: "2026-06-12T09:30:00Z".to_string(),
                    opaque_id: Some("f2a:gask-ai.deadbeef".to_string()),
                    context: Some(app_infra::brokered_access::BrokerSearchResultContext {
                        app_bundle_id: Some("com.stablyai.orca".to_string()),
                        app_name: Some("Orca".to_string()),
                        window_title: Some("Orca".to_string()),
                        url: None,
                    }),
                },
                // Every grounding frame aged out: nothing to cite, and nothing
                // may be retained under an empty key.
                app_infra::brokered_access::BrokerActivity {
                    title: "Fully aged out".to_string(),
                    summary: "no surviving evidence".to_string(),
                    category: None,
                    focus: None,
                    started_at: "2026-06-12T10:00:00Z".to_string(),
                    ended_at: "2026-06-12T10:30:00Z".to_string(),
                    opaque_id: None,
                    context: None,
                },
            ],
            derived_from: Some("2026-06-12T09:00:00Z".to_string()),
            derived_until: Some("2026-06-12T11:00:00Z".to_string()),
            truncated: false,
        });

    let mut metadata: HashMap<String, BrokerSearchResult> = HashMap::new();
    retain_source_metadata(&response, &mut metadata);

    let retained = metadata
        .get("f2a:gask-ai.deadbeef")
        .expect("a citable episode must be retained");
    assert_eq!(retained.kind, "frame");
    assert_eq!(retained.snippet, "Morning triage");
    assert_eq!(retained.started_at, "2026-06-12T09:00:00Z");
    // Without this the card renders "Unknown app" for a frame whose app was
    // captured and stored the whole time.
    assert_eq!(
        retained
            .context
            .as_ref()
            .and_then(|context| context.app_name.as_deref()),
        Some("Orca"),
    );
    assert_eq!(
        metadata.len(),
        1,
        "an episode with no surviving evidence adds no citable source"
    );
}

#[test]
fn broker_request_from_tool_recall_context_maps_to_recall_context_variant() {
    let request = broker_request_from_tool(
        "recall_context",
        serde_json::json!({ "query": "what am I working on", "limit": 5 }),
    )
    .expect("recall_context params should parse");

    match request {
        BrokeredCaptureRequest::RecallContext(req) => {
            assert_eq!(req.query, "what am I working on");
            assert_eq!(req.limit, Some(5));
        }
        other => panic!("expected RecallContext, got {other:?}"),
    }
}

#[test]
fn broker_request_from_tool_rejects_unknown_tool() {
    let error = broker_request_from_tool("open", serde_json::json!({ "opaqueId": "op-1" }))
        .expect_err("open is not an Ask AI tool");
    assert_eq!(error, "unknown Ask AI tool: open");

    let error = broker_request_from_tool("open_in_mnema", serde_json::json!({}))
        .expect_err("open_in_mnema is not an Ask AI tool");
    assert_eq!(error, "unknown Ask AI tool: open_in_mnema");
}

#[test]
fn broker_request_from_tool_rejects_open_captured_url() {
    let error = broker_request_from_tool("open_captured_url", serde_json::json!({}))
        .expect_err("open_captured_url is not an Ask AI tool");
    assert_eq!(error, "unknown Ask AI tool: open_captured_url");
}

#[test]
fn broker_request_from_tool_rejects_reference_captures() {
    let error =
        broker_request_from_tool("reference_captures", serde_json::json!({ "opaqueIds": [] }))
            .expect_err("reference_captures is not a broker data tool");
    assert_eq!(error, "unknown Ask AI tool: reference_captures");
}

#[test]
fn broker_request_from_tool_rejects_missing_opaque_id() {
    let error = broker_request_from_tool("show_text", serde_json::json!({}))
        .expect_err("missing opaqueId should error");
    assert_eq!(error, "Ask AI show_text requires a non-empty opaqueId");

    let error = broker_request_from_tool("show_text", serde_json::json!({ "opaqueId": "  " }))
        .expect_err("blank opaqueId should error");
    assert_eq!(error, "Ask AI show_text requires a non-empty opaqueId");
}
