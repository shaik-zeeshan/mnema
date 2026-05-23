#[cfg(unix)]
use std::time::Duration;
use std::{env, io::IsTerminal, path::PathBuf, process::ExitCode};

use app_infra::brokered_access::{
    BrokerAuthStatus, BrokerAuthStatusKind, BrokerClientIdentity, BrokerClientIdentitySource,
    BrokerErrorResponse, BrokerSearchRequest, BrokerTimelineRequest, BrokeredCaptureAccess,
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
    limit: u32,
    next_cursor: Option<String>,
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineData {
    intervals: Vec<TimelineIntervalData>,
    limit: u32,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineIntervalData {
    kind: String,
    started_at: String,
    ended_at: String,
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<SearchResultContextData>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShowTextData {
    id: String,
    kind: String,
    text: String,
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
            });
            run_data_command("timeline", &identity, request, cli.format, cli.no_prompt).await
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
    let access =
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER).map_err(broker_error)?;
    let mut response = access
        .execute_for_identity(identity.clone(), request.clone())
        .await
        .map_err(broker_error)?;

    if response_requires_authorization(&response) {
        if no_prompt || !can_prompt_for_authorization() {
            return print_structured_error(command, identity, format, auth_required_error());
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

    if let BrokeredCaptureResponse::Error(error) = response {
        let cli_error = map_broker_response_error(error);
        return print_structured_error(command, identity, format, cli_error);
    }

    match command {
        "search" => {
            let BrokeredCaptureResponse::Search(response) = response else {
                return Err(broker_failure("unexpected search response"));
            };
            print_envelope(command, identity, format, &map_search_data(response))
        }
        "timeline" => {
            let BrokeredCaptureResponse::Timeline(response) = response else {
                return Err(broker_failure("unexpected timeline response"));
            };
            print_envelope(command, identity, format, &map_timeline_data(response))
        }
        "show-text" => {
            let BrokeredCaptureResponse::ShowText(response) = response else {
                return Err(broker_failure("unexpected show-text response"));
            };
            print_envelope(
                command,
                identity,
                format,
                &ShowTextData {
                    id: response.opaque_id,
                    kind: map_kind(&response.kind),
                    text: response.text,
                },
            )
        }
        "open" => {
            let BrokeredCaptureResponse::OpenInMnema(response) = response else {
                return Err(broker_failure("unexpected open response"));
            };
            print_envelope(
                command,
                identity,
                format,
                &OpenData {
                    id: response.opaque_id,
                    opened: response.opened,
                },
            )
        }
        _ => Err(broker_failure("unsupported command")),
    }
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
    if let Some(value) = explicit {
        return BrokerClientIdentity::new(value, BrokerClientIdentitySource::Explicit)
            .map_err(|_| usage_error("--client must contain a visible client name"));
    }
    for (key, source) in [
        ("MNEMA_CLI_CLIENT", BrokerClientIdentitySource::Env),
        ("AI_AGENT", BrokerClientIdentitySource::Env),
    ] {
        if let Ok(value) = env::var(key) {
            if let Ok(identity) = BrokerClientIdentity::new(value, source) {
                return Ok(identity);
            }
        }
    }
    if let Some(label) = inferred_agent_label_from_env(|key| env::var_os(key).is_some()) {
        return BrokerClientIdentity::new(label, BrokerClientIdentitySource::Inferred)
            .map_err(broker_error);
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
    SearchData {
        results: response
            .results
            .into_iter()
            .map(|result| SearchResultData {
                id: result.opaque_id,
                kind: map_kind(&result.kind),
                snippet: result.snippet,
                started_at: result.started_at,
                ended_at: result.ended_at,
                context: result.context.map(|context| SearchResultContextData {
                    app_bundle_id: context.app_bundle_id,
                    app_name: context.app_name,
                    window_title: context.window_title,
                }),
            })
            .collect(),
        limit: response.limit,
        next_cursor: None,
    }
}

fn map_timeline_data(response: app_infra::brokered_access::BrokerTimelineResponse) -> TimelineData {
    TimelineData {
        intervals: response
            .intervals
            .into_iter()
            .map(|interval| TimelineIntervalData {
                kind: if interval.kind.starts_with("audio") {
                    "audio".to_string()
                } else {
                    "screen".to_string()
                },
                started_at: interval.started_at,
                ended_at: interval.ended_at.unwrap_or_default(),
                summary: None,
                context: interval.context.map(|context| SearchResultContextData {
                    app_bundle_id: context.app_bundle_id,
                    app_name: context.app_name,
                    window_title: context.window_title,
                }),
            })
            .collect(),
        limit: response.limit,
        next_cursor: None,
    }
}

fn map_kind(kind: &str) -> String {
    match kind {
        "frame" => "screenText",
        "audio" => "audioTranscript",
        other => other,
    }
    .to_string()
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
    }

    #[test]
    fn cli_rejects_removed_aliases() {
        assert!(Cli::try_parse_from(["mnema", "auth", "status"]).is_err());
        assert!(Cli::try_parse_from(["mnema", "open-in-mnema", "f1"]).is_err());
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
                }),
            }],
            limit: 1,
        });

        let context = data.results[0]
            .context
            .as_ref()
            .expect("context should map through");
        assert_eq!(context.app_bundle_id.as_deref(), Some("com.example.Linear"));
        assert_eq!(context.app_name.as_deref(), Some("Linear"));
        assert_eq!(context.window_title.as_deref(), Some("Roadmap"));
    }

    #[test]
    fn timeline_mapping_preserves_allowlisted_context() {
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse {
            intervals: vec![app_infra::brokered_access::BrokerTimelineInterval {
                kind: "screen".to_string(),
                started_at: "2026-05-17T10:00:00Z".to_string(),
                ended_at: Some("2026-05-17T10:00:00Z".to_string()),
                reason: None,
                context: Some(app_infra::brokered_access::BrokerSearchResultContext {
                    app_bundle_id: Some("com.example.Linear".to_string()),
                    app_name: Some("Linear".to_string()),
                    window_title: Some("Roadmap".to_string()),
                }),
            }],
            limit: 1,
        });

        let context = data.intervals[0]
            .context
            .as_ref()
            .expect("context should map through");
        assert_eq!(context.app_bundle_id.as_deref(), Some("com.example.Linear"));
        assert_eq!(context.app_name.as_deref(), Some("Linear"));
        assert_eq!(context.window_title.as_deref(), Some("Roadmap"));
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
}
