use super::*;
use capture_types::McpTransport;

#[test]
fn connect_failure_log_head_drops_the_transport_error_and_the_label() {
    let cfg = stdio_cfg("mcp-1", "Ana's: Linear", "true", Vec::new());
    let head = |error: &str| connect_failure_log_head(error, &cfg);
    assert_eq!(
        head("failed to connect to \"Ana's: Linear\": error sending request for url (https://x.test/mcp?token=s3cret)"),
        "failed to connect to \"mcp-1\""
    );
    assert_eq!(
        head("timed out connecting to \"Ana's: Linear\" after 10s"),
        "timed out connecting to \"mcp-1\" after 10s"
    );
    assert!(!head("stdio connector \"Ana's: Linear\" has no command").contains("Linear"));
}

/// A stdio server config for tests: enabled, no env/secret/curation.
/// Dialing it reads the connector secret through the process vault, so pin
/// the shared test vault (idempotent) — the read must be Ok(None), never a
/// keychain touch or a "vault not initialized" error.
fn stdio_cfg(id: &str, label: &str, command: &str, args: Vec<String>) -> McpServerConfig {
    crate::test_support::install_shared_test_secret_vault();
    McpServerConfig {
        id: id.to_string(),
        label: label.to_string(),
        enabled: true,
        transport: McpTransport::Stdio,
        auth_mode: capture_types::McpAuthMode::Bearer,
        command: Some(command.to_string()),
        args,
        env: Vec::new(),
        url: None,
        secret_env_name: None,
        enabled_tools: None,
    }
}

/// A fresh slot for `cfg`, as `slot_for` would build it.
fn test_slot(cfg: &McpServerConfig) -> ServerSlot {
    ServerSlot {
        fingerprint: config_fingerprint(cfg),
        state: Mutex::new(SlotState::default()),
    }
}

/// A malicious/compromised MCP server ships a tool whose `inputSchema` is a
/// multi-megabyte object (padding + `description` fields the model reads).
/// `tool_info_from_rmcp` is the ONE projection of an rmcp `Tool` onto the
/// model-facing `ToolInfo`; its schema is streamed to the model as the tool's
/// parameter schema every turn. It caps the tool DESCRIPTION but must cap the
/// SCHEMA too, or one rogue server floods the turn (and opens an unbounded
/// prompt-injection channel via schema text) — the same INV-B5 bound the
/// result/description caps enforce.
#[test]
fn tool_info_from_rmcp_bounds_a_giant_server_schema() {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), serde_json::json!("object"));
    schema.insert(
        "description".to_string(),
        serde_json::json!("z".repeat(1_000_000)),
    );
    let tool = rmcp::model::Tool::new("echo", "does things", std::sync::Arc::new(schema));
    let info = tool_info_from_rmcp("srv", tool).expect("a valid tool name must survive");
    let schema_len = serde_json::to_string(&info.input_schema)
        .expect("schema serializes")
        .len();
    assert!(
        schema_len <= 20_000,
        "a server-controlled input schema reached the model unbounded ({schema_len} chars) — \
         it must be capped like the tool result and description (INV-B5)"
    );
}

/// A server tool name the provider would reject (a dot, or `mcp__<id>__<name>`
/// over 64 chars) must be DROPPED at discovery — offered anyway, the provider
/// rejects the ENTIRE request and every tool vanishes for the turn.
#[test]
fn tool_info_from_rmcp_drops_provider_invalid_tool_names() {
    let schema = || std::sync::Arc::new(serde_json::Map::new());
    let dotted = rmcp::model::Tool::new("list.files", "dotted", schema());
    assert!(tool_info_from_rmcp("srv", dotted).is_none());
    // `mcp__srv__` + 55 chars = 65 > 64.
    let long = rmcp::model::Tool::new("t".repeat(55), "long", schema());
    assert!(tool_info_from_rmcp("srv", long).is_none());
    let valid = rmcp::model::Tool::new("list_files", "fine", schema());
    assert!(tool_info_from_rmcp("srv", valid).is_some());
}

// ---- assemble_turn_tools: the pure per-turn assembly rules ----

