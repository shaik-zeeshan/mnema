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
/// then on `tools/call` returns a SUCCESSFUL JSON-RPC result (not a JSON-RPC
/// error) carrying `isError:true` and a huge `content` text (injected via the
/// `MNEMA_BIG_CONTENT` env). This is an in-band tool-level error: rmcp yields
/// `Ok(CallToolResult { is_error: Some(true), .. })`. Because `content`
/// serializes BEFORE `isError`, once the content passes the 24k result cap the
/// `"isError":true` flag is exactly what gets truncated away.
const BIG_INBAND_ERROR_SCRIPT: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
*'"method":"initialize"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"0"}}}'
  ;;
*'"method":"tools/list"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}'
  ;;
*'"method":"tools/call"'*)
  printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"content":[{"type":"text","text":"'"$MNEMA_BIG_CONTENT"'"}],"isError":true}}'
  ;;
  esac
done"#;

/// A tool that returns an in-band error (`isError:true`) whose content
/// exceeds the 24k result cap must still tell the model it was an error.
/// Because `content` serializes before `isError`, truncating the serialized
/// result string drops the flag and the (truncated) error content reaches the
/// model as an apparent success — poisoning the turn.
#[tokio::test]
async fn large_in_band_error_result_keeps_its_error_signal() {
    // Dialing this server reads the connector secret through the PROCESS
    // vault. Without pinning the shared test vault first, that read falls
    // through to `SecretVaultHandle::new` -> the REAL `day.mnema.vault`
    // keychain item — prompting the developer, and (because the process slot
    // is first-writer-wins) leaving the whole test binary pointed at the
    // production vault. The sibling `mod tests` pins it inside `stdio_cfg`;
    // these later modules build their own config and have to do it too.
    crate::test_support::install_shared_test_secret_vault();
    let dir = fixture_dir("mnema-mcp-inband-error");
    let big = "x".repeat(60_000);
    let cfg = McpServerConfig {
        id: "test-inband-error-mcp-server".to_string(),
        label: "InbandError".to_string(),
        enabled: true,
        transport: McpTransport::Stdio,
        auth_mode: capture_types::McpAuthMode::Bearer,
        command: Some("sh".to_string()),
        args: vec!["-c".to_string(), BIG_INBAND_ERROR_SCRIPT.to_string()],
        env: vec![McpEnvVar {
            name: "MNEMA_BIG_CONTENT".to_string(),
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
    .expect("the call must not hang")
    .expect("an in-band error result is a successful transport response (Ok)");

    assert!(
        result.to_lowercase().contains("error"),
        "a truncated in-band error result must still signal the error to the \
         model, else a tool error is read as a success; got prefix: {}",
        &result.chars().take(80).collect::<String>()
    );

    drop(slot);
    let _ = std::fs::remove_dir_all(&dir);
}
