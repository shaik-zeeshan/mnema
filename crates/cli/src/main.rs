#[cfg(unix)]
use std::time::Duration;
use std::{env, io::IsTerminal, path::PathBuf, process::ExitCode};

use app_infra::brokered_access::{
    BrokerAuthStatus, BrokerAuthStatusKind, BrokerClientIdentity, BrokerClientIdentitySource,
    BrokerErrorResponse, BrokerSearchRequest, BrokerSpeaker, BrokerSpeakerCoverage,
    BrokerSpeakerTurn, BrokerSpeakersRequest, BrokerTimelineRequest, BrokeredCaptureAccess,
    BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::process::Command;
#[cfg(unix)]
use tokio::time::timeout;
#[cfg(unix)]
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use uuid::Uuid;

mod mcp;

const APP_IDENTIFIER: &str = env!("MNEMA_APP_IDENTIFIER");
#[cfg(unix)]
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(120);
const BROKER_AUTHORIZATION_REQUEST_FILE_NAME: &str = "broker-authorization-request.json";
const INFERRED_AGENT_ENV_LABELS: &[(&str, &str)] = &[
    ("CLAUDECODE", "Claude Code"),
    ("CLAUDE_CODE", "Claude Code"),
    ("CURSOR_TRACE_ID", "Cursor"),
    ("CODEX_CI", "Codex"),
    ("CODEX_MANAGED_BY_BUN", "Codex"),
    ("CODEX_MANAGED_PACKAGE_ROOT", "Codex"),
    ("CODEX_SANDBOX", "Codex"),
    ("CODEX_THREAD_ID", "Codex"),
    ("OPENCODE", "OpenCode"),
    ("OPENCODE_PID", "OpenCode"),
    ("PI_CODING_AGENT", "PI"),
];

#[derive(Parser, Debug)]
#[command(name = "mnema", version)]
struct Cli {
    #[arg(long, global = true)]
    client: Option<String>,
    #[arg(long, global = true, value_enum)]
    format: Option<OutputFormat>,
    #[arg(long, global = true)]
    no_prompt: bool,
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand, Debug)]
enum CommandKind {
    Search(SearchArgs),
    Timeline(TimelineArgs),
    /// List the people and voices heard inside the active grant's time scope,
    /// longest-speaking first, each with the handle `--speaker` takes.
    Speakers(SpeakersArgs),
    ShowText {
        opaque_result_id: String,
    },
    Open {
        opaque_result_id: String,
    },
    Access {
        #[command(subcommand)]
        command: AccessCommand,
    },
    /// Run as a local MCP server over stdio (for Claude Desktop, Cursor, etc.)
    Mcp,
}

#[derive(Subcommand, Debug)]
enum AccessCommand {
    Status {
        #[arg(long)]
        all_clients: bool,
    },
    Request {
        #[arg(long, value_enum, default_value = "last-day")]
        scope: AccessScope,
        #[arg(long, value_enum, default_value = "24h")]
        duration: AccessDuration,
    },
    KnownClients,
    Revoke {
        grant_id: String,
    },
    RevokeClient {
        client_name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args, Debug)]
struct SearchArgs {
    #[arg(long)]
    query: String,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    to: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
    #[arg(long)]
    app: Option<String>,
    #[arg(long)]
    window_title: Option<String>,
    /// Case-insensitive substring of the page URL, matched against the sanitized
    /// host/path form (query strings and fragments are never indexed).
    #[arg(long)]
    url: Option<String>,
    /// Case-sensitive regular expression over the same sanitized host/path URL
    /// (prefix with `(?i)` for case-insensitive matching).
    #[arg(long, conflicts_with = "url")]
    url_regex: Option<String>,
    /// Opaque speaker handle from `mnema speakers` (or from any `show-text`
    /// speaker), narrowing results to audio that person or voice was heard in.
    /// Matches voices the user assigned AND voices recognition only guessed at.
    #[arg(long, conflicts_with_all = ["app", "window_title", "url", "url_regex"])]
    speaker: Option<String>,
    /// `nextCursor` from a previous search response, to fetch the next page.
    /// Re-send the identical query and filters alongside it.
    #[arg(long)]
    cursor: Option<String>,
}

#[derive(Args, Debug)]
struct TimelineArgs {
    #[arg(long)]
    from: String,
    #[arg(long)]
    to: String,
    #[arg(long)]
    limit: Option<u32>,
    #[arg(long)]
    app: Option<String>,
    #[arg(long)]
    window_title: Option<String>,
    /// Case-insensitive substring of the page URL, matched against the sanitized
    /// host/path form (query strings and fragments are never indexed).
    #[arg(long)]
    url: Option<String>,
    /// Case-sensitive regular expression over the same sanitized host/path URL
    /// (prefix with `(?i)` for case-insensitive matching).
    #[arg(long, conflicts_with = "url")]
    url_regex: Option<String>,
    /// Opaque speaker handle from `mnema speakers` (or from any `show-text`
    /// speaker), narrowing the window to audio that person or voice was heard in.
    /// Matches voices the user assigned AND voices recognition only guessed at.
    #[arg(long, conflicts_with_all = ["app", "window_title", "url", "url_regex"])]
    speaker: Option<String>,
}

#[derive(Args, Debug)]
struct SpeakersArgs {
    /// Case-insensitive substring of a person's name. Named people only — it is
    /// how a quiet person who ranks below the limit is still findable.
    #[arg(long)]
    name: Option<String>,
    /// Maximum number of speakers to return.
    #[arg(long)]
    limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Json,
    Yaml,
    Toon,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AccessScope {
    LastDay,
    AllRetained,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize)]
enum AccessDuration {
    #[value(name = "1h")]
    OneHour,
    #[value(name = "24h")]
    TwentyFourHours,
    #[value(name = "7d")]
    SevenDays,
}