/// A discovered tool as `connect_and_list` would cache it.
fn discovered_tool(name: &str) -> ToolInfo {
    ToolInfo {
        name: name.to_string(),
        description: Some(format!("does {name}")),
        input_schema: serde_json::json!({ "type": "object" }),
    }
}

/// One bad connector must cost only its own tools, never the turn.
#[test]
fn assemble_skips_an_errored_server_without_failing_the_turn() {
    let fetched = vec![
        (
            stdio_cfg("bad", "Bad", "sh", Vec::new()),
            Err("connection refused".to_string()),
        ),
        (
            stdio_cfg("good", "Good", "sh", Vec::new()),
            Ok((vec![discovered_tool("echo")], 0)),
        ),
    ];
    let (tools, notes) = assemble_turn_tools(fetched);
    assert_eq!(tools.len(), 1, "the healthy server's tools must survive");
    assert_eq!(tools[0].name, "mcp__good__echo");
    assert!(notes.is_empty(), "an errored server is skipped, not noted");
}

/// The default tool budget: an uncurated server over the 32-cap is trimmed
/// to the first 32 in server order, and the trim is NON-silent (one note).
#[test]
fn assemble_caps_an_uncurated_server_and_notes_the_trim() {
    let discovered: Vec<ToolInfo> = (0..40).map(|i| discovered_tool(&format!("t{i}"))).collect();
    let fetched = vec![(
        stdio_cfg("big", "Big", "sh", Vec::new()),
        Ok((discovered, 0)),
    )];
    let (tools, notes) = assemble_turn_tools(fetched);
    assert_eq!(tools.len(), 32);
    assert_eq!(tools[0].name, "mcp__big__t0");
    assert_eq!(tools[31].name, "mcp__big__t31");
    assert_eq!(notes.len(), 1, "the trim must surface exactly one note");
    assert!(
        notes[0].contains("\"Big\""),
        "note names the server: {}",
        notes[0]
    );
    assert!(notes[0].contains("40") && notes[0].contains("32"));
}

/// Offered tools carry the model-facing `mcp__<id>__<name>` and a
/// `(via <label>)` description prefix (with or without a server description).
#[test]
fn assemble_applies_model_naming_and_via_label_descriptions() {
    let mut undescribed = discovered_tool("bare");
    undescribed.description = None;
    let fetched = vec![(
        stdio_cfg("github-2", "GitHub", "sh", Vec::new()),
        Ok((vec![discovered_tool("create_issue"), undescribed], 0)),
    )];
    let (tools, _notes) = assemble_turn_tools(fetched);
    assert_eq!(tools[0].name, "mcp__github-2__create_issue");
    assert_eq!(tools[0].description, "(via GitHub) does create_issue");
    assert_eq!(tools[1].description, "(via GitHub)");
}

/// Offered schemas are normalized (a non-object becomes a permissive object)
/// and bounded (a giant object is replaced) before reaching the model.
#[test]
fn assemble_normalizes_and_bounds_offered_schemas() {
    let mut non_object = discovered_tool("a");
    non_object.input_schema = serde_json::json!("not an object");
    let mut giant = discovered_tool("b");
    giant.input_schema = serde_json::json!({ "type": "object", "description": "z".repeat(50_000) });
    let fetched = vec![(
        stdio_cfg("srv", "Srv", "sh", Vec::new()),
        Ok((vec![non_object, giant], 0)),
    )];
    let (tools, _notes) = assemble_turn_tools(fetched);
    assert!(
        tools[0].parameters_schema.is_object(),
        "non-object must normalize"
    );
    let bounded = serde_json::to_string(&tools[1].parameters_schema).expect("serializes");
    assert!(
        bounded.len() <= 20_000,
        "giant schema must be bounded: {} chars",
        bounded.len()
    );
}

