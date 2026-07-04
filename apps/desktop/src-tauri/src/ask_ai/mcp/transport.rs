//! MCP transport construction + per-transport secret delivery (ADR 0048).
//!
//! Builds an initialized client [`McpClient`] for one enabled server. The single
//! optional keychain secret (keyed by instance id) is injected per transport:
//! HTTP → `Authorization: Bearer`; stdio → the env var the server names
//! (`secret_env_name`). Only ENABLED servers ever reach here (the manager
//! filters), and nothing here connects at app launch — the deferred-startup
//! invariant is the manager's concern.

use app_infra::load_mcp_server_secret;
use capture_types::{McpServerConfig, McpTransport};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};

/// A connected, initialized MCP client for one server. The client is an
/// `RunningService`: dropping it cancels the service loop, which closes the
/// transport. For stdio that KILLS the whole process GROUP
/// (`rmcp::transport::child_process::ChildWithCleanup::drop` → `kill()` →
/// `killpg`): the child is spawned as a process-group leader on Unix, so a
/// launcher's grandchildren (e.g. the real server behind `npx`) die with it.
/// The manager needs no explicit child teardown — dropping a cached handle is
/// enough. (Unix-only group semantics; macOS exercised — SUPPORTS.md.)
pub(crate) type McpClient = RunningService<RoleClient, ()>;

/// A stable fingerprint of the CONNECT-relevant config fields only. When it
/// changes (a Settings edit to the command/url/env/secret-name/transport) the
/// manager drops the cached handle and redials. A label rename or a curation
/// (`enabled_tools`) change deliberately does NOT change it — curation is applied
/// at turn build against the already-discovered tool list, no redial needed.
///
/// Note: the secret VALUE lives in the keychain (re-read at connect), not here,
/// so a pure secret rotation without a config edit is a documented v1 gap — the
/// failure-policy redial-on-next-error picks it up once the stale secret fails.
pub(crate) fn config_fingerprint(cfg: &McpServerConfig) -> String {
    serde_json::json!({
        "transport": cfg.transport,
        "command": cfg.command,
        "args": cfg.args,
        "env": cfg.env,
        "url": cfg.url,
        "secretEnvName": cfg.secret_env_name,
    })
    .to_string()
}

/// Connect to one server, run the MCP initialize handshake, and return the live
/// client. The single keychain secret (if any) is injected per transport.
pub(crate) async fn connect(cfg: &McpServerConfig) -> Result<McpClient, String> {
    // The single optional secret lives ONLY in the OS keychain, keyed by id
    // (never in the persisted settings). Read at connect time.
    let secret = load_mcp_server_secret(&cfg.id)
        .map_err(|error| format!("failed to read the secret for \"{}\": {error}", cfg.label))?;

    match cfg.transport {
        McpTransport::Stdio => connect_stdio(cfg, secret).await,
        McpTransport::Http => connect_http(cfg, secret).await,
    }
}

async fn connect_stdio(cfg: &McpServerConfig, secret: Option<String>) -> Result<McpClient, String> {
    let command = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| format!("stdio connector \"{}\" has no command", cfg.label))?;

    // ponytail: bare command name — a packaged macOS app has a minimal PATH (see
    // the macOS-GUI-PATH note in CLAUDE.md), so `npx` resolves only if the user
    // gives a PATH-reachable or absolute command. PATH augmentation would go here
    // if bare-`npx` configs prove common; not built until they do.
    let mut command_builder = tokio::process::Command::new(command);
    command_builder.args(&cfg.args);
    // Non-secret env rows are plain settings values.
    for env in &cfg.env {
        command_builder.env(&env.name, &env.value);
    }
    // stdio secret delivery: the env var the user named carries the keychain
    // secret (e.g. `GITHUB_TOKEN`). Skipped when either the name or the secret is
    // absent.
    let secret_env_name = cfg
        .secret_env_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let (Some(name), Some(secret)) = (secret_env_name, secret) {
        command_builder.env(name, secret);
    }

    // Spawn as a process-group leader so rmcp's drop-kill (`killpg`) takes out
    // the whole group — the launcher (`npx`) AND its server grandchildren — not
    // just the launcher. Unix-only (process-wrap's `JobObject` is the Windows
    // sibling when that platform is addressed; SUPPORTS.md).
    let mut command_wrap = process_wrap::tokio::CommandWrap::from(command_builder);
    #[cfg(unix)]
    command_wrap.wrap(process_wrap::tokio::ProcessGroup::leader());

    let transport = TokioChildProcess::new(command_wrap)
        .map_err(|error| format!("failed to spawn \"{}\": {error}", cfg.label))?;
    ().serve(transport)
        .await
        .map_err(|error| format!("failed to connect to \"{}\": {error}", cfg.label))
}