impl AccessDuration {
    fn seconds(self) -> u64 {
        match self {
            Self::OneHour => 60 * 60,
            Self::TwentyFourHours => 24 * 60 * 60,
            Self::SevenDays => 7 * 24 * 60 * 60,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<T: Serialize> {
    schema_version: u32,
    command: String,
    client: ClientEnvelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorEnvelope>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientEnvelope {
    label: String,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    code: &'static str,
    message: String,
    retryable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchData {
    results: Vec<SearchResultData>,
    /// Effective (server-clamped) limit actually applied — not necessarily the
    /// `--limit` the caller asked for.
    limit: u32,
    /// Cursor for the next page, or absent when this page exhausted the matches.
    /// Pass it back as `--cursor` with the same query and filters.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
    /// Only on a `--speaker` search: how much audio the filter could NOT check.
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker_coverage: Option<BrokerSpeakerCoverage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResultData {
    id: String,
    kind: String,
    snippet: String,
    started_at: String,
    ended_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<SearchResultContextData>,
    /// What the `--speaker` said in this recording. Present only on a filtered
    /// search, and only that speaker's words. ABSENT IS NEVER SILENCE — see
    /// [`ShowTextData::turns`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    turns: Vec<BrokerSpeakerTurn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResultContextData {
    #[serde(skip_serializing_if = "Option::is_none")]
    app_bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineData {
    intervals: Vec<TimelineIntervalData>,
    /// Effective (server-clamped) limit actually applied.
    limit: u32,
    /// True when the intervals filled the effective limit — see [`SearchData`].
    truncated: bool,
    /// Only on a `--speaker` timeline: how much audio the filter could NOT check.
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker_coverage: Option<BrokerSpeakerCoverage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineIntervalData {
    kind: String,
    started_at: String,
    ended_at: String,
    /// Followable capture id — pass it to `show-text`. Absent when the interval has
    /// no representative capture to point at.
    #[serde(skip_serializing_if = "Option::is_none")]
    opaque_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<SearchResultContextData>,
    /// What the `--speaker` said in this interval — same rules as
    /// [`SearchResultData::turns`], including that absence is never silence.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    turns: Vec<BrokerSpeakerTurn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowTextData {
    id: String,
    kind: String,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    speakers: Vec<BrokerSpeaker>,
    /// Who said which words, each turn indexing into `speakers`. ABSENT MEANS
    /// "COULD NOT ATTRIBUTE", never "nobody spoke" — `text` still has the words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    turns: Vec<BrokerSpeakerTurn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenData {
    id: String,
    opened: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationRequest {
    schema_version: u32,
    request_id: String,
    client: AuthorizationClient,
    command: String,
    scope: AuthorizationScope,
    duration: AuthorizationDuration,
    interactive: bool,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationClient {
    label: String,
    source: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationScope {
    minimum: String,
    preferred: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationDuration {
    minimum_seconds: u64,
    preferred_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationResponse {
    schema_version: u32,
    request_id: String,
    decision: String,
    reason: Option<String>,
}

#[derive(Debug)]
struct CliError {
    exit: u8,
    code: &'static str,
    message: String,
    retryable: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.message);
            ExitCode::from(error.exit)
        }
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    let identity = resolve_identity(cli.client.as_deref())?;
    match cli.command {
        CommandKind::Search(args) => {
            let request = BrokeredCaptureRequest::Search(BrokerSearchRequest {
                query: args.query,
                from: args.from,
                to: args.to,
                limit: args.limit,
                app: args.app,
                window_title: args.window_title,
                url: args.url,
                url_regex: args.url_regex,
                cursor: args.cursor,
                speaker: args.speaker,
            });
            run_data_command("search", &identity, request, cli.format, cli.no_prompt).await
        }
        CommandKind::Timeline(args) => {
            let request = BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
                from: args.from,
                to: args.to,
                limit: args.limit,
                app: args.app,
                window_title: args.window_title,
                url: args.url,
                url_regex: args.url_regex,
                speaker: args.speaker,
            });
            run_data_command("timeline", &identity, request, cli.format, cli.no_prompt).await
        }
        CommandKind::Speakers(args) => {
            let request = BrokeredCaptureRequest::Speakers(BrokerSpeakersRequest {
                name: args.name,
                limit: args.limit,
            });
            run_data_command("speakers", &identity, request, cli.format, cli.no_prompt).await
        }
        CommandKind::ShowText { opaque_result_id } => {
            run_data_command(
                "show-text",
                &identity,
                BrokeredCaptureRequest::ShowText {
                    opaque_id: opaque_result_id,
                },
                cli.format,
                cli.no_prompt,
            )
            .await
        }
        CommandKind::Open { opaque_result_id } => {
            run_data_command(
                "open",
                &identity,
                BrokeredCaptureRequest::OpenInMnema {
                    opaque_id: opaque_result_id,
                },
                cli.format,
                cli.no_prompt,
            )
            .await
        }
        CommandKind::Access { command } => {
            if cli.format.is_some() {
                return Err(usage_error("--format is only supported for data commands"));
            }
            run_access_command(command, &identity, cli.no_prompt).await
        }
        CommandKind::Mcp => {
            if cli.format.is_some() {
                return Err(usage_error("--format is only supported for data commands"));
            }
            mcp::serve(identity).await
        }
    }
}

async fn run_data_command(
    command: &str,
    identity: &BrokerClientIdentity,
    request: BrokeredCaptureRequest,
    format: Option<OutputFormat>,
    no_prompt: bool,
) -> Result<(), CliError> {
    let format = format.unwrap_or(OutputFormat::Json);
    let allow_prompt = !no_prompt && can_prompt_for_authorization();
    match execute_data_request(command, identity, request, allow_prompt).await {
        Ok(value) => print_envelope(command, identity, format, &value),
        Err(error) => print_structured_error(command, identity, format, error),
    }
}

/// Shared broker path for the CLI data commands and the MCP server: execute,
/// run the app-approval flow once if the broker asks for it, and map the
/// response to the stable output shape.
async fn execute_data_request(
    command: &str,
    identity: &BrokerClientIdentity,
    request: BrokeredCaptureRequest,
    allow_prompt: bool,
) -> Result<serde_json::Value, CliError> {
    let access =
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER).map_err(broker_error)?;
    let mut response = access
        .execute_for_identity(identity.clone(), request.clone())
        .await
        .map_err(broker_error)?;

    if response_requires_authorization(&response) {
        if !allow_prompt {
            return Err(auth_required_error());
        }
        request_authorization(
            command,
            identity,
            AccessScope::LastDay,
            AccessDuration::TwentyFourHours,
        )
        .await?;
        response = access
            .execute_for_identity(identity.clone(), request)
            .await
            .map_err(broker_error)?;
    }

    let value = match response {
        BrokeredCaptureResponse::Error(error) => return Err(map_broker_response_error(error)),
        BrokeredCaptureResponse::Search(response) if command == "search" => {
            serde_json::to_value(map_search_data(response))
        }
        BrokeredCaptureResponse::Timeline(response) if command == "timeline" => {
            serde_json::to_value(map_timeline_data(response))
        }
        // Serialized verbatim: the broker response is already the CLI shape
        // (speakers/limit/truncated), and re-declaring it here is how a handle or
        // a turn count would silently go missing.
        BrokeredCaptureResponse::Speakers(response) if command == "speakers" => {
            serde_json::to_value(response)
        }
        BrokeredCaptureResponse::ShowText(response) if command == "show-text" => {
            serde_json::to_value(ShowTextData {
                id: response.opaque_id,
                kind: response.kind,
                text: response.text,
                speakers: response.speakers,
                turns: response.turns,
            })
        }
        BrokeredCaptureResponse::OpenInMnema(response) if command == "open" => {
            serde_json::to_value(OpenData {
                id: response.opaque_id,
                opened: response.opened,
            })
        }
        _ => return Err(broker_failure(format!("unexpected {command} response"))),
    };
    value.map_err(|error| CliError {
        exit: 21,
        code: "output_serialization_failed",
        message: error.to_string(),
        retryable: false,
    })
}

async fn run_access_command(
    command: AccessCommand,
    identity: &BrokerClientIdentity,
    no_prompt: bool,
) -> Result<(), CliError> {
    let access =
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER).map_err(broker_error)?;
    match command {
        AccessCommand::Status { all_clients } => {
            let grants = access.list_grants().map_err(broker_error)?;
            let active = grants
                .grants
                .iter()
                .filter(|grant| !grant.revoked && grant.expires_at_unix_ms > now_unix_ms())
                .filter(|grant| all_clients || grant.normalized_label == identity.normalized_label)
                .count();
            println!(
                "Client: {} ({})",
                identity.label,
                identity_source_name(&identity.source)
            );
            println!(
                "CLI Access: {active} active grant(s){}",
                if all_clients { "" } else { " for this client" }
            );
            Ok(())
        }
        AccessCommand::Request { scope, duration } => {
            if !can_start_explicit_authorization_request(no_prompt) {
                return Err(auth_required_error());
            }
            request_authorization("access request", identity, scope, duration).await?;
            println!("CLI Access request approved or queued. Run `mnema access status` to inspect grants.");
            Ok(())
        }
        AccessCommand::KnownClients => {
            println!("Auto-detected clients:");
            for label in inferred_agent_labels() {
                println!("- {label}");
            }
            println!("Use --client <name> or MNEMA_CLI_CLIENT for unlisted clients.");
            Ok(())
        }
        AccessCommand::Revoke { grant_id } => {
            let revoked = access.revoke_grant(&grant_id).map_err(broker_error)?;
            println!(
                "{}",
                if revoked {
                    "Grant revoked."
                } else {
                    "Grant not found or already inactive."
                }
            );
            Ok(())
        }
        AccessCommand::RevokeClient { client_name, yes } => {
            if !yes {
                return Err(usage_error("revoke-client requires --yes"));
            }
            let count = access
                .revoke_grants_for_client(&client_name)
                .map_err(broker_error)?;
            println!("Revoked {count} grant(s).");
            Ok(())
        }
    }
}

fn can_prompt_for_authorization() -> bool {
    can_prompt_for_authorization_with(
        || std::io::stdin().is_terminal(),
        || std::io::stdout().is_terminal(),
        || std::io::stderr().is_terminal(),
    )
}

fn can_prompt_for_authorization_with(
    stdin_is_terminal: impl FnOnce() -> bool,
    stdout_is_terminal: impl FnOnce() -> bool,
    stderr_is_terminal: impl FnOnce() -> bool,
) -> bool {
    stdin_is_terminal() && stdout_is_terminal() && stderr_is_terminal()
}

fn can_start_explicit_authorization_request(no_prompt: bool) -> bool {
    !no_prompt
}

async fn request_authorization(
    command: &str,
    identity: &BrokerClientIdentity,
    scope: AccessScope,
    duration: AccessDuration,
) -> Result<(), CliError> {
    eprintln!("CLI Access approval required. Opening Mnema...");
    let request = AuthorizationRequest {
        schema_version: 1,
        request_id: Uuid::new_v4().to_string(),
        client: AuthorizationClient {
            label: identity.label.clone(),
            source: identity_source_name(&identity.source).to_string(),
        },
        command: command.to_string(),
        scope: AuthorizationScope {
            minimum: "lastDay".to_string(),
            preferred: match scope {
                AccessScope::LastDay => "lastDay",
                AccessScope::AllRetained => "allRetained",
            }
            .to_string(),
        },
        duration: AuthorizationDuration {
            minimum_seconds: 3600,
            preferred_seconds: duration.seconds(),
        },
        interactive: true,
        created_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
    };
    match send_authorization_request(&request).await {
        Ok(()) => Ok(()),
        Err(first_error) if should_retry_authorization_with_app_launch(&first_error) => {
            let _ = launch_mnema_app().await;
            let _ = write_legacy_wake_request();
            authorization_retry_result(first_error, send_authorization_request(&request).await)
        }
        Err(first_error) => Err(first_error),
    }
}

fn authorization_retry_result(
    _first_error: CliError,
    retry_result: Result<(), CliError>,
) -> Result<(), CliError> {
    retry_result
}

#[cfg(unix)]
async fn send_authorization_request(request: &AuthorizationRequest) -> Result<(), CliError> {
    let socket_path = authorization_socket_path();
    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
        .await
        .map_err(|_| app_unavailable_error())?
        .map_err(|_| app_unavailable_error())?;
    let raw = serde_json::to_string(request).map_err(|error| CliError {
        exit: 21,
        code: "output_serialization_failed",
        message: error.to_string(),
        retryable: false,
    })?;
    stream
        .write_all(format!("{raw}\n").as_bytes())
        .await
        .map_err(|_| app_unavailable_error())?;
    let mut response = String::new();
    timeout(
        AUTHORIZATION_TIMEOUT,
        BufReader::new(stream).read_line(&mut response),
    )
    .await
    .map_err(|_| timeout_error())?
    .map_err(|_| app_unavailable_error())?;
    let response: AuthorizationResponse =
        serde_json::from_str(&response).map_err(|_| app_unavailable_error())?;
    if response.request_id != request.request_id {
        return Err(app_unavailable_error());
    }
    match response.decision.as_str() {
        "approved" => Ok(()),
        "denied" => Err(authorization_denied_error()),
        "unavailable" => Err(app_unavailable_error()),
        _ => Err(app_unavailable_error()),
    }
}

#[cfg(not(unix))]
async fn send_authorization_request(_request: &AuthorizationRequest) -> Result<(), CliError> {
    Err(app_unavailable_error())
}

fn should_retry_authorization_with_app_launch(error: &CliError) -> bool {
    error.code == "app_unavailable"
}

#[cfg(unix)]
fn authorization_socket_path() -> PathBuf {
    default_app_config_dir()
        .unwrap_or_else(|| env::temp_dir().join(APP_IDENTIFIER))
        .join("cli-access.sock")
}

async fn launch_mnema_app() -> Result<(), CliError> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open")
        .args(["-b", APP_IDENTIFIER])
        .status()
        .await;
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", "", "mnema"])
        .status()
        .await;
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg("mnema").status().await;

    status
        .ok()
        .filter(|status| status.success())
        .map(|_| ())
        .ok_or_else(app_unavailable_error)
}

fn write_legacy_wake_request() -> Result<(), CliError> {
    let config_dir = default_app_config_dir().ok_or_else(app_unavailable_error)?;
    std::fs::create_dir_all(&config_dir).map_err(|_| app_unavailable_error())?;
    std::fs::write(
        config_dir.join(BROKER_AUTHORIZATION_REQUEST_FILE_NAME),
        r#"{"route":"/access/request","settingsTab":"access","focus":"cliAccess"}"#,
    )
    .map_err(|_| app_unavailable_error())
}

fn default_app_config_dir() -> Option<PathBuf> {
    if let Ok(path) = env::var("MNEMA_APP_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(APP_IDENTIFIER)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::config_dir().map(|dir| dir.join(APP_IDENTIFIER))
    }
}

fn resolve_identity(explicit: Option<&str>) -> Result<BrokerClientIdentity, CliError> {
    resolve_identity_from_env(
        explicit,
        |key| env::var(key).ok(),
        |key| env::var_os(key).is_some(),
    )
}

// Identity precedence, highest first:
//   1. --client flag                  (explicit caller intent)
//   2. MNEMA_CLI_CLIENT env value      (deliberate mnema-specific override)
//   3. curated known-agent detection   (stable, version-free catalog label)
//   4. AI_AGENT env value              (generic fallback for unrecognized agents)
//   5. default CLI identity
//
// Curated detection deliberately outranks AI_AGENT: it keys off the env *key*
// presence (e.g. CLAUDECODE) and yields a stable label, whereas AI_AGENT carries
// a value that may embed a version (e.g. `claude-code_2-1-152_agent`). Matching on
// the versioned value would force a fresh broker grant on every release, so a known
// agent collapses to its catalog label and reuses one grant across versions.
fn resolve_identity_from_env(
    explicit: Option<&str>,
    env_value: impl Fn(&str) -> Option<String>,
    env_has_key: impl Fn(&str) -> bool,
) -> Result<BrokerClientIdentity, CliError> {
    if let Some(value) = explicit {
        return BrokerClientIdentity::new(value, BrokerClientIdentitySource::Explicit)
            .map_err(|_| usage_error("--client must contain a visible client name"));
    }
    if let Some(value) = env_value("MNEMA_CLI_CLIENT") {
        if let Ok(identity) = BrokerClientIdentity::new(value, BrokerClientIdentitySource::Env) {
            return Ok(identity);
        }
    }
    if let Some(label) = inferred_agent_label_from_env(&env_has_key) {
        return BrokerClientIdentity::new(label, BrokerClientIdentitySource::Inferred)
            .map_err(broker_error);
    }
    if let Some(value) = env_value("AI_AGENT") {
        if let Ok(identity) = BrokerClientIdentity::new(value, BrokerClientIdentitySource::Env) {
            return Ok(identity);
        }
    }
    Ok(BrokerClientIdentity::default_cli())
}

fn inferred_agent_label_from_env(env_has_key: impl Fn(&str) -> bool) -> Option<&'static str> {
    INFERRED_AGENT_ENV_LABELS
        .iter()
        .find(|(key, _)| env_has_key(key))
        .map(|(_, label)| *label)
}

fn inferred_agent_labels() -> Vec<&'static str> {
    let mut labels = Vec::new();
    for (_, label) in INFERRED_AGENT_ENV_LABELS {
        if !labels.contains(label) {
            labels.push(*label);
        }
    }
    labels
}

fn print_envelope<T: Serialize>(
    command: &str,
    identity: &BrokerClientIdentity,
    format: OutputFormat,
    data: &T,
) -> Result<(), CliError> {
    let envelope = Envelope {
        schema_version: 1,
        command: command.to_string(),
        client: client_envelope(identity),
        data: Some(data),
        error: None,
    };
    print_serialized(&envelope, format)
}

fn print_structured_error(
    command: &str,
    identity: &BrokerClientIdentity,
    format: OutputFormat,
    error: CliError,
) -> Result<(), CliError> {
    let envelope = Envelope::<()> {
        schema_version: 1,
        command: command.to_string(),
        client: client_envelope(identity),
        data: None,
        error: Some(ErrorEnvelope {
            code: error.code,
            message: error.message.clone(),
            retryable: error.retryable,
        }),
    };
    print_serialized(&envelope, format)?;
    Err(error)
}

fn print_serialized<T: Serialize>(value: &T, format: OutputFormat) -> Result<(), CliError> {
    let raw = match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(value).map_err(|error| error.to_string())
        }
        OutputFormat::Yaml => yaml_serde::to_string(value).map_err(|error| error.to_string()),
        OutputFormat::Toon => toon_rs::ser::to_string_streaming(value, &Default::default())
            .map_err(|error| error.to_string()),
    }
    .map_err(|error| CliError {
        exit: 21,
        code: "output_serialization_failed",
        message: error,
        retryable: false,
    })?;
    println!("{raw}");
    Ok(())
}

fn map_search_data(response: app_infra::brokered_access::BrokerSearchResponse) -> SearchData {
    let next_cursor = response.next_cursor.clone();
    let speaker_coverage = response.speaker_coverage.clone();
    SearchData {
        results: response
            .results
            .into_iter()
            .map(|result| SearchResultData {
                id: result.opaque_id,
                kind: result.kind,
                snippet: result.snippet,
                started_at: result.started_at,
                ended_at: result.ended_at,
                context: result.context.map(|context| SearchResultContextData {
                    app_bundle_id: context.app_bundle_id,
                    app_name: context.app_name,
                    window_title: context.window_title,
                    url: context.url,
                }),
                turns: result.turns,
            })
            .collect(),
        limit: response.limit,
        next_cursor,
        speaker_coverage,
    }
}

fn map_timeline_data(response: app_infra::brokered_access::BrokerTimelineResponse) -> TimelineData {
    let truncated = response.intervals.len() as u32 >= response.limit;
    let speaker_coverage = response.speaker_coverage.clone();
    TimelineData {
        intervals: response
            .intervals
            .into_iter()
            .map(|interval| TimelineIntervalData {
                // Broker kinds pass through verbatim: renaming them here would hide
                // mic-vs-system audio from every agent reading this output.
                kind: interval.kind,
                started_at: interval.started_at,
                ended_at: interval.ended_at.unwrap_or_default(),
                opaque_id: interval.opaque_id,
                context: interval.context.map(|context| SearchResultContextData {
                    app_bundle_id: context.app_bundle_id,
                    app_name: context.app_name,
                    window_title: context.window_title,
                    url: context.url,
                }),
                turns: interval.turns,
            })
            .collect(),
        limit: response.limit,
        truncated,
        speaker_coverage,
    }
}

fn client_envelope(identity: &BrokerClientIdentity) -> ClientEnvelope {
    ClientEnvelope {
        label: identity.label.clone(),
        source: identity_source_name(&identity.source).to_string(),
    }
}

fn identity_source_name(source: &BrokerClientIdentitySource) -> &'static str {
    match source {
        BrokerClientIdentitySource::Explicit => "explicit",
        BrokerClientIdentitySource::Env => "env",
        BrokerClientIdentitySource::Inferred => "inferred",
        BrokerClientIdentitySource::Defaulted => "defaulted",
    }
}

fn response_requires_authorization(response: &BrokeredCaptureResponse) -> bool {
    let BrokeredCaptureResponse::Error(error) = response else {
        return false;
    };
    error.error == BrokerAuthStatusKind::AuthorizationRequired
        && error.message
            == BrokerAuthStatus::authorization_required()
                .reason
                .unwrap_or_default()
}

fn map_broker_response_error(error: BrokerErrorResponse) -> CliError {
    if error.message.contains("outside the grant scope") {
        return CliError {
            exit: 13,
            code: "outside_grant_scope",
            message: error.message,
            retryable: false,
        };
    }
    if error.message
        == BrokerAuthStatus::authorization_required()
            .reason
            .unwrap_or_default()
    {
        return auth_required_error();
    }
    broker_failure(error.message)
}

fn auth_required_error() -> CliError {
    CliError {
        exit: 10,
        code: "authorization_required",
        message: "CLI Access approval is required.".to_string(),
        retryable: true,
    }
}

fn timeout_error() -> CliError {
    CliError {
        exit: 11,
        code: "authorization_timeout",
        message: "CLI Access approval timed out.".to_string(),
        retryable: true,
    }
}

fn app_unavailable_error() -> CliError {
    CliError {
        exit: 12,
        code: "app_unavailable",
        message: "Mnema app is unavailable.".to_string(),
        retryable: true,
    }
}

fn authorization_denied_error() -> CliError {
    CliError {
        exit: 10,
        code: "authorization_denied",
        message: "CLI Access approval was denied.".to_string(),
        retryable: true,
    }
}

fn broker_failure(message: impl Into<String>) -> CliError {
    CliError {
        exit: 20,
        code: "broker_operation_failed",
        message: message.into(),
        retryable: false,
    }
}

fn broker_error(error: impl std::fmt::Display) -> CliError {
    broker_failure(error.to_string())
}

fn usage_error(message: impl Into<String>) -> CliError {
    CliError {
        exit: 2,
        code: "usage",
        message: message.into(),
        retryable: false,
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_accepts_documented_data_commands() {
        Cli::try_parse_from([
            "mnema",
            "search",
            "--query",
            "invoice",
            "--app",
            "Linear",
            "--window-title",
            "Roadmap",
        ])
        .unwrap();
        Cli::try_parse_from([
            "mnema",
            "timeline",
            "--from",
            "2026-05-22T10:00:00Z",
            "--to",
            "2026-05-22T11:00:00Z",
            "--app",
            "Linear",
            "--window-title",
            "Roadmap",
        ])
        .unwrap();
        Cli::try_parse_from(["mnema", "show-text", "f1.deadbeef"]).unwrap();
        Cli::try_parse_from(["mnema", "open", "f1.deadbeef"]).unwrap();
        Cli::try_parse_from(["mnema", "--client", "Claude Desktop", "mcp"]).unwrap();
    }

    #[test]
    fn cli_url_filters_are_mutually_exclusive() {
        Cli::try_parse_from(["mnema", "search", "--query", "invoice", "--url", "github.com"])
            .unwrap();
        Cli::try_parse_from([
            "mnema",
            "search",
            "--query",
            "invoice",
            "--url-regex",
            "^github\\.com/",
        ])
        .unwrap();
        assert!(Cli::try_parse_from([
            "mnema",
            "search",
            "--query",
            "invoice",
            "--url",
            "github.com",
            "--url-regex",
            "^github",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "mnema",
            "timeline",
            "--from",
            "2026-05-22T10:00:00Z",
            "--to",
            "2026-05-22T11:00:00Z",
            "--url",
            "github.com",
            "--url-regex",
            "^github",
        ])
        .is_err());
    }

    #[test]
    fn cli_rejects_removed_aliases() {
        assert!(Cli::try_parse_from(["mnema", "auth", "status"]).is_err());
        assert!(Cli::try_parse_from(["mnema", "open-in-mnema", "f1"]).is_err());
        // `open-url` was removed: the broker never opens a raw captured URL, so the
        // CLI no longer exposes the command (see ADR 0038 / brokered_access.rs).
        assert!(Cli::try_parse_from(["mnema", "open-url", "f1.deadbeef"]).is_err());
    }

    #[test]
    fn cli_accepts_access_commands() {
        Cli::try_parse_from(["mnema", "access", "status", "--all-clients"]).unwrap();
        Cli::try_parse_from([
            "mnema",
            "access",
            "request",
            "--scope",
            "all-retained",
            "--duration",
            "7d",
        ])
        .unwrap();
        Cli::try_parse_from(["mnema", "access", "known-clients"]).unwrap();
        Cli::try_parse_from(["mnema", "access", "revoke", "grant-1"]).unwrap();
        Cli::try_parse_from(["mnema", "access", "revoke-client", "Codex", "--yes"]).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn authorization_socket_lives_under_app_config_dir() {
        let config_dir = default_app_config_dir().expect("config dir should resolve");

        assert_eq!(
            authorization_socket_path(),
            config_dir.join("cli-access.sock")
        );
    }

    #[test]
    fn search_mapping_preserves_allowlisted_context() {
        let data = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![app_infra::brokered_access::BrokerSearchResult {
                opaque_id: "f1.signature".to_string(),
                kind: "frame".to_string(),
                snippet: "frame target".to_string(),
                started_at: "2026-05-17T10:00:00Z".to_string(),
                ended_at: "2026-05-17T10:00:00Z".to_string(),
                context: Some(app_infra::brokered_access::BrokerSearchResultContext {
                    app_bundle_id: Some("com.example.Linear".to_string()),
                    app_name: Some("Linear".to_string()),
                    window_title: Some("Roadmap".to_string()),
                    url: Some("linear.app/team/roadmap".to_string()),
                }),
                span_start_ms: None,
                span_end_ms: None,
                aligned_frame_id: None,
                turns: Vec::new(),
            }],
            limit: 1,
            next_cursor: None,
            speaker_coverage: None,
        });

        let context = data.results[0]
            .context
            .as_ref()
            .expect("context should map through");
        assert_eq!(context.app_bundle_id.as_deref(), Some("com.example.Linear"));
        assert_eq!(context.app_name.as_deref(), Some("Linear"));
        assert_eq!(context.window_title.as_deref(), Some("Roadmap"));
        assert_eq!(context.url.as_deref(), Some("linear.app/team/roadmap"));
    }

    #[test]
    fn timeline_mapping_preserves_allowlisted_context() {
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse {
            intervals: vec![app_infra::brokered_access::BrokerTimelineInterval {
                kind: "frame".to_string(),
                started_at: "2026-05-17T10:00:00Z".to_string(),
                ended_at: Some("2026-05-17T10:00:00Z".to_string()),
                opaque_id: None,
                context: Some(app_infra::brokered_access::BrokerSearchResultContext {
                    app_bundle_id: Some("com.example.Linear".to_string()),
                    app_name: Some("Linear".to_string()),
                    window_title: Some("Roadmap".to_string()),
                    url: Some("linear.app/team/roadmap".to_string()),
                }),
                turns: Vec::new(),
            }],
            limit: 1,
            speaker_coverage: None,
        });

        let context = data.intervals[0]
            .context
            .as_ref()
            .expect("context should map through");
        assert_eq!(context.app_bundle_id.as_deref(), Some("com.example.Linear"));
        assert_eq!(context.app_name.as_deref(), Some("Linear"));
        assert_eq!(context.window_title.as_deref(), Some("Roadmap"));
        assert_eq!(context.url.as_deref(), Some("linear.app/team/roadmap"));
    }