/// Invalid-name drops recorded at discovery surface as one preamble note per
/// turn — non-silent, mirroring the 32-cap trim.
#[test]
fn assemble_notes_invalid_name_drops_from_discovery() {
    let fetched = vec![(
        stdio_cfg("srv", "Srv", "sh", Vec::new()),
        Ok((vec![discovered_tool("ok")], 2)),
    )];
    let (tools, notes) = assemble_turn_tools(fetched);
    assert_eq!(tools.len(), 1, "surviving tools are still offered");
    assert_eq!(notes.len(), 1, "the drop must surface exactly one note");
    assert!(
        notes[0].contains("\"Srv\"") && notes[0].contains('2'),
        "{}",
        notes[0]
    );
}

/// `reap_slots` (the pure core of `reconcile`) retains only enabled ids;
/// a reaped slot's child dies with its last ref.
#[test]
fn reap_slots_retains_only_enabled_ids() {
    let keep_cfg = stdio_cfg("keep", "Keep", "sh", Vec::new());
    let gone_cfg = stdio_cfg("gone", "Gone", "sh", Vec::new());
    let mut slots = HashMap::new();
    slots.insert("keep".to_string(), Arc::new(test_slot(&keep_cfg)));
    slots.insert("gone".to_string(), Arc::new(test_slot(&gone_cfg)));
    let keep: HashSet<String> = ["keep".to_string()].into();
    reap_slots(&keep, &mut slots);
    assert_eq!(slots.len(), 1);
    assert!(slots.contains_key("keep"));
}

/// `slot_for` is the slot cache's whole policy: an unchanged config must
/// REUSE the slot (no needless redial mid-session); a connect-relevant edit
/// must REPLACE it (the old child dies on last ref, next use dials fresh).
#[tokio::test]
async fn slot_for_reuses_an_unchanged_config_and_replaces_an_edited_one() {
    let inner = Inner::default();
    let cfg = stdio_cfg("srv", "Srv", "sh", Vec::new());
    let first = inner.slot_for(&cfg).await;
    let second = inner.slot_for(&cfg).await;
    assert!(
        Arc::ptr_eq(&first, &second),
        "same config must reuse the slot"
    );

    let mut edited = cfg.clone();
    edited.args.push("--flag".to_string());
    let third = inner.slot_for(&edited).await;
    assert!(
        !Arc::ptr_eq(&second, &third),
        "an edited config must replace the slot"
    );
    let cached = Arc::clone(inner.slots.lock().await.get("srv").expect("slot present"));
    assert!(
        Arc::ptr_eq(&cached, &third),
        "the map must hold the NEW slot"
    );
}

/// The bounded second-failure arm of the redial policy: a server that fails
/// twice in a row (here: a command that cannot even spawn) must surface the
/// readable "failed twice" error, not retry forever.
#[tokio::test]
async fn a_connect_that_fails_twice_surfaces_the_failed_twice_error() {
    let cfg = stdio_cfg(
        "test-gone-mcp-server",
        "Gone",
        "/nonexistent/mnema-mcp-no-such-binary",
        Vec::new(),
    );
    let slot = test_slot(&cfg);
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        call_with_redial(
            &slot,
            &cfg,
            "echo",
            &serde_json::json!({}),
            CALL_TOOL_BUDGET,
        ),
    )
    .await
    .expect("a spawn failure must fail fast, not hang");
    let error = result.expect_err("an unspawnable server must fail the call");
    assert!(
        error.contains("failed this tool call twice"),
        "the redial's second failure must surface the bounded twice-failed arm: {error}"
    );
}

/// A unique scratch dir for one test's fixture signal files.
fn fixture_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// `warm` and `list_server_tools` call `ensure_ready` with no timeout of their
/// own, and `ensure_ready` holds the per-slot lock across the connect. A server
/// that hangs during initialize must therefore be bounded here, or it poisons
/// the slot forever (leaked child/task) and taxes every later turn.
#[tokio::test]
async fn ensure_ready_is_bounded_on_a_hung_server() {
    // `sleep` ignores stdin and never writes stdout, so the MCP initialize
    // handshake never completes (rmcp's `serve_client` has no internal timeout).
    let cfg = stdio_cfg(
        "test-hung-mcp-server",
        "Hung",
        "sleep",
        vec!["60".to_string()],
    );
    let slot = test_slot(&cfg);
    // Beyond the discovery budget: a bounded `ensure_ready` returns an error
    // inside its budget; an unbounded one never returns and this outer window
    // elapses.
    let outcome = tokio::time::timeout(
        DISCOVERY_TURN_BUDGET + Duration::from_secs(3),
        slot.ensure_ready(&cfg),
    )
    .await;
    assert!(
        matches!(outcome, Ok(Err(_))),
        "ensure_ready on a hung server must return a bounded error, not hang"
    );
}

