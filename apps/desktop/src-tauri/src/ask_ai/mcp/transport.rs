//! MCP transport construction + per-transport secret delivery (ADR 0048).
//!
//! Builds an initialized client [`McpClient`] for one enabled server. The single
//! optional keychain secret (keyed by instance id) is injected per transport:
//! HTTP → `Authorization: Bearer`; stdio → the env var the server names
//! (`secret_env_name`). Only ENABLED servers ever reach here (the manager
//! filters), and nothing here connects at app launch — the deferred-startup
//! invariant is the manager's concern.

use app_infra::load_mcp_server_secret;
use capture_types::{McpAuthMode, McpServerConfig, McpTransport};
use rmcp::service::RunningService;
use rmcp::transport::auth::{AuthClient, AuthorizationManager};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};

use super::OAuthCredentialStore;

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
        "authMode": cfg.auth_mode,
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
    // The keychain slot is polymorphic (bearer string OR OAuth Token Set). A
    // connector edited from http+OAuth to stdio KEEPS auth_mode=OAuth (auth_mode
    // is ignored for stdio). The settings-save flip-clear keys on the EFFECTIVE
    // auth mode so that edit does clear the slot — but a failed delete or any
    // other path leaving a stale Token Set must still never reach a child:
    // injecting it into the env var would hand the refresh token to that process
    // verbatim. Mirror of connect_http's bearer-path backstop, in the stdio
    // direction — never deliver an OAuth payload as a static secret.
    if secret.as_deref().is_some_and(secret_is_oauth_token_set) {
        return Err(format!(
            "stdio connector \"{}\" holds an OAuth Token Set in its keychain slot; refusing to \
             inject it into the child process environment — remove and re-add the connector (or \
             clear its secret)",
            cfg.label
        ));
    }
    let command = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| format!("stdio connector \"{}\" has no command", cfg.label))?;

    let mut command_builder = tokio::process::Command::new(command);
    command_builder.args(&cfg.args);
    // A packaged macOS app inherits launchd's minimal PATH (no Homebrew/nvm), so
    // a bare `npx` doesn't resolve — and even an ABSOLUTE npx dies, because its
    // `#!/usr/bin/env node` shebang can't find `node` either. Give the child the
    // user's login-shell PATH: Rust resolves the program via the PATH set on the
    // Command, and the shebang's `env` lookup inherits it. A user-provided PATH
    // env row below still overrides this.
    if let Some(path) = tokio::task::spawn_blocking(login_shell_path).await.ok().flatten() {
        command_builder.env("PATH", path);
    }
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

/// The user's login-shell PATH, resolved once per process (the shell invocation
/// costs ~100ms, so it runs on first connect only, off the async runtime via
/// `spawn_blocking`). `"$PATH"` quotes correctly across zsh/bash AND fish — fish
/// treats PATH as a path variable and joins it with colons in quoted expansion.
/// Any failure (no shell, bad exit, empty output) yields None → child inherits
/// the app's PATH unchanged, i.e. the pre-fix behavior.
pub(crate) fn login_shell_path() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let output = std::process::Command::new(shell)
            .args(["-l", "-c", r#"printf '%s' "$PATH""#])
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        let path = String::from_utf8(output.stdout).ok()?;
        let path = path.trim();
        (!path.is_empty()).then(|| path.to_string())
    })
    .as_deref()
}

