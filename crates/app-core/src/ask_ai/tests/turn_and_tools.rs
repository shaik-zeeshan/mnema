use super::*;

#[test]
fn a_failed_engine_resolve_is_saved_as_an_error_turn() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let infra: AppInfraState = Arc::new(
                ::app_infra::AppInfra::initialize(dir.path())
                    .await
                    .expect("infra"),
            );
            let saved =
                persist_resolve_failure(&infra, "c1", "Title", "chat", 0, "q?", "no_default_model")
                    .await;
            assert!(saved);
            let conv = infra
                .conversation()
                .get_conversation("c1")
                .await
                .expect("read")
                .expect("conversation exists");
            assert_eq!(conv.title, "Title");
            assert_eq!(conv.origin, "chat");
            assert_eq!(conv.turns.len(), 1);
            assert_eq!(conv.turns[0].phase, "error");
            assert_eq!(
                conv.turns[0].error_message.as_deref(),
                Some("no_default_model")
            );
        });
}
#[test]
fn a_turn_stopped_before_the_model_is_saved_unless_abandoned() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let infra: AppInfraState = Arc::new(
                ::app_infra::AppInfra::initialize(dir.path())
                    .await
                    .expect("infra"),
            );
            // A follow-up (turn 1) stopped before the model ran keeps its
            // row, so the next send and a Retry agree with the store.
            persist_turn(
                &infra,
                "c1",
                "T",
                "quick_recall",
                0,
                "q0",
                "a0",
                None,
                Some(&[]),
                &[],
                &[],
                "done",
                None,
            )
            .await;
            persist_stopped_before_model(
                &infra,
                "c1",
                "",
                "quick_recall",
                Some("quick_recall"),
                1,
                "q1",
            )
            .await;
            let conv = infra
                .conversation()
                .get_conversation("c1")
                .await
                .expect("read")
                .expect("conversation exists");
            assert_eq!(conv.turns.len(), 2);
            assert_eq!(conv.turns[1].question, "q1");
            assert_eq!(conv.turns[1].phase, "done");

            // An abandoned Quick Recall first question leaves nothing behind.
            persist_stopped_before_model(&infra, "c2", "T", "quick_recall", None, 0, "q").await;
            assert!(infra
                .conversation()
                .get_conversation("c2")
                .await
                .expect("read")
                .is_none());
        });
}

#[test]
fn the_context_bar_and_the_debug_row_agree_on_a_lump_sum_report() {
    // The usual case: the provider reports a split and the occupancy is
    // their sum.
    assert_eq!(context_tokens(11, 7, 18), 18);
    assert_eq!(
        context_tokens(11, 7, 0),
        18,
        "a missing total is not authoritative"
    );

    // rig keeps `total_tokens` separate because some providers report only
    // one number. Both surfaces that show occupancy read this one function,
    // so a lump-sum report cannot render as a real context bar next to an
    // "in 0 tok · out 0 tok" debug row.
    assert_eq!(context_tokens(0, 0, 4321), 4321);

    // No report at all stays zero rather than inventing a number.
    assert_eq!(context_tokens(0, 0, 0), 0);

    // Provider-supplied values are untrusted arithmetic inputs.
    assert_eq!(
        context_tokens(u64::MAX, 1, 0),
        u64::MAX,
        "no overflow panic"
    );
}

#[test]
fn ask_ai_broker_identity_uses_ask_ai_label() {
    let identity = ask_ai_broker_identity().expect("Ask AI identity should be valid");

    assert_eq!(identity.label, "Ask AI");
    assert_eq!(identity.normalized_label, "ask ai");
    assert_eq!(identity.source, BrokerClientIdentitySource::Inferred);
}

#[test]
fn build_ask_ai_tools_gates_fetch_url_on_the_setting() {
    // Security invariant: the network-fetch tool is opt-in. It must be ABSENT
    // from the model's toolset when the setting is off, and PRESENT when on.
    let off = build_ask_ai_tools(false, Vec::new());
    assert!(
        !off.iter().any(|t| t.name == "fetch_url"),
        "fetch_url must not be offered when web-fetch is disabled"
    );
    let on = build_ask_ai_tools(true, Vec::new());
    assert!(
        on.iter().any(|t| t.name == "fetch_url"),
        "fetch_url must be offered when web-fetch is enabled"
    );
}

#[test]
fn build_ask_ai_tools_always_offers_app_control_and_appends_mcp() {
    let mcp = vec![ai_engine::AgentTool {
        name: "mcp__github__create_issue".to_string(),
        description: "(via GitHub) create an issue".to_string(),
        parameters_schema: serde_json::json!({ "type": "object" }),
    }];
    let tools = build_ask_ai_tools(false, mcp);
    // App-control tools are unconditional (no setting gates them).
    for name in [
        "capture_status",
        "start_capture",
        "stop_capture",
        "pause_capture",
        "resume_capture",
    ] {
        assert!(
            tools.iter().any(|t| t.name == name),
            "app-control tool `{name}` must always be present"
        );
    }
    // Passed MCP tools are appended verbatim.
    assert!(tools.iter().any(|t| t.name == "mcp__github__create_issue"));
}