// Classification is the whole retry policy: `TransportSend` is the one
// variant that provably never reached the server. `McpError`, `Timeout`,
// `TransportClosed`, `UnexpectedResponse`, and `Cancelled` are all
// constructible in rmcp 2.1.0 (`DynamicTransportError::from_parts` exists
// for exactly this), so every variant is covered.

fn transport_send_error() -> rmcp::ServiceError {
    rmcp::ServiceError::TransportSend(rmcp::transport::DynamicTransportError::from_parts(
        "test",
        std::any::TypeId::of::<()>(),
        Box::new(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe")),
    ))
}

#[test]
fn classify_transport_send_is_retryable() {
    let failure = classify_service_error(transport_send_error());
    assert!(
        failure.retryable,
        "a failed transport write never reached the server"
    );
    assert!(
        failure.text.contains("pipe"),
        "text must stay readable: {}",
        failure.text
    );
}

#[test]
fn classify_post_send_errors_are_not_retryable() {
    let post_send = [
        rmcp::ServiceError::TransportClosed,
        rmcp::ServiceError::Timeout {
            timeout: Duration::from_secs(60),
        },
        rmcp::ServiceError::McpError(rmcp::model::ErrorData::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "boom",
            None,
        )),
        rmcp::ServiceError::UnexpectedResponse,
        rmcp::ServiceError::Cancelled {
            reason: Some("test".to_string()),
        },
    ];
    for error in post_send {
        let text = error.to_string();
        let failure = classify_service_error(error);
        assert!(
            !failure.retryable,
            "{text}: may have executed server-side; a redial could duplicate side effects"
        );
        assert_eq!(failure.text, text, "error text must surface unchanged");
    }
}

/// A scripted stdio MCP server for the retryable-path test. It speaks just
/// enough newline-delimited JSON-RPC for rmcp's client: answers
/// `initialize` and `tools/list` (ignoring `notifications/initialized`).
/// Run 1 then closes its STDIN but keeps running (stdout open), so the
/// client's next send fails at the transport write — rmcp's service loop is
/// still alive and reports `TransportSend`, the provably-pre-transmit
/// failure. (Exiting instead would kill the loop via stdout EOF and yield
/// `TransportClosed`, the non-retryable ambiguous case.) Run 2 is a healthy
/// server that answers `tools/call`. Each startup appends to a counter file.
fn retry_fixture_script(counter: &std::path::Path, stdin_closed: &std::path::Path) -> String {
    format!(
        r#"echo run >> "{counter}"
RUN=$(wc -l < "{counter}")
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
*'"method":"initialize"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fixture","version":"0"}}}}}}'
  ;;
*'"method":"tools/list"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"tools":[{{"name":"echo","inputSchema":{{"type":"object"}}}}]}}}}'
  if [ "$RUN" -eq 1 ]; then
    exec 0<&-
    : > "{stdin_closed}"
    exec sleep 60
  fi
  ;;
*'"method":"tools/call"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"content":[{{"type":"text","text":"ok"}}],"isError":false}}}}'
  ;;
  esac
done"#,
        counter = counter.display(),
        stdin_closed = stdin_closed.display(),
    )
}