async fn connect_http(cfg: &McpServerConfig, secret: Option<String>) -> Result<McpClient, String> {
    let url = cfg
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| format!("http connector \"{}\" has no url", cfg.label))?;

    // OAuth connectors carry no bearer secret: the access token is minted from
    // the keychain-stored Token Set by rmcp's AuthorizationManager, never attached
    // as a static header here. `secret` is the shared slot's contents — for an
    // OAuth connector that's the serialized Token Set, irrelevant to this path —
    // so the OAuth branch ignores it.
    if cfg.auth_mode == McpAuthMode::OAuth {
        return connect_http_oauth(cfg, url).await;
    }

    let mut config = StreamableHttpClientTransportConfig::with_uri(url);
    // HTTP secret delivery: the reqwest streamable-HTTP client turns this into an
    // `Authorization: Bearer <secret>` header on every request. A bearer secret
    // must ride TLS only — refuse to attach it to a cleartext remote endpoint,
    // where it would leak to any on-path eavesdropper (loopback is exempt, since
    // that traffic never leaves the machine).
    if let Some(secret) = secret {
        if !secret_may_ride_url(url) {
            return Err(format!(
                "http connector \"{}\" has a secret but its URL is not HTTPS (and not loopback); \
                 refusing to send the secret in cleartext — use an https:// endpoint",
                cfg.label
            ));
        }
        // The keychain slot is polymorphic (bearer string OR OAuth Token Set).
        // After an OAuth→Bearer mode flip that never disconnected, the slot can
        // still hold the Token Set JSON — attaching THAT as a bearer header
        // would send the refresh token to the server verbatim. Mirror of
        // `oauth_token_present`'s parse gate, in the other direction.
        if secret_is_oauth_token_set(&secret) {
            return Err(format!(
                "connector \"{}\" is set to Bearer auth but its keychain slot holds an OAuth \
                 Token Set; refusing to send it as a bearer header — enter a bearer secret \
                 (or switch the connector back to OAuth)",
                cfg.label
            ));
        }
        config = config.auth_header(secret);
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    ().serve(transport)
        .await
        .map_err(|error| format!("failed to connect to \"{}\": {error}", cfg.label))
}

/// Connect an OAuth HTTP connector (`auth_mode == OAuth`). The access token is
/// minted and silently refreshed by rmcp's [`AuthorizationManager`] from the
/// Token Set persisted in this connector's keychain slot (via
/// [`OAuthCredentialStore`]); [`AuthClient`] injects it as the bearer token on
/// every request. No static bearer secret is involved — this path never touches
/// the `Authorization` header itself, and the shared slot here holds the Token
/// Set, not a token string.
///
/// Two "not connectable yet" states surface as readable errors instead of opening
/// a browser — starting the authorize flow is a Settings action (ADR 0051), never
/// a side effect of a chat turn:
///   - no Token Set stored → *Needs authorization* ("click Connect in Settings");
///   - a stored Token Set whose refresh token is dead → *Needs reconnect*.
async fn connect_http_oauth(cfg: &McpServerConfig, url: &str) -> Result<McpClient, String> {
    // Discovery, token exchange, and refresh ALL ride this base URL, so the same
    // TLS-or-loopback guard the bearer secret gets must gate it too — refuse to
    // run the OAuth flow against a cleartext remote endpoint before building it.
    if !secret_may_ride_url(url) {
        return Err(format!(
            "oauth connector \"{}\" has a non-HTTPS (and non-loopback) URL; refusing to run \
             the OAuth flow in cleartext — use an https:// endpoint",
            cfg.label
        ));
    }

    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|error| format!("failed to prepare OAuth for \"{}\": {error}", cfg.label))?;
    manager.set_credential_store(OAuthCredentialStore::new(cfg.id.clone()));

    // Discovery is server-supplied and re-run every connect: the token/refresh
    // endpoint can point ELSEWHERE than the (TLS-guarded) base URL, and rmcp does
    // not enforce https on it for this flow. Pre-discover and gate every endpoint
    // through the same guard BEFORE the refresh below rides it — pre-setting the
    // metadata means `initialize_from_store` reuses it (no second round-trip).
    let md = manager.discover_metadata().await.map_err(|error| {
        format!("failed to load OAuth metadata for \"{}\": {error}", cfg.label)
    })?;
    super::oauth_flow::discovered_endpoints_secure(&md)
        .map_err(|reason| format!("oauth connector \"{}\": {reason}", cfg.label))?;
    manager.set_metadata(md);

    // Warm from the keychain-stored Token Set. `false` = nothing persisted → this
    // connector was never authorized (or was disconnected); do NOT start an
    // interactive authorize here — that's the Settings "Connect" button's job.
    let loaded = manager.initialize_from_store().await.map_err(|error| {
        format!("failed to load OAuth credentials for \"{}\": {error}", cfg.label)
    })?;
    if !loaded {
        return Err(format!(
            "connector \"{}\" is not authorized yet — click Connect in Settings",
            cfg.label
        ));
    }

    // Force a mint/refresh up front so a dead refresh token surfaces here as a
    // readable "needs reconnect" rather than an opaque mid-turn tool failure.
    manager.get_access_token().await.map_err(|error| {
        format!(
            "connector \"{}\" needs reconnecting in Settings (its authorization expired): {error}",
            cfg.label
        )
    })?;

    // AuthClient wraps a rustls reqwest client: on each request it calls
    // get_access_token (auto-refreshing) and injects the bearer token, so the
    // handshake below is identical to the bearer path from here on.
    // ponytail: aliased reqwest 0.13 — rmcp's `StreamableHttpClient` is impl'd for
    // ITS reqwest (0.13), not the workspace 0.12 `reqwest::Client` used elsewhere;
    // reqwest 0.13 defaults to rustls, so this client is native-tls-free.
    let auth_client = AuthClient::new(reqwest13::Client::new(), manager);
    let config = StreamableHttpClientTransportConfig::with_uri(url);
    let transport = StreamableHttpClientTransport::with_client(auth_client, config);
    ().serve(transport)
        .await
        .map_err(|error| format!("failed to connect to \"{}\": {error}", cfg.label))
}

