use super::*;
use capture_types::{McpEnvVar, McpServerConfig, McpTransport};

fn fixture_dir(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// A scripted stdio MCP server that answers the handshake + `tools/list`,
/// then on every `tools/call` returns a JSON-RPC ERROR whose `message` is a
/// giant server-controlled string (passed in via the `MNEMA_BIG_ERROR` env).
/// This is the "error result from the server" that the 24k result cap must
/// bound before it is streamed back to the model as a tool result.
const HUGE_ERROR_SCRIPT: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
*'"method":"initialize"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}'
  ;;
*'"method":"tools/list"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}'
  ;;
*'"method":"tools/call"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32000,"message":"'"$MNEMA_BIG_ERROR"'"}}'
  ;;
  esac
done"#;

/// A malicious/compromised MCP server answers `tools/call` with a JSON-RPC
/// error carrying a multi-ten-kilobyte `message`. That server-controlled
/// error text is streamed to the model as the tool result, so it MUST be
/// bounded by the same 24k cap as a successful result — otherwise one rogue
/// server floods the turn (and opens an unbounded prompt-injection channel)
/// on every failed call.
#[tokio::test]
async fn a_giant_server_error_result_is_bounded_before_reaching_the_model() {
    // Dialing this server reads the connector secret through the PROCESS
    // vault. Without pinning the shared test vault first, that read falls
    // through to `SecretVaultHandle::new` -> the REAL `day.mnema.vault`
    // keychain item — prompting the developer, and (because the process slot
    // is first-writer-wins) leaving the whole test binary pointed at the
    // production vault. The sibling `mod tests` pins it inside `stdio_cfg`;
    // these later modules build their own config and have to do it too.
    crate::test_support::install_shared_test_secret_vault();
    let dir = fixture_dir("mnema-mcp-huge-error");
    let big = "x".repeat(60_000);
    let cfg = McpServerConfig {
        id: "test-huge-error-mcp-server".to_string(),
        label: "HugeError".to_string(),
        enabled: true,
        transport: McpTransport::Stdio,
        auth_mode: capture_types::McpAuthMode::Bearer,
        command: Some("sh".to_string()),
        args: vec!["-c".to_string(), HUGE_ERROR_SCRIPT.to_string()],
        env: vec![McpEnvVar {
            name: "MNEMA_BIG_ERROR".to_string(),
            value: big,
        }],
        url: None,
        secret_env_name: None,
        enabled_tools: None,
    };
    let slot = ServerSlot {
        fingerprint: config_fingerprint(&cfg),
        state: Mutex::new(SlotState::default()),
    };

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
    .expect("the call must not hang");

    let error = result.expect_err("a server JSON-RPC error must surface as an Err");
    assert!(
        error.chars().count() <= 24_600,
        "a server-controlled error result reached the model unbounded ({} chars) — the 24k \
         result cap must bound error results too, not only successful ones",
        error.chars().count()
    );

    drop(slot);
    let _ = std::fs::remove_dir_all(&dir);
}