/// Without discovery the model can only filter by a handle it has no way to
/// learn, so the whole speaker surface is unreachable from the app's own chat.
#[test]
fn speakers_tool_is_offered_and_its_response_is_handled() {
    let tools = build_ask_ai_tools(false, Vec::new());
    assert!(
        tools.iter().any(|tool| tool.name == "speakers"),
        "the speaker discovery tool must be offered"
    );
    for tool in tools
        .iter()
        .filter(|tool| tool.name == "search" || tool.name == "timeline")
    {
        assert!(
            tool.parameters_schema["properties"]
                .get("speaker")
                .is_some(),
            "`{}` must accept the speaker handle",
            tool.name
        );
        assert!(
            tool.description.contains("speaker"),
            "`{}` must tell the model the filter exists",
            tool.name
        );
    }

    let request = broker_request_from_tool("speakers", serde_json::json!({ "name": "priya" }))
        .expect("speakers params should map to a broker request");
    let BrokeredCaptureRequest::Speakers(request) = request else {
        panic!("speakers must map to a Speakers request");
    };
    assert_eq!(request.name.as_deref(), Some("priya"));

    let response =
        BrokeredCaptureResponse::Speakers(app_infra::brokered_access::BrokerSpeakersResponse {
            speakers: vec![app_infra::brokered_access::BrokerSpeakerSummary {
                name: Some("Priya".to_string()),
                handle: app_infra::brokered_access::BrokerSpeakerHandle {
                    id: "p1.signature".to_string(),
                    kind: "person".to_string(),
                    start_ms: None,
                    end_ms: None,
                },
                speaking_ms: 42_000,
                assigned_turns: 40,
                recognized_turns: 12,
            }],
            limit: 20,
            truncated: true,
        });
    let value = broker_response_to_tool_value(response)
        .expect("a speakers response is a handled path, not an unexpected one");
    assert_eq!(value["speakers"][0]["handle"]["id"], "p1.signature");
    assert_eq!(value["speakers"][0]["handle"]["kind"], "person");
    assert_eq!(value["speakers"][0]["recognizedTurns"], 12);
    assert_eq!(value["truncated"], true);
}

/// The preamble sends BOTH "what did X say" and "when was X talking" to "ONE
/// filtered `search` or `timeline`" — but `search` REQUIRES a `query`, and
/// neither of those questions carries a keyword. A model obeying that
/// instruction has to invent one, and an invented keyword silently narrows a
/// person's words down to the recordings that happen to match it; the answer
/// then reports that slice as everything they said, which the new
/// `speakerCoverage` rule cannot catch (coverage counts audio the filter could
/// not check, not audio the keyword threw away). The CLI door publishes the
/// rule in this same branch (`.agents/skills/mnema-data/SKILL.md`: "a question
/// with no keyword in it … is answerable only through `timeline --speaker`; do
/// not invent a keyword to force it through `search`"); this door offers the
/// same filter with no such instruction.
#[test]
fn preamble_routes_a_keywordless_person_question_to_timeline() {
    let tools = build_ask_ai_tools(false, Vec::new());
    let search = tools
        .iter()
        .find(|tool| tool.name == "search")
        .expect("`search` must be offered");
    // The premise: `search` cannot run without a keyword.
    assert!(
        search.parameters_schema["required"]
            .as_array()
            .expect("`search` declares its required params")
            .iter()
            .any(|value| value.as_str() == Some("query")),
        "`search` requires `query`: {}",
        search.parameters_schema
    );
    // …and the preamble really does aim a keyword-less question at `search`.
    let preamble = build_ask_ai_preamble(&[]);
    assert!(
        preamble.contains("\"what did X say\""),
        "the preamble routes \"what did X say\" through the speaker workflow: {preamble}"
    );

    let lowered = preamble.to_lowercase();
    assert!(
        lowered.contains("no keyword") || lowered.contains("without a keyword"),
        "the preamble must route a question with no keyword in it to `timeline`, the only \
tool that can answer it: {preamble}"
    );
    assert!(
        lowered.contains("invent a keyword"),
        "the preamble must forbid inventing a keyword to force a person question through \
`search`: {preamble}"
    );
}

#[test]
fn classify_ask_ai_tool_routes_in_precedence_order() {
    // App-control first (Workstream A).
    for name in [
        "capture_status",
        "start_capture",
        "stop_capture",
        "pause_capture",
        "resume_capture",
    ] {
        assert_eq!(
            classify_ask_ai_tool(name),
            ToolRoute::AppControl,
            "{name} must route app-control"
        );
    }
    // Then `fetch_url` (Workstream B).
    assert_eq!(classify_ask_ai_tool("fetch_url"), ToolRoute::FetchUrl);
    // Then MCP connectors (Workstream C), split into server id + tool.
    assert_eq!(
        classify_ask_ai_tool("mcp__github__create_issue"),
        ToolRoute::Mcp {
            server_id: "github",
            tool: "create_issue"
        }
    );
    // Everything else — data tools, unknowns, and malformed `mcp__` names
    // (no `__` separator, empty halves) — falls through to the broker.
    for name in [
        "search",
        "timeline",
        "show_text",
        "speakers",
        "recall_context",
        "activities",
        "mcp__",
        "mcp__nounderscore",
        "mcp__id__",
        "unknown_tool",
    ] {
        assert_eq!(
            classify_ask_ai_tool(name),
            ToolRoute::Broker,
            "{name} must fall through to the broker"
        );
    }
}

#[test]
fn mcp_connector_suffix_uses_trimmed_label_and_filters_blanks() {
    let labels: std::collections::HashMap<String, String> = [
        ("github".to_string(), " GitHub ".to_string()),
        ("blank".to_string(), "   ".to_string()),
    ]
    .into();
    // A known connector's label is trimmed into the suffix.
    assert_eq!(
        mcp_connector_suffix("mcp__github__create_issue", &labels).as_deref(),
        Some(" from GitHub")
    );
    // A blank/whitespace label yields NO suffix, not an empty " from ".
    assert_eq!(mcp_connector_suffix("mcp__blank__tool", &labels), None);
    // Unknown server ids and non-MCP names get no suffix.
    assert_eq!(mcp_connector_suffix("mcp__unknown__tool", &labels), None);
    assert_eq!(mcp_connector_suffix("search", &labels), None);
}