async fn connect_http(cfg: &McpServerConfig, secret: Option<String>) -> Result<McpClient, String> {
    let url = cfg
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| format!("http connector \"{}\" has no url", cfg.label))?;

    let mut config = StreamableHttpClientTransportConfig::with_uri(url);
    // HTTP secret delivery: the reqwest streamable-HTTP client turns this into an
    // `Authorization: Bearer <secret>` header on every request.
    if let Some(secret) = secret {
        config = config.auth_header(secret);
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    ().serve(transport)
        .await
        .map_err(|error| format!("failed to connect to \"{}\": {error}", cfg.label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_cfg() -> McpServerConfig {
        McpServerConfig {
            id: "connector".to_string(),
            label: "GitHub".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "server-github".to_string()],
            env: Vec::new(),
            url: None,
            secret_env_name: Some("GITHUB_TOKEN".to_string()),
            enabled_tools: None,
        }
    }

    // The fingerprint is the slot-reuse key: a connect-relevant edit must change it
    // (→ the manager drops the old child and redials), while a cosmetic edit must
    // NOT (→ no needless reconnect mid-session).

    #[test]
    fn fingerprint_is_stable_for_cosmetic_edits() {
        let base = config_fingerprint(&stdio_cfg());

        let mut relabeled = stdio_cfg();
        relabeled.label = "Renamed".to_string();
        assert_eq!(config_fingerprint(&relabeled), base, "label rename must not redial");

        let mut recurated = stdio_cfg();
        recurated.enabled_tools = Some(vec!["search".to_string()]);
        assert_eq!(config_fingerprint(&recurated), base, "curation change must not redial");

        // `id` is the slot key, not a connect field; it is not part of the fingerprint.
        let mut reided = stdio_cfg();
        reided.id = "connector-2".to_string();
        assert_eq!(config_fingerprint(&reided), base);
    }

    /// Dropping the stdio transport must kill the whole process GROUP, not just
    /// the launcher: a `sh` launcher forks a `sleep 300` grandchild (pid written
    /// to a pidfile), then execs into a `sleep 60` that never speaks MCP. The
    /// `connect` handshake therefore hangs; abandoning it drops the transport →
    /// rmcp drop-kill → `killpg`. Without the `ProcessGroup::leader()` wrap the
    /// grandchild survives and this test fails its liveness poll.
    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_a_stdio_transport_kills_the_grandchild_too() {
        let pidfile = std::env::temp_dir().join(format!(
            "mnema-mcp-group-kill-{}-{:?}.pid",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&pidfile);

        let mut cfg = stdio_cfg();
        cfg.id = "group-kill-test".to_string();
        cfg.secret_env_name = None;
        cfg.command = Some("sh".to_string());
        cfg.args = vec![
            "-c".to_string(),
            format!("sleep 300 & echo $! > '{}'; exec sleep 60", pidfile.display()),
        ];

        // The handshake never completes (sleep speaks no MCP) — give the pidfile
        // time to appear, then abandon the connect. Dropping the timed-out future
        // drops the transport, which must group-kill launcher + grandchild.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), connect(&cfg)).await;

        let grandchild_pid = std::fs::read_to_string(&pidfile)
            .expect("launcher should have written the grandchild pidfile before the timeout")
            .trim()
            .to_string();
        let _ = std::fs::remove_file(&pidfile);

        // Poll `kill -0` until the grandchild is gone (drop-kill runs on a spawned
        // task, so allow it a bounded moment).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let alive = std::process::Command::new("kill")
                .args(["-0", &grandchild_pid])
                .stderr(std::process::Stdio::null())
                .status()
                .expect("kill -0 should run")
                .success();
            if !alive {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild sleep (pid {grandchild_pid}) survived the transport drop — \
                 process-group kill did not reach it"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    #[test]
    fn fingerprint_changes_for_connect_relevant_edits() {
        let base = config_fingerprint(&stdio_cfg());

        for mutate in [
            |c: &mut McpServerConfig| c.command = Some("node".to_string()),
            |c: &mut McpServerConfig| c.args.push("--flag".to_string()),
            |c: &mut McpServerConfig| c.env.push(capture_types::McpEnvVar {
                name: "A".to_string(),
                value: "b".to_string(),
            }),
            |c: &mut McpServerConfig| c.secret_env_name = Some("OTHER_TOKEN".to_string()),
            |c: &mut McpServerConfig| {
                c.transport = McpTransport::Http;
                c.url = Some("https://mcp.example.com".to_string());
            },
        ] {
            let mut edited = stdio_cfg();
            mutate(&mut edited);
            assert_ne!(
                config_fingerprint(&edited),
                base,
                "a connect-relevant edit must change the fingerprint"
            );
        }
    }
}