    #[test]
    fn timeline_mapping_passes_broker_kinds_and_opaque_ids_through() {
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse {
            intervals: vec![
                timeline_interval("frame", Some("f1.signature")),
                timeline_interval("audio_microphone", Some("a1.signature")),
                timeline_interval("audio_system", None),
            ],
            limit: 3,
            speaker_coverage: None,
        });

        assert_eq!(
            data.intervals
                .iter()
                .map(|interval| interval.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["frame", "audio_microphone", "audio_system"],
            "broker kinds must reach the agent unrenamed"
        );
        assert_eq!(data.intervals[1].opaque_id.as_deref(), Some("a1.signature"));

        let json = serde_json::to_string(&data.intervals[2]).expect("interval should serialize");
        assert!(
            !json.contains("opaqueId"),
            "an interval with nothing to follow omits the field: {json}"
        );
        assert!(
            !json.contains("summary"),
            "the phantom summary field is gone: {json}"
        );
    }

    #[test]
    fn search_mapping_passes_broker_kinds_through() {
        let data = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![
                search_result_with_kind("f1", "frame"),
                search_result_with_kind("a1", "audio_microphone"),
                search_result_with_kind("a2", "audio_system"),
            ],
            limit: 3,
            next_cursor: None,
            speaker_coverage: None,
        });

        assert_eq!(
            data.results
                .iter()
                .map(|result| result.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["frame", "audio_microphone", "audio_system"]
        );
    }

    fn timeline_interval(
        kind: &str,
        opaque_id: Option<&str>,
    ) -> app_infra::brokered_access::BrokerTimelineInterval {
        app_infra::brokered_access::BrokerTimelineInterval {
            kind: kind.to_string(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: Some("2026-05-17T10:00:30Z".to_string()),
            opaque_id: opaque_id.map(str::to_string),
            context: None,
            turns: Vec::new(),
        }
    }

    fn search_result_with_kind(
        id: &str,
        kind: &str,
    ) -> app_infra::brokered_access::BrokerSearchResult {
        app_infra::brokered_access::BrokerSearchResult {
            kind: kind.to_string(),
            ..search_result(id)
        }
    }

    #[test]
    fn search_cursor_and_timeline_truncation_reach_the_envelope() {
        let paged = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![search_result("f1")],
            limit: 1,
            next_cursor: Some("v1:42:1:0".to_string()),
            speaker_coverage: None,
        });
        assert_eq!(paged.next_cursor.as_deref(), Some("v1:42:1:0"));

        let last = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![search_result("f1")],
            limit: 20,
            next_cursor: None,
            speaker_coverage: None,
        });
        assert!(last.next_cursor.is_none());

        // Timeline has no cursor: it merges two independently-limited sources and
        // re-sorts, so a full page only reports that records may have been dropped.
        let timeline = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse {
            intervals: Vec::new(),
            limit: 0,
            speaker_coverage: None,
        });
        assert!(timeline.truncated, "limit 0 can never be complete");
    }

    #[test]
    fn cli_accepts_the_speaker_surface() {
        Cli::try_parse_from(["mnema", "speakers"]).unwrap();
        Cli::try_parse_from(["mnema", "speakers", "--name", "priya", "--limit", "5"]).unwrap();
        Cli::try_parse_from(["mnema", "search", "--query", "standup", "--speaker", "p1.sig"])
            .unwrap();
        Cli::try_parse_from([
            "mnema",
            "timeline",
            "--from",
            "2026-05-22T10:00:00Z",
            "--to",
            "2026-05-22T11:00:00Z",
            "--speaker",
            "p1.sig",
        ])
        .unwrap();
    }

    /// The broker rejects speaker + screen filters outright (the app lives on
    /// frames, the voice on audio, nothing joins them). The CLI door says so
    /// before the round trip, the same way it does for the two url filters.
    #[test]
    fn cli_rejects_a_speaker_filter_beside_a_screen_filter() {
        assert!(Cli::try_parse_from([
            "mnema", "search", "--query", "standup", "--speaker", "p1.sig", "--app", "Zoom",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "mnema",
            "timeline",
            "--from",
            "2026-05-22T10:00:00Z",
            "--to",
            "2026-05-22T11:00:00Z",
            "--speaker",
            "p1.sig",
            "--url",
            "zoom.us",
        ])
        .is_err());
    }

    /// SKILL.md step 9 offers `mnema search --speaker <handle>` and
    /// `mnema timeline --speaker <handle>` as interchangeable answers to "what did
    /// *someone* say". They are not: `search` still requires `--query`, so the
    /// keyword-free question the speaker filter exists for ("what did Priya say
    /// yesterday") parses only through `timeline`.
    #[test]
    fn a_keyword_free_speaker_question_only_parses_through_timeline() {
        assert!(
            Cli::try_parse_from(["mnema", "search", "--speaker", "p1.sig"]).is_err(),
            "search still requires --query"
        );
        Cli::try_parse_from([
            "mnema",
            "timeline",
            "--from",
            "2026-05-22T00:00:00Z",
            "--to",
            "2026-05-22T23:59:59Z",
            "--speaker",
            "p1.sig",
        ])
        .expect("timeline is the keyword-free door");
    }

    /// The `speakers` response is serialized VERBATIM, so nothing in this crate
    /// declares its wire shape — this is the only place the CLI's own contract
    /// ("`name` absent for an unnamed voice", `speakingMs`, `recognizedTurns`,
    /// `handle.startMs`) is asserted against what an agent actually receives.
    #[test]
    fn speakers_envelope_omits_the_name_of_an_unnamed_voice() {
        let response = app_infra::brokered_access::BrokerSpeakersResponse {
            speakers: vec![app_infra::brokered_access::BrokerSpeakerSummary {
                name: None,
                handle: app_infra::brokered_access::BrokerSpeakerHandle {
                    id: "v1.signature".to_string(),
                    kind: "voice".to_string(),
                    start_ms: Some(0),
                    end_ms: Some(30_000),
                },
                speaking_ms: 12_000,
                assigned_turns: 0,
                recognized_turns: 0,
            }],
            limit: 20,
            truncated: true,
        };
        let json = serde_json::to_value(&response).expect("speakers should serialize");
        assert_eq!(json["speakers"][0]["speakingMs"], 12_000);
        assert_eq!(json["speakers"][0]["recognizedTurns"], 0);
        assert_eq!(json["speakers"][0]["handle"]["startMs"], 0);
        assert!(
            json["speakers"][0].get("name").is_none(),
            "an unnamed voice omits `name`, never reports it as null: {json}"
        );
    }

    fn speaker_turn(text: &str) -> BrokerSpeakerTurn {
        BrokerSpeakerTurn {
            speaker: None,
            start_ms: 1_000,
            end_ms: 4_000,
            text: text.to_string(),
        }
    }

    fn speaker_coverage() -> BrokerSpeakerCoverage {
        BrokerSpeakerCoverage {
            recordings_with_unnamed_voices: 3,
            recordings_without_speaker_data: 7,
        }
    }

    /// Asserted on the SERIALIZED envelope, not the struct: a field the
    /// presentation layer forgot to carry does not exist as far as an agent is
    /// concerned, and a struct-level assert is exactly what misses it.
    #[test]
    fn search_mapping_carries_speaker_turns_and_coverage() {
        let data = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![app_infra::brokered_access::BrokerSearchResult {
                turns: vec![speaker_turn("we ship on Friday")],
                ..search_result("a1.signature")
            }],
            limit: 1,
            next_cursor: None,
            speaker_coverage: Some(speaker_coverage()),
        });

        let json = serde_json::to_value(&data).expect("search data should serialize");
        assert_eq!(json["results"][0]["turns"][0]["text"], "we ship on Friday");
        assert_eq!(json["results"][0]["turns"][0]["startMs"], 1_000);
        assert_eq!(json["speakerCoverage"]["recordingsWithUnnamedVoices"], 3);
        assert_eq!(json["speakerCoverage"]["recordingsWithoutSpeakerData"], 7);

        let unfiltered = serde_json::to_value(map_search_data(
            app_infra::brokered_access::BrokerSearchResponse {
                results: vec![search_result("f1.signature")],
                limit: 1,
                next_cursor: None,
                speaker_coverage: None,
            },
        ))
        .expect("search data should serialize");
        assert!(unfiltered["results"][0].get("turns").is_none());
        assert!(unfiltered.get("speakerCoverage").is_none());
    }

    #[test]
    fn timeline_mapping_carries_speaker_turns_and_coverage() {
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse {
            intervals: vec![app_infra::brokered_access::BrokerTimelineInterval {
                turns: vec![speaker_turn("we ship on Friday")],
                ..timeline_interval("audio_microphone", Some("a1.signature"))
            }],
            limit: 1,
            speaker_coverage: Some(speaker_coverage()),
        });

        let json = serde_json::to_value(&data).expect("timeline data should serialize");
        assert_eq!(json["intervals"][0]["turns"][0]["text"], "we ship on Friday");
        assert_eq!(json["speakerCoverage"]["recordingsWithUnnamedVoices"], 3);
        assert_eq!(json["speakerCoverage"]["recordingsWithoutSpeakerData"], 7);

        let unfiltered = serde_json::to_value(map_timeline_data(
            app_infra::brokered_access::BrokerTimelineResponse {
                intervals: vec![timeline_interval("audio_microphone", Some("a1.signature"))],
                limit: 1,
                speaker_coverage: None,
            },
        ))
        .expect("timeline data should serialize");
        assert!(unfiltered["intervals"][0].get("turns").is_none());
        assert!(unfiltered.get("speakerCoverage").is_none());
    }

    /// `show-text` speakers are re-serialized by the CLI, so the handle an agent
    /// needs to filter by has to survive that hop.
    #[test]
    fn show_text_mapping_carries_speaker_handles_and_turns() {
        let data = ShowTextData {
            id: "a1.signature".to_string(),
            kind: "audio_microphone".to_string(),
            text: "we ship on Friday".to_string(),
            speakers: vec![BrokerSpeaker {
                name: Some("Priya".to_string()),
                attribution: "assigned".to_string(),
                confidence: None,
                handle: app_infra::brokered_access::BrokerSpeakerHandle {
                    id: "p1.signature".to_string(),
                    kind: "person".to_string(),
                    start_ms: None,
                    end_ms: None,
                },
            }],
            turns: vec![BrokerSpeakerTurn {
                speaker: Some(0),
                ..speaker_turn("we ship on Friday")
            }],
        };

        let json = serde_json::to_value(&data).expect("show-text data should serialize");
        assert_eq!(json["speakers"][0]["handle"]["id"], "p1.signature");
        assert_eq!(json["speakers"][0]["handle"]["kind"], "person");
        assert_eq!(json["turns"][0]["speaker"], 0);
        assert_eq!(json["turns"][0]["text"], "we ship on Friday");
    }

    fn search_result(id: &str) -> app_infra::brokered_access::BrokerSearchResult {
        app_infra::brokered_access::BrokerSearchResult {
            opaque_id: id.to_string(),
            kind: "frame".to_string(),
            snippet: String::new(),
            started_at: "2026-05-17T10:00:00Z".to_string(),
            ended_at: "2026-05-17T10:00:00Z".to_string(),
            context: None,
            span_start_ms: None,
            span_end_ms: None,
            aligned_frame_id: None,
            turns: Vec::new(),
        }
    }

    #[test]
    fn version_flag_is_defined() {
        Cli::command().debug_assert();
        Cli::try_parse_from(["mnema", "--version"]).unwrap_err();
    }

    #[test]
    fn authorization_denial_does_not_retry_by_launching_app() {
        assert!(!should_retry_authorization_with_app_launch(
            &authorization_denied_error()
        ));
    }

    #[test]
    fn authorization_timeout_does_not_retry_by_launching_app() {
        assert!(!should_retry_authorization_with_app_launch(&timeout_error()));
    }

    #[test]
    fn app_unavailable_retries_by_launching_app() {
        assert!(should_retry_authorization_with_app_launch(
            &app_unavailable_error()
        ));
    }

    #[test]
    fn authorization_retry_propagates_second_attempt_error() {
        let error =
            authorization_retry_result(app_unavailable_error(), Err(authorization_denied_error()))
                .unwrap_err();

        assert_eq!(error.code, "authorization_denied");
        assert_eq!(error.exit, 10);
    }

    #[test]
    fn authorization_prompt_requires_interactive_stdio() {
        assert!(can_prompt_for_authorization_with(|| true, || true, || true));
        assert!(!can_prompt_for_authorization_with(
            || false,
            || true,
            || true
        ));
        assert!(!can_prompt_for_authorization_with(
            || true,
            || false,
            || true
        ));
        assert!(!can_prompt_for_authorization_with(
            || true,
            || true,
            || false
        ));
    }

    #[test]
    fn explicit_access_request_is_allowed_without_interactive_stdio() {
        assert!(can_start_explicit_authorization_request(false));
        assert!(!can_start_explicit_authorization_request(true));
    }

    #[test]
    fn inferred_agent_markers_include_pi() {
        assert_eq!(
            INFERRED_AGENT_ENV_LABELS
                .iter()
                .find(|(key, _)| *key == "PI_CODING_AGENT")
                .map(|(_, label)| *label),
            Some("PI")
        );
    }

    #[test]
    fn inferred_agent_markers_include_current_codex_harness() {
        for key in [
            "CODEX_CI",
            "CODEX_MANAGED_BY_BUN",
            "CODEX_MANAGED_PACKAGE_ROOT",
            "CODEX_THREAD_ID",
        ] {
            assert_eq!(
                INFERRED_AGENT_ENV_LABELS
                    .iter()
                    .find(|(candidate, _)| *candidate == key)
                    .map(|(_, label)| *label),
                Some("Codex"),
                "{key} should infer Codex"
            );
        }
    }

    #[test]
    fn infers_codex_from_current_harness_marker() {
        assert_eq!(
            inferred_agent_label_from_env(|key| key == "CODEX_THREAD_ID"),
            Some("Codex")
        );
    }

    #[test]
    fn infers_opencode_from_runtime_markers() {
        for marker in ["OPENCODE", "OPENCODE_PID"] {
            assert_eq!(
                inferred_agent_label_from_env(|key| key == marker),
                Some("OpenCode"),
                "{marker} should infer OpenCode"
            );
        }
    }

    #[test]
    fn known_client_list_is_derived_from_inferred_markers() {
        assert_eq!(
            inferred_agent_labels(),
            vec!["Claude Code", "Cursor", "Codex", "OpenCode", "PI"]
        );
    }

    #[test]
    fn known_agent_detection_beats_versioned_ai_agent_value() {
        // Claude Code sets both CLAUDECODE=1 and a versioned AI_AGENT value.
        // Curated detection must win so the identity is version-free.
        let identity = resolve_identity_from_env(
            None,
            |key| (key == "AI_AGENT").then(|| "claude-code_2-1-152_agent".to_string()),
            |key| key == "CLAUDECODE",
        )
        .expect("identity resolves");
        assert_eq!(identity.normalized_label, "claude code");
        assert!(matches!(
            identity.source,
            BrokerClientIdentitySource::Inferred
        ));
    }

    #[test]
    fn explicit_mnema_client_env_overrides_known_agent_detection() {
        let identity = resolve_identity_from_env(
            None,
            |key| match key {
                "MNEMA_CLI_CLIENT" => Some("Custom Label".to_string()),
                "AI_AGENT" => Some("claude-code_2-1-152_agent".to_string()),
                _ => None,
            },
            |key| key == "CLAUDECODE",
        )
        .expect("identity resolves");
        assert_eq!(identity.normalized_label, "custom label");
        assert!(matches!(identity.source, BrokerClientIdentitySource::Env));
    }

    #[test]
    fn unrecognized_agent_falls_back_to_ai_agent_value() {
        let identity = resolve_identity_from_env(
            None,
            |key| (key == "AI_AGENT").then(|| "Some Tool".to_string()),
            |_| false,
        )
        .expect("identity resolves");
        assert_eq!(identity.normalized_label, "some tool");
        assert!(matches!(identity.source, BrokerClientIdentitySource::Env));
    }
}