/// Whether a keychain-slot payload is a serialized OAuth Token Set rather than a
/// plain bearer secret. The slot is polymorphic; this is the bearer-attach
/// counterpart of `oauth_flow::oauth_token_present`'s parse gate — each mode
/// refuses the other mode's payload, so a Bearer↔OAuth flip that skipped
/// Disconnect can never leak the leftover payload down the wrong path.
// ponytail: a genuine bearer secret that happens to parse as StoredCredentials
// JSON is not a real case (same accepted ceiling as `oauth_token_present`).
fn secret_is_oauth_token_set(secret: &str) -> bool {
    serde_json::from_str::<rmcp::transport::auth::StoredCredentials>(secret).is_ok()
}

/// Whether the connector's bearer secret may be attached to a request to `raw`.
/// A secret rides TLS only, EXCEPT for a loopback endpoint (a local MCP server on
/// this machine, where cleartext never leaves the host). An unparseable URL is
/// treated as unsafe (the secret is withheld and the connect refused).
///
/// Reused by the OAuth flow in `manager` (discovery/registration/token/refresh
/// and the revocation endpoint all ride the same TLS-or-loopback rule), so it is
/// `pub(crate)` — one guard, not two.
pub(crate) fn secret_may_ride_url(raw: &str) -> bool {
    match url::Url::parse(raw) {
        Ok(parsed) => parsed.scheme() == "https" || url_host_is_loopback(&parsed),
        Err(_) => false,
    }
}