/// The retryable path end to end: a transport-write failure (request never
/// transmitted) must redial exactly once and succeed on the fresh
/// connection. The counter file proves the second connect happened.
#[tokio::test]
async fn transport_send_failure_redials_once() {
    let dir = fixture_dir("mnema-mcp-retry-test");
    let counter = dir.join("connects");
    let stdin_closed = dir.join("stdin-closed");
    let cfg = stdio_cfg(
        "test-retry-mcp-server",
        "Retry",
        "sh",
        vec![
            "-c".to_string(),
            retry_fixture_script(&counter, &stdin_closed),
        ],
    );
    let slot = test_slot(&cfg);

    // Warm the connection on run 1, then wait until the fixture has
    // PROVABLY closed its stdin — only then is the next write guaranteed to
    // fail pre-transmit rather than land in the pipe and hang.
    slot.ensure_ready(&cfg)
        .await
        .expect("run 1 handshake must succeed");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !stdin_closed.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fixture never signaled stdin close"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let result = tokio::time::timeout(
        Duration::from_secs(20),
        call_with_redial(
            &slot,
            &cfg,
            "echo",
            &serde_json::json!({}),
            CALL_TOOL_BUDGET,
        ),
    )
    .await
    .expect("call must not hang");
    assert!(
        result.is_ok(),
        "redial onto the healthy run-2 server must succeed: {result:?}"
    );
    assert!(
        result.unwrap().contains("ok"),
        "run 2's tool result must come back"
    );

    let connects = std::fs::read_to_string(&counter).expect("counter file");
    assert_eq!(
        connects.lines().count(),
        2,
        "exactly one redial: two connects total"
    );

    drop(slot); // kills run 2's child; run 1's died with its handle
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scripted stdio MCP server (same framing as [`retry_fixture_script`])
/// that answers the handshake + `tools/list`, then STALLS on `tools/call`:
/// appends the invocation to a counter file and execs into a `sleep` —
/// stdout stays open, no response ever comes, the call await hangs.
fn stall_fixture_script(calls: &std::path::Path) -> String {
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
*'"method":"initialize"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fixture","version":"0"}}}}}}'
  ;;
*'"method":"tools/list"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"tools":[{{"name":"echo","inputSchema":{{"type":"object"}}}}]}}}}'
  ;;
*'"method":"tools/call"'*)
  echo call >> "{calls}"
  exec sleep 60
  ;;
  esac
done"#,
        calls = calls.display(),
    )
}

/// The execution budget end to end: a server that hangs on `tools/call`
/// must yield a bounded, NON-retried error and leave the slot invalidated.
/// The counter file proves exactly one `tools/call` reached the fixture (a
/// timed-out call may have executed server-side, so a retry could duplicate
/// a write — ADR 0048); the emptied slot proves the next call dials fresh.
/// The budget is injected small: a real [`CALL_TOOL_BUDGET`] would take a
/// minute, and tokio virtual time cannot bound a real child process
/// (auto-advance races past real I/O, spuriously elapsing the connect too).
#[tokio::test]
async fn hung_tool_call_is_bounded_not_retried_and_invalidates_the_slot() {
    let dir = fixture_dir("mnema-mcp-stall-test");
    let calls = dir.join("calls");
    let cfg = stdio_cfg(
        "test-stall-mcp-server",
        "Stall",
        "sh",
        vec!["-c".to_string(), stall_fixture_script(&calls)],
    );
    let slot = test_slot(&cfg);

    let budget = Duration::from_secs(2);
    let result = tokio::time::timeout(
        budget + Duration::from_secs(8), // grace covers the (fast) connect
        call_with_redial(&slot, &cfg, "echo", &serde_json::json!({}), budget),
    )
    .await
    .expect("a hung tools/call must be bounded by the budget, not hang");

    let error = result.expect_err("a timed-out call must surface as an error");
    assert!(
        error.contains("timed out after 2 s"),
        "readable timeout text expected: {error}"
    );
    let calls_seen = std::fs::read_to_string(&calls).expect("counter file");
    assert_eq!(
        calls_seen.lines().count(),
        1,
        "a timed-out call must NOT be retried"
    );
    assert!(
        slot.state.lock().await.established.is_none(),
        "timeout must invalidate the slot so the next call dials fresh"
    );

    // The stalled fixture already died with its handle (invalidate + return
    // dropped the last ref); only the scratch dir is left to clean.
    let _ = std::fs::remove_dir_all(&dir);
}