/// A host is loopback if it is `localhost` or a loopback IP (`127.0.0.0/8`, `::1`).
fn url_host_is_loopback(parsed: &url::Url) -> bool {
    match parsed.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
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
            auth_mode: capture_types::McpAuthMode::Bearer,
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

    /// The login-shell PATH resolves and is a plausible PATH string. If the shell
    /// invocation breaks (bad flags, fish quoting regression), this returns None
    /// or junk and stdio connectors silently regress to the minimal launchd PATH.
    ///
    /// macOS-only: the login-shell PATH mechanism is Unix (`$SHELL -l -c`) and
    /// SUPPORTS.md marks Windows unaddressed for it.
    #[cfg(target_os = "macos")]
    #[test]
    fn login_shell_path_resolves() {
        let path = login_shell_path().expect("login shell should yield a PATH");
        assert!(
            path.split(':').any(|dir| std::path::Path::new(dir).is_dir()),
            "PATH should contain at least one existing directory: {path}"
        );
    }

    /// The polymorphic-slot backstop, stdio side (ADR 0051 deferred finding). A
    /// connector edited from http+OAuth to stdio KEEPS `auth_mode=OAuth` (the
    /// frontend passes authMode verbatim, and auth_mode is ignored for stdio, so
    /// the settings-save flip-clear — keyed on the auth mode — never fires). Its
    /// keychain slot still holds the serialized OAuth Token Set. `connect_stdio`
    /// must REFUSE to inject that JSON into the child process env: it carries the
    /// refresh token, and a mismatched payload must never ride the wrong path.
    #[tokio::test]
    async fn a_stale_oauth_token_set_never_rides_into_the_child_env() {
        let mut cfg = stdio_cfg();
        cfg.auth_mode = McpAuthMode::OAuth;
        let token_set = serde_json::to_string(&rmcp::transport::auth::StoredCredentials::new(
            "client-xyz".to_string(),
            None,
            Vec::new(),
            None,
        ))
        .expect("serialize token set");

        // The backstop fires BEFORE resolving the command / spawning the child,
        // so this resolves fast with the refusal — in the unfixed state it would
        // inject the JSON into GITHUB_TOKEN and spawn.
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connect_stdio(&cfg, Some(token_set)),
        )
        .await;
        let error = outcome
            .expect("the stdio Token-Set backstop must refuse before spawning, not hang")
            .expect_err("a Token Set payload must never be injected into the child env");
        assert!(
            error.contains("Token Set"),
            "expected the Token-Set refusal, got: {error}"
        );

        // A plain opaque bearer secret still passes the gate (delivered to stdio).
        assert!(!secret_is_oauth_token_set("ghp_opaque_stdio_secret"));
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
            // Flipping the auth mode redials: bearer and OAuth are entirely
            // different transports (static header vs. token-minting AuthClient).
            |c: &mut McpServerConfig| c.auth_mode = McpAuthMode::OAuth,
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

#[cfg(test)]
mod http_secret_scheme_review_security_b {
    use super::*;

    fn http_cfg(url: &str) -> McpServerConfig {
        McpServerConfig {
            id: "http-connector".to_string(),
            label: "Remote".to_string(),
            enabled: true,
            transport: McpTransport::Http,
            auth_mode: capture_types::McpAuthMode::Bearer,
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            url: Some(url.to_string()),
            secret_env_name: None,
            enabled_tools: None,
        }
    }

    /// A remote (non-loopback) `http://` endpoint must NEVER receive the bearer
    /// secret: attaching `Authorization: Bearer <secret>` to a cleartext request
    /// ships the keychain token to any on-path eavesdropper. `connect_http` must
    /// REFUSE before dialing rather than send it.
    ///
    /// Uses RFC 5737 TEST-NET-1 (192.0.2.1, guaranteed unroutable) so that in the
    /// unfixed (vulnerable) state the connect merely hangs against a dead address
    /// — it never actually transmits the secret from the test.
    #[tokio::test]
    async fn a_remote_http_url_refuses_to_send_the_secret_in_cleartext() {
        let cfg = http_cfg("http://192.0.2.1/");
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connect_http(&cfg, Some("bearer-secret".to_string())),
        )
        .await;
        let error = outcome
            .expect("connect_http must refuse a cleartext secret fast, not hang dialing")
            .expect_err("a cleartext remote URL carrying a secret must be refused");
        assert!(
            error.contains("cleartext") || error.to_lowercase().contains("https"),
            "expected a cleartext/https refusal, got: {error}"
        );
    }

    /// An OAuth connector's discovery/token/refresh calls all ride the base URL,
    /// so a cleartext non-loopback endpoint must be refused BEFORE the OAuth flow
    /// starts — the same guard the bearer secret gets. RFC 5737 TEST-NET-1 again,
    /// so nothing is transmitted even in the unfixed state.
    #[tokio::test]
    async fn an_oauth_connector_refuses_a_cleartext_remote_endpoint() {
        let mut cfg = http_cfg("http://192.0.2.1/");
        cfg.auth_mode = McpAuthMode::OAuth;
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            // The bearer `secret` is ignored on the OAuth path; the guard fires
            // before any network work, so this returns fast rather than dialing.
            connect_http(&cfg, None),
        )
        .await;
        let error = outcome
            .expect("connect_http must refuse a cleartext OAuth endpoint fast, not hang dialing")
            .expect_err("an OAuth flow over a cleartext remote URL must be refused");
        assert!(
            error.contains("cleartext") || error.to_lowercase().contains("https"),
            "expected a cleartext/https refusal, got: {error}"
        );
    }

    /// The polymorphic-slot backstop (ADR 0051 deferred finding): a connector
    /// flipped OAuth→Bearer without disconnecting still holds the serialized
    /// OAuth Token Set in its keychain slot. The bearer path must REFUSE to
    /// attach that JSON as `Authorization: Bearer …` — it carries the refresh
    /// token (and client id) verbatim. Mirror of the read-side gate that stops
    /// a stale bearer secret reading as OAuth-authorized.
    #[tokio::test]
    async fn a_stale_oauth_token_set_never_rides_as_a_bearer_header() {
        let mut cfg = http_cfg("https://mcp.example.com/");
        cfg.auth_mode = McpAuthMode::Bearer;
        let token_set = serde_json::to_string(&rmcp::transport::auth::StoredCredentials::new(
            "client-xyz".to_string(),
            None,
            Vec::new(),
            None,
        ))
        .expect("serialize token set");

        // The backstop fires BEFORE dialing, so this resolves fast with the
        // refusal — in the unfixed state it would attach the JSON and dial.
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connect_http(&cfg, Some(token_set)),
        )
        .await;
        let error = outcome
            .expect("the Token-Set backstop must refuse before dialing, not hang")
            .expect_err("a Token Set payload must never be attached as a bearer header");
        assert!(
            error.contains("Token Set"),
            "expected the Token-Set refusal, got: {error}"
        );

        // A plain opaque bearer secret passes the gate — and so does a
        // JSON-shaped one that is not a Token Set: the gate keys on the
        // StoredCredentials SHAPE (client_id required), not on "is this JSON",
        // so a real JSON bearer secret is never refused.
        assert!(!secret_is_oauth_token_set("lin_bearer_secret_abc"));
        assert!(!secret_is_oauth_token_set("{}"));
        assert!(!secret_is_oauth_token_set("{\"foo\":1}"));
    }

    /// The scheme guard: TLS and loopback may carry the secret; a remote
    /// cleartext endpoint (or a junk URL) may not.
    #[test]
    fn secret_only_rides_tls_or_loopback() {
        // Allowed: TLS to anywhere, cleartext only to loopback.
        assert!(secret_may_ride_url("https://mcp.example.com/"));
        assert!(secret_may_ride_url("http://127.0.0.1:8080/"));
        assert!(secret_may_ride_url("http://localhost:3000/mcp"));
        assert!(secret_may_ride_url("http://[::1]:9/"));
        // Denied: cleartext to a remote host, or an unparseable URL.
        assert!(!secret_may_ride_url("http://mcp.example.com/"));
        assert!(!secret_may_ride_url("http://10.0.0.5/"));
        assert!(!secret_may_ride_url("ftp://mcp.example.com/"));
        assert!(!secret_may_ride_url("not a url"));
    }
}
