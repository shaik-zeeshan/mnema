#[cfg(unix)]
use std::time::Duration;
use std::{env, path::PathBuf, process::ExitCode};

use app_infra::brokered_access::{
    format_broker_unix_ms, minimum_scope_for_start, minimum_scope_for_window_start,
    BrokerAuthStatus, BrokerAuthStatusKind, BrokerClientIdentity, BrokerClientIdentitySource,
    BrokerErrorResponse, BrokerGrant, BrokerGrantScope, BrokerSearchRequest, BrokerSpeaker,
    BrokerSpeakerCoverage, BrokerSpeakerTurn, BrokerSpeakersRequest, BrokerTimelineRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest, BrokeredCaptureResponse,
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
/// How long to wait for an instant, windowless verdict before telling the user a
/// window is opening. Long enough for the app to answer from its permission file,
/// short enough to be invisible ahead of a window that takes longer than this to
/// draw anyway.
#[cfg(unix)]
const WINDOW_ANNOUNCE_DELAY: Duration = Duration::from_millis(400);
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
    /// Pre-authorize this machine's Mnema access from the terminal. Agents do
    /// not need it: a data command opens the same approval window by itself.
    Request {
        #[arg(long, value_enum, default_value = "last-day")]
        scope: AccessScope,
    },
    KnownClients,
    /// Block a tool's standing access. Re-enable it in Mnema under
    /// Settings -> Data -> Access.
    Revoke {
        client: String,
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
    #[value(name = "last-7-days", alias = "last7days")]
    Last7Days,
    AllRetained,
}

impl AccessScope {
    fn grant_scope(self) -> BrokerGrantScope {
        match self {
            Self::LastDay => BrokerGrantScope::LAST_DAY,
            Self::Last7Days => BrokerGrantScope::LAST_7_DAYS,
            Self::AllRetained => BrokerGrantScope::AllRetainedHistory,
        }
    }
}

/// The `--scope` spelling of a scope, for a message that tells the caller how to
/// widen. Derived from the one wire name so the two cannot drift.
fn scope_flag_name(scope: BrokerGrantScope) -> &'static str {
    match scope.wire_name() {
        "lastDay" => "last-day",
        "last7Days" => "last-7-days",
        _ => "all-retained",
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
    /// This client's access does not reach as far back as `--from` asked, so
    /// these results cover LESS than the window requested. Never read a thin page
    /// as "nothing happened" while this is set. Always serialized.
    scope_clamped: bool,
    /// The `--scope` that would have covered the request. Only when clamped.
    #[serde(skip_serializing_if = "Option::is_none")]
    required_scope: Option<String>,
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
    /// Where inside the recording the match actually falls, in ms from its
    /// start. A segment runs up to five minutes, so `startedAt`/`endedAt` alone
    /// place a hit no more precisely than "somewhere in here". Audio results
    /// only — a frame has no sub-segment span. The broker's `alignedFrameId`
    /// stays withheld: it is a raw database id, which this surface never emits.
    #[serde(skip_serializing_if = "Option::is_none")]
    span_start_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span_end_ms: Option<i64>,
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
    /// Oldest / newest `startedAt` returned: the slice of the requested window
    /// this page actually covers. A truncated page is the window's NEWEST end, so
    /// without these a caller reads the tail as the whole span.
    #[serde(skip_serializing_if = "Option::is_none")]
    covered_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    covered_to: Option<String>,
    /// Only on a `--speaker` timeline: how much audio the filter could NOT check.
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker_coverage: Option<BrokerSpeakerCoverage>,
    /// See [`SearchData::scope_clamped`]. Always serialized.
    scope_clamped: bool,
    /// The `--scope` that would have covered the request. Only when clamped.
    #[serde(skip_serializing_if = "Option::is_none")]
    required_scope: Option<String>,
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
struct AuthorizationResponse {
    schema_version: u32,
    request_id: String,
    decision: String,
    #[serde(default)]
    reason: Option<String>,
    /// What the permission carries AFTER the approval — present on `approved`.
    /// The approval window only enforces the request's `minimum`, so an approval
    /// can land NARROWER than what was asked for; reading `approved` as a yes is
    /// how a caller ends up silently under-served.
    #[serde(default)]
    grant: Option<AuthorizationGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizationGrant {
    #[allow(dead_code)]
    id: String,
    client_label: String,
    /// `lastDay` | `last7Days` | `allRetained`. No expiry: a permission stands
    /// until it idles out or is blocked in Settings.
    scope: String,
    #[allow(dead_code)]
    created: bool,
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
            mcp::serve(identity, !cli.no_prompt).await
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
    // `--no-prompt` is the ONLY control for "do not bother anyone". The old TTY
    // gate inspected the caller's file descriptors to guess whether a human was
    // at the Mac — which they say nothing about — and every agent harness pipes
    // stdout, so it dead-ended exactly the callers this surface exists for.
    let allow_prompt = !no_prompt;
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
    // A window that is empty whatever the permission says — `--from` after `--to`,
    // or a window lying wholly in the future — is refused by the broker with the
    // SAME "outside the grant scope" it uses for a window a wider permission would
    // reach. The widen door below reads that code as "ask the user for more scope",
    // so without this an agent asking about tomorrow pops a standing-permission
    // consent window that cannot help — even for a client already holding
    // `allRetained` — and then dies at exit 13 on the retry.
    if let Some(message) = empty_window_message(&request) {
        return Err(usage_error(message));
    }
    // A window whose END sits before the permission's scope start is refused on
    // the transport instead of clamped, so it never reaches the clamp door below.
    // `timeline` REQUIRES `--to`, so without this every dated timeline question
    // past the permission dies at exit 13 while the identical open-ended one gets
    // the approval window — whether a caller can widen must not turn on whether
    // it bounded its question.
    let mut widened = false;
    let mut response = match access
        .execute_for_identity(identity.clone(), request.clone())
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let error = broker_error(error);
            if !allow_prompt || error.code != "outside_grant_scope" {
                return Err(error);
            }
            widened = true;
            // Always the widen door: a client with no permission never reaches
            // this branch at all — the broker answers it with an
            // authorization-required RESPONSE (handled below), and only a client
            // that already has a row can have a window refused for sitting past
            // that row's scope.
            authorize_wider_access(command, identity, &request, ApprovalDoor::Widen).await?;
            access
                .execute_for_identity(identity.clone(), request.clone())
                .await
                .map_err(broker_error)?
        }
    };

    // Two ways this client's access can fall short of the call: none at all, or
    // one too narrow for the `--from` it asked for. Both take the same door —
    // a clamp that only widened on the NEXT run is how an agent reports "nothing
    // there" for a window it was never allowed to see.
    if response_requires_authorization(&response) || response_scope_clamped(&response) {
        if !allow_prompt {
            if response_requires_authorization(&response) {
                return Err(auth_required_error());
            }
            // A clamped page is real data. Hand it back with its marker rather
            // than throwing it away — the caller asked not to be prompted, not to
            // be lied to.
        } else if !widened {
            // The two ways in are also the two doors: "no permission at all" is a
            // first grant, a clamp is a widen of one that already exists.
            let door = if response_requires_authorization(&response) {
                ApprovalDoor::FirstGrant
            } else {
                ApprovalDoor::Widen
            };
            // At most ONE approval window per command: a second one for the same
            // call is the reflex-clicking this permission model exists to stop.
            match authorize_wider_access(command, identity, &request, door).await {
                Ok(()) => {
                    response = access
                        .execute_for_identity(identity.clone(), request)
                        .await
                        .map_err(broker_error)?;
                }
                // Nobody said no — the channel just never answered. The clamped
                // page in hand is data this client's STANDING permission already
                // covers, and the widen was an optional extra on top of it;
                // discarding it turns a partial answer into a total failure. Same
                // reasoning as the `--no-prompt` branch above, and as ADR 0048. A
                // real verdict (`denied`/`blocked`) still fails the call.
                Err(error) if widen_never_answered(&error) && response_scope_clamped(&response) => {
                }
                Err(error) => return Err(error),
            }
        }
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
            println!(
                "Client: {} ({})",
                identity.label,
                identity_source_name(&identity.source)
            );
            let rows: Vec<&BrokerGrant> = grants
                .grants
                .iter()
                .filter(|grant| all_clients || grant.normalized_label == identity.normalized_label)
                .collect();
            if rows.is_empty() {
                println!(
                    "CLI Access: none{}. Run a data command to ask for it.",
                    if all_clients { "" } else { " for this client" }
                );
            }
            for grant in rows {
                println!("{}", access_status_line(grant));
            }
            Ok(())
        }
        AccessCommand::Request { scope } => {
            if no_prompt {
                return Err(auth_required_error());
            }
            // A human pre-authorizing keeps the choice: `minimum` is the floor the
            // window enforces, so sending the REQUESTED scope there would grey out
            // every narrower option they might have preferred. Unlike a widen, no
            // call is waiting on this scope, so a narrower answer is a valid one —
            // and when it lands under a permission the client already holds, the
            // window says so before the click.
            let grant = request_authorization(
                "access request",
                identity,
                BrokerGrantScope::LAST_DAY,
                scope.grant_scope(),
            )
            .await?;
            match grant {
                Some(grant) => println!("{}", access_request_approved_line(&grant)),
                None => println!("CLI Access approved."),
            }
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
        AccessCommand::Revoke { client } => {
            // Blocked, not deleted: a rejection the tool can re-prompt its way
            // out of on the next run is not a rejection (ADR 0059).
            let blocked = access.block_client(&client).map_err(broker_error)?;
            println!(
                "{}",
                if blocked {
                    format!(
                        "Blocked {client}. Re-enable it in Mnema under Settings -> Data -> Access."
                    )
                } else {
                    format!("{client} has no access to block.")
                }
            );
            Ok(())
        }
    }
}

/// The receipt for a completed `access request`. The scope reads in the
/// `--scope` spelling, like every other CLI surface: the wire name (`allRetained`)
/// is not a value any caller can type back, and a reader handed one runs
/// `mnema access request --scope allRetained` straight into a usage error.
fn access_request_approved_line(grant: &AuthorizationGrant) -> String {
    let scope = BrokerGrantScope::from_wire_name(&grant.scope).map_or_else(
        || grant.scope.clone(),
        |scope| scope_flag_name(scope).to_string(),
    );
    format!(
        "CLI Access approved: {} can read {scope} until you block it in Settings.",
        grant.client_label
    )
}

/// One line per standing permission: scope, last use, blocked state. There is no
/// expiry to report — a permission dies 30 days after its last use, and
/// idle-expired rows are pruned on load, so nothing dead reaches here.
fn access_status_line(grant: &BrokerGrant) -> String {
    let state = if grant.blocked {
        "blocked".to_string()
    } else {
        format!("active, {}", scope_flag_name(grant.scope))
    };
    format!(
        "- {}: {} (last used {})",
        grant.label,
        state,
        format_broker_unix_ms(grant.last_used_at_unix_ms)
    )
}

/// Why this window is empty whatever the permission says, or `None` when only a
/// narrow permission stands between the caller and its answer.
///
/// The broker ends every window at `min(--to, now)` and refuses `start > end` with
/// the same "outside the grant scope" it raises for a window a wider permission
/// WOULD reach (`scoped_date_range`). Mirroring that one rule here is what keeps
/// the widen door from answering the caller's own contradiction with an approval
/// window. Anything less certain (a missing or unparseable `--from`) stays the
/// broker's to judge.
fn empty_window_message(request: &BrokeredCaptureRequest) -> Option<&'static str> {
    let (from, to) = match request {
        BrokeredCaptureRequest::Search(request) => (request.from.as_deref(), request.to.as_deref()),
        BrokeredCaptureRequest::Timeline(request) => {
            (Some(request.from.as_str()), Some(request.to.as_str()))
        }
        _ => (None, None),
    };
    let parse =
        |value: Option<&str>| value.and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok());
    let from = parse(from)?;
    match parse(to) {
        Some(to) if to < from => Some("--from must not be later than --to"),
        _ if from > OffsetDateTime::now_utc() => {
            Some("--from is in the future, and nothing is recorded after now")
        }
        _ => None,
    }
}

/// The narrowest access this call actually needs, from its `--from`. Always the
/// channel `preferred`; the `minimum` only on a widen, where a narrower answer
/// cannot satisfy the call and would take standing access away (see
/// [`authorize_wider_access`] for why the first-grant door keeps the narrow
/// options live). Shares [`minimum_scope_for_start`] with the broker's own clamp
/// marker, so the two can never disagree about what a `--from` costs.
///
/// `--to` is a fallback bound, not an afterthought: `search` accepts a `--to` with
/// no `--from`, and the broker then starts the window at the permission's scope
/// start — so a `--to` older than that start makes the range END before it BEGINS
/// and is refused on the transport. That reaches the widen door needing a scope
/// reaching back at least as far as `--to`, and deriving the ask from an absent
/// `--from` asks for `lastDay`: the one scope that cannot satisfy the call. The
/// user would be prompted, approve, and still hit exit 13 — on every run.
fn needed_scope_for(request: &BrokeredCaptureRequest) -> BrokerGrantScope {
    // WHICH bound this is decides how it is priced. A window START is served to
    // within the broker's own clamp slack, so it goes through
    // `minimum_scope_for_window_start` — otherwise "N days ago", which is over the
    // band edge by construction, asks the user for a standing permission one band
    // wider than the call needs, and the first-grant door then fails at exit 22
    // for the user who approves the pre-selected floor that would have worked.
    // A `--to` fallback gets the exact `minimum_scope_for_start`: nothing
    // tolerates a `to` under the window start, that range is refused outright
    // rather than clamped, so the ask has to reach PAST it.
    let (bound, is_window_start) = match request {
        // `from` first: it is the older bound whenever both are present.
        BrokeredCaptureRequest::Search(request) => match request.from.as_deref() {
            Some(from) => (Some(from), true),
            None => (request.to.as_deref(), false),
        },
        BrokeredCaptureRequest::Timeline(request) => (Some(request.from.as_str()), true),
        _ => (None, true),
    };
    // An unparseable bound is the broker's error to report, not ours to guess at.
    let Some(start) = bound.and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok()) else {
        return BrokerGrantScope::LAST_DAY;
    };
    let start_unix_ms = (start.unix_timestamp_nanos() / 1_000_000).max(0) as u64;
    if is_window_start {
        minimum_scope_for_window_start(start_unix_ms, now_unix_ms())
    } else {
        minimum_scope_for_start(start_unix_ms, now_unix_ms())
    }
}

/// An approval is not a yes. The window enforces only the request's `minimum`,
/// and an existing permission is widened rather than replaced, so `approved` can
/// still leave the row narrower than this call needs. Retrying on that produces
/// a clamped page and a confident, incomplete answer.
fn verify_granted_scope(
    needed: BrokerGrantScope,
    grant: Option<&AuthorizationGrant>,
) -> Result<(), CliError> {
    // ponytail: no `grant` field, or a scope name this build does not know, means
    // we cannot verify — retry and let the broker's own clamp marker speak.
    let Some(granted) = grant.and_then(|grant| BrokerGrantScope::from_wire_name(&grant.scope))
    else {
        return Ok(());
    };
    if granted.covers(&needed) {
        return Ok(());
    }
    Err(CliError {
        exit: 22,
        code: "scope_not_granted",
        message: format!(
            "CLI Access was granted at `{}`, which does not cover this request (`{}`). Widen it in \
             Mnema under Settings -> Data -> Access, or run `mnema access request --scope {}`.",
            scope_flag_name(granted),
            scope_flag_name(needed),
            scope_flag_name(needed)
        ),
        retryable: false,
    })
}

/// The widen never reached a verdict: nobody refused it, the channel just did
/// not answer.
fn widen_never_answered(error: &CliError) -> bool {
    matches!(
        error.code,
        "app_unavailable" | "authorization_timeout" | "authorization_busy"
    )
}

/// Which decision the user is being asked to make. Both doors open the same
/// window, and the window pre-selects the request's `minimum` — so the floor is
/// the whole of what the default answer grants, and it cannot be the same on
/// both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalDoor {
    /// This client holds no permission at all: the question is whether the tool
    /// gets access, and how much.
    FirstGrant,
    /// The client already holds a permission, and it is too narrow for this call.
    Widen,
}

/// Ask for the access this call actually needs, and refuse to retry on an
/// approval that came back narrower than that.
///
/// The `minimum` is per door, because a widen is not a first grant:
///
/// - On a **widen** the floor is `needed`. The row already stands narrower than
///   that — that is why this door was reached — and an approval SETS the row's
///   scope, so every answer below `needed` both fails this call AND takes away
///   access the tool already had. Offering one is offering a dead end that also
///   destroys standing access; the window pre-selects the floor, so the default
///   answer is the one that works.
/// - On a **first grant** the floor stays at the narrowest band. The window
///   disables every option below `minimum`, so sending `needed` there would let a
///   never-seen tool passing `--from 1970-01-01` make "All retained" the ONLY
///   thing the user can grant on that tool's first prompt. `needed` rides in
///   `preferred` instead — pre-selected, not forced — and a narrower approval is
///   caught by [`verify_granted_scope`] rather than silently under-serving the
///   caller. `access request` sends the same pair for the same reason.
///
/// A widen whose ask cannot be derived (`show-text`/`open` on an out-of-scope or
/// deleted result id derives `lastDay`) can still land below a wider row. The
/// window states the access the client already holds whenever the selection sits
/// under it, so that approval is informed rather than silent.
async fn authorize_wider_access(
    command: &str,
    identity: &BrokerClientIdentity,
    request: &BrokeredCaptureRequest,
    door: ApprovalDoor,
) -> Result<(), CliError> {
    let needed = needed_scope_for(request);
    let minimum = match door {
        ApprovalDoor::Widen => needed,
        ApprovalDoor::FirstGrant => BrokerGrantScope::LAST_DAY,
    };
    let grant = request_authorization(command, identity, minimum, needed).await?;
    verify_granted_scope(needed, grant.as_ref())
}

async fn request_authorization(
    command: &str,
    identity: &BrokerClientIdentity,
    minimum: BrokerGrantScope,
    preferred: BrokerGrantScope,
) -> Result<Option<AuthorizationGrant>, CliError> {
    let request = AuthorizationRequest {
        schema_version: 1,
        request_id: Uuid::new_v4().to_string(),
        client: AuthorizationClient {
            label: identity.label.clone(),
            source: identity_source_name(&identity.source).to_string(),
        },
        command: command.to_string(),
        scope: AuthorizationScope {
            minimum: minimum.wire_name().to_string(),
            preferred: preferred.wire_name().to_string(),
        },
        created_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string()),
    };
    match send_authorization_request(&request).await {
        Ok(grant) => Ok(grant),
        Err(first_error) if should_retry_authorization_with_app_launch(&first_error) => {
            let _ = launch_mnema_app().await;
            authorization_retry_result(first_error, send_authorization_request(&request).await)
        }
        Err(first_error) => Err(first_error),
    }
}

fn authorization_retry_result<T>(
    _first_error: CliError,
    retry_result: Result<T, CliError>,
) -> Result<T, CliError> {
    retry_result
}

#[cfg(unix)]
async fn send_authorization_request(
    request: &AuthorizationRequest,
) -> Result<Option<AuthorizationGrant>, CliError> {
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
    let mut reader = BufReader::new(stream);
    let response = read_verdict_line(&mut reader, || {
        eprintln!("CLI Access approval required. Opening Mnema...");
    })
    .await?;
    let response: AuthorizationResponse =
        serde_json::from_str(&response).map_err(|_| app_unavailable_error())?;
    authorization_result_for_response(&request.request_id, response)
}

/// Read one verdict line, two-phase.
///
/// Every verdict a window cannot change — `blocked` above all, plus `busy`,
/// `onboardingRequired` and the malformed-request codes — is answered from the
/// app's permission file without anything being opened, so it lands in
/// milliseconds. Announcing the window before that verdict arrives promises a
/// window that will never appear (and for `blocked`, never can). Hold the line
/// for one beat; only a request the app is still sitting on has a window, and
/// only then does `announce` run.
#[cfg(unix)]
async fn read_verdict_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    announce: impl FnOnce(),
) -> Result<String, CliError> {
    let mut response = String::new();
    let read = reader.read_line(&mut response);
    tokio::pin!(read);
    match timeout(WINDOW_ANNOUNCE_DELAY, &mut read).await {
        Ok(read) => {
            read.map_err(|_| app_unavailable_error())?;
        }
        Err(_) => {
            announce();
            timeout(AUTHORIZATION_TIMEOUT, &mut read)
                .await
                .map_err(|_| timeout_error())?
                .map_err(|_| app_unavailable_error())?;
        }
    }
    Ok(response)
}

/// Turn one channel response into this call's result.
///
/// The echo check rejects a verdict meant for a DIFFERENT request, but an EMPTY
/// `requestId` is not a crossed wire: the app answers `invalidRequest` for a
/// request it never parsed, so there is no id for it to echo. Treating that as a
/// mismatch swallowed the app's own verdict into `app_unavailable` — the one code
/// that relaunches (`open -b`) an app that is running and already answered.
fn authorization_result_for_response(
    request_id: &str,
    response: AuthorizationResponse,
) -> Result<Option<AuthorizationGrant>, CliError> {
    if !response.request_id.is_empty() && response.request_id != request_id {
        return Err(app_unavailable_error());
    }
    if response.decision == "approved" {
        // The empty-id exemption above exists only for verdicts on requests the app
        // never parsed. An approval is by definition issued for a request it DID
        // parse, so a real app always echoes the id — an `approved` with no id is
        // nobody's answer to this call, and accepting it would also feed its
        // `grant` field into the scope check that suppresses `scope_not_granted`.
        if response.request_id != request_id {
            return Err(app_unavailable_error());
        }
        return Ok(response.grant);
    }
    Err(authorization_response_error(
        &response.decision,
        response.reason.as_deref(),
    ))
}

#[cfg(not(unix))]
async fn send_authorization_request(
    _request: &AuthorizationRequest,
) -> Result<Option<AuthorizationGrant>, CliError> {
    Err(app_unavailable_error())
}

/// One error per channel reason code. Collapsing them into "Mnema app is
/// unavailable" is what told a user with unfinished onboarding that their app was
/// broken, and had the CLI relaunch an app that was merely mid-approval.
///
/// Only `app_unavailable` triggers the `open -b` relaunch (see
/// [`should_retry_authorization_with_app_launch`]), so every code here that means
/// "the app answered" stops the relaunch by construction.
fn authorization_response_error(decision: &str, reason: Option<&str>) -> CliError {
    // `blocked` rides on BOTH fields; either one is the standing rejection.
    match (decision, reason.unwrap_or_default()) {
        ("blocked", _) | (_, "blocked") => CliError {
            exit: 15,
            code: "access_blocked",
            message: "Mnema access for this client is blocked. Only Mnema can lift it: \
                      Settings -> Data -> Access."
                .to_string(),
            retryable: false,
        },
        ("denied", "closed") => CliError {
            exit: 14,
            code: "authorization_window_closed",
            message: "The Mnema access approval closed without a decision.".to_string(),
            retryable: true,
        },
        ("denied", _) => authorization_denied_error(),
        (_, "busy") => CliError {
            exit: 16,
            code: "authorization_busy",
            message: "Mnema is already showing another access approval. Answer it, then run \
                      this again."
                .to_string(),
            retryable: true,
        },
        (_, "onboardingRequired") => CliError {
            exit: 17,
            code: "onboarding_required",
            message: "Mnema onboarding is not finished. Open Mnema, complete setup, then run \
                      this again."
                .to_string(),
            retryable: false,
        },
        (_, "invalidRequest") => CliError {
            exit: 18,
            code: "authorization_invalid_request",
            message: "Mnema rejected the access request as malformed.".to_string(),
            retryable: false,
        },
        (_, "unsupportedVersion") => CliError {
            exit: 19,
            code: "authorization_unsupported_version",
            message: "This mnema CLI speaks a request version the Mnema app does not. Update \
                      both to the same release."
                .to_string(),
            retryable: false,
        },
        _ => app_unavailable_error(),
    }
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
    let scope_clamped = response.scope_clamped;
    let required_scope = response.required_scope.clone();
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
                span_start_ms: result.span_start_ms,
                span_end_ms: result.span_end_ms,
                turns: result.turns,
            })
            .collect(),
        limit: response.limit,
        next_cursor,
        speaker_coverage,
        scope_clamped,
        required_scope,
    }
}

fn map_timeline_data(response: app_infra::brokered_access::BrokerTimelineResponse) -> TimelineData {
    // `truncated` + the covered span come from the broker now (they used to be
    // recomputed here, which left Ask AI with no equivalent at all). One rule,
    // both doors.
    let truncated = response.truncated;
    let covered_from = response.covered_from.clone();
    let covered_to = response.covered_to.clone();
    let speaker_coverage = response.speaker_coverage.clone();
    let scope_clamped = response.scope_clamped;
    let required_scope = response.required_scope.clone();
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
        covered_from,
        covered_to,
        speaker_coverage,
        scope_clamped,
        required_scope,
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

/// The broker answered, but the permission's scope cut the requested window
/// short. Marked, never silent — see `BrokerSearchResponse::scope_clamped`.
fn response_scope_clamped(response: &BrokeredCaptureResponse) -> bool {
    match response {
        BrokeredCaptureResponse::Search(response) => response.scope_clamped,
        BrokeredCaptureResponse::Timeline(response) => response.scope_clamped,
        _ => false,
    }
}

fn map_broker_response_error(error: BrokerErrorResponse) -> CliError {
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
        // Not retryable: the user answered, and re-asking is the reflex-clicking
        // this permission model exists to stop.
        message: "CLI Access approval was denied.".to_string(),
        retryable: false,
    }
}

/// The two messages the broker raises for "this window or result is past the
/// permission". Matched as a SUFFIX, never with `contains`: the broker's own text
/// is the tail of the message (`AppInfraError` prefixes its variant, e.g.
/// `invalid search request: …`), while caller-supplied text is only ever quoted
/// mid-message.
///
/// A `contains` here made the consent trigger forgeable from the command line.
/// `broker_failure` classifies EVERY broker error, some of which echo caller text
/// back (`urlRegex is not a valid regular expression: <the pattern>`), and
/// `execute_data_request` opens the approval window on this code alone — so
/// `--url-regex "(outside the grant scope"` popped a standing-permission consent
/// window for a request that has nothing to do with scope, then reported exit 13
/// for what is really a bad regex.
///
// ponytail: suffix match, because the classification has to survive `impl
// Display`. Upgrade path if the wording ever drifts: give app-infra a dedicated
// `AppInfraError` variant (or an exported predicate) and match on that instead of
// on prose.
const OUTSIDE_GRANT_SCOPE_MESSAGES: &[&str] = &[
    "requested broker time range is outside the grant scope",
    "result is unavailable or outside the grant scope",
];

/// The one place a broker failure becomes a `CliError`, so the out-of-scope
/// classification cannot be reached by one door and missed by the other. The
/// window case (`search`/`timeline`) surfaces as an `Err` on the transport and
/// the result-id case (`show-text`/`open`) as a `BrokerErrorResponse`; both
/// mean "widen the permission", not "the query blew up inside Mnema", and an
/// agent told 20 goes looking for a Mnema fault instead of asking for scope.
fn broker_failure(message: impl Into<String>) -> CliError {
    let message = message.into();
    if OUTSIDE_GRANT_SCOPE_MESSAGES
        .iter()
        .any(|sentinel| message.ends_with(sentinel))
    {
        return CliError {
            exit: 13,
            code: "outside_grant_scope",
            message,
            retryable: false,
        };
    }
    CliError {
        exit: 20,
        code: "broker_operation_failed",
        message,
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
        Cli::try_parse_from([
            "mnema",
            "search",
            "--query",
            "invoice",
            "--url",
            "github.com",
        ])
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
        Cli::try_parse_from(["mnema", "access", "request", "--scope", "all-retained"]).unwrap();
        Cli::try_parse_from(["mnema", "access", "request", "--scope", "last-7-days"]).unwrap();
        Cli::try_parse_from(["mnema", "access", "request", "--scope", "last7days"]).unwrap();
        Cli::try_parse_from(["mnema", "access", "known-clients"]).unwrap();
        // One `revoke`, taking a CLIENT name: grant ids are no longer a surface
        // any human sees, and `revoke-client` was the only one that ever worked
        // on the thing a user actually thinks about.
        Cli::try_parse_from(["mnema", "access", "revoke", "Codex"]).unwrap();
    }

    /// A permission has no expiry to choose, so `--duration` cannot mean
    /// anything; and `revoke-client` collapsed into `revoke`.
    #[test]
    fn cli_rejects_the_removed_access_surface() {
        assert!(Cli::try_parse_from([
            "mnema",
            "access",
            "request",
            "--scope",
            "all-retained",
            "--duration",
            "7d",
        ])
        .is_err());
        assert!(
            Cli::try_parse_from(["mnema", "access", "revoke-client", "Codex", "--yes"]).is_err()
        );
    }

    /// The app answers `invalidRequest` for a request it never parsed — an
    /// oversized line, or bytes that are not the request shape — so it has no
    /// request id to echo and sends an EMPTY one. Rejecting that echo turns the
    /// app's exit-18 verdict into exit 12 `app_unavailable`, which also relaunches
    /// (`open -b`) an app that is running and already answered. `invalidRequest`
    /// and `authorization_invalid_request` (exit 18) are a contract this branch
    /// added and SKILL.md documents; the code has to be reachable.
    #[test]
    fn a_verdict_the_app_could_not_echo_an_id_for_still_reaches_its_own_exit_code() {
        let error = authorization_result_for_response(
            "request-1",
            AuthorizationResponse {
                schema_version: 1,
                request_id: String::new(),
                decision: "unavailable".to_string(),
                reason: Some("invalidRequest".to_string()),
                grant: None,
            },
        )
        .expect_err("a malformed-request verdict is an error");

        assert_eq!(error.code, "authorization_invalid_request");
        assert_eq!(error.exit, 18);
        assert!(
            !should_retry_authorization_with_app_launch(&error),
            "the app answered, so nothing may relaunch it"
        );
    }

    /// The echo check still has to reject a response for a DIFFERENT request:
    /// dropping it entirely would let a crossed or stale verdict authorize this
    /// call.
    #[test]
    fn a_verdict_for_another_request_is_still_rejected() {
        let error = authorization_result_for_response(
            "request-1",
            AuthorizationResponse {
                schema_version: 1,
                request_id: "request-2".to_string(),
                decision: "approved".to_string(),
                reason: None,
                grant: Some(AuthorizationGrant {
                    id: "grant-1".to_string(),
                    client_label: "Claude Code".to_string(),
                    scope: "allRetained".to_string(),
                    created: true,
                }),
            },
        )
        .expect_err("a verdict for another request is not this call's answer");

        assert_eq!(error.code, "app_unavailable");
    }

    /// The empty-id exemption exists only for verdicts on requests the app never
    /// parsed (`invalidRequest`). An APPROVAL is only ever issued for a request it
    /// DID parse, so a real app always has the id to echo — an `approved` with an
    /// empty id is nobody's answer to this call, and accepting it would also feed
    /// its `grant` metadata into the check that suppresses `scope_not_granted`.
    #[test]
    fn an_approval_without_an_echoed_request_id_is_rejected() {
        let error = authorization_result_for_response(
            "request-1",
            AuthorizationResponse {
                schema_version: 1,
                request_id: String::new(),
                decision: "approved".to_string(),
                reason: None,
                grant: Some(AuthorizationGrant {
                    id: "grant-1".to_string(),
                    client_label: "Claude Code".to_string(),
                    scope: "allRetained".to_string(),
                    created: true,
                }),
            },
        )
        .expect_err("an approval that echoes no request id is not this call's answer");

        assert_eq!(error.code, "app_unavailable");
    }

    /// The receipt for `mnema access request` is the one place the CLI hands a
    /// scope name back to a human or an agent. Every other CLI surface —
    /// `access status`, and the `scope_not_granted` message — spells a scope the
    /// way `--scope` takes it, and SKILL.md tells agents those are the only
    /// spellings. Printing the WIRE name tells the reader to re-run
    /// `mnema access request --scope allRetained`, which clap rejects (exit 2).
    #[test]
    fn the_access_request_receipt_spells_the_scope_the_way_the_flag_does() {
        for (wire, flag) in [
            ("lastDay", "last-day"),
            ("last7Days", "last-7-days"),
            ("allRetained", "all-retained"),
        ] {
            let line = access_request_approved_line(&AuthorizationGrant {
                id: "grant-1".to_string(),
                client_label: "Claude Code".to_string(),
                scope: wire.to_string(),
                created: true,
            });

            assert!(
                line.contains(flag),
                "the receipt must name the scope the way `--scope` takes it: {line:?}"
            );
            assert!(
                !line.contains(wire),
                "the wire spelling is not a value any caller can type back: {line:?}"
            );
        }
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
            scope_clamped: false,
            required_scope: None,
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
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse::page(
            vec![app_infra::brokered_access::BrokerTimelineInterval {
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
            1,
            None,
        ));

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
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse::page(
            vec![
                timeline_interval("frame", Some("f1.signature")),
                timeline_interval("audio_microphone", Some("a1.signature")),
                timeline_interval("audio_system", None),
            ],
            3,
            None,
        ));

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
            scope_clamped: false,
            required_scope: None,
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
            scope_clamped: false,
            required_scope: None,
        });
        assert_eq!(paged.next_cursor.as_deref(), Some("v1:42:1:0"));

        let last = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![search_result("f1")],
            limit: 20,
            next_cursor: None,
            speaker_coverage: None,
            scope_clamped: false,
            required_scope: None,
        });
        assert!(last.next_cursor.is_none());

        // Timeline has no cursor: it merges two independently-limited sources and
        // re-sorts, so a full page only reports that records may have been dropped.
        let timeline = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse::page(
            Vec::new(),
            0,
            None,
        ));
        assert!(timeline.truncated, "limit 0 can never be complete");
    }

    #[test]
    fn cli_accepts_the_speaker_surface() {
        Cli::try_parse_from(["mnema", "speakers"]).unwrap();
        Cli::try_parse_from(["mnema", "speakers", "--name", "priya", "--limit", "5"]).unwrap();
        Cli::try_parse_from([
            "mnema",
            "search",
            "--query",
            "standup",
            "--speaker",
            "p1.sig",
        ])
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
            "mnema",
            "search",
            "--query",
            "standup",
            "--speaker",
            "p1.sig",
            "--app",
            "Zoom",
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
            scope_clamped: false,
            required_scope: None,
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
                scope_clamped: false,
                required_scope: None,
            },
        ))
        .expect("search data should serialize");
        assert!(unfiltered["results"][0].get("turns").is_none());
        assert!(unfiltered.get("speakerCoverage").is_none());
    }

    #[test]
    fn search_mapping_carries_the_sub_segment_span_but_never_the_frame_id() {
        let data = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![
                app_infra::brokered_access::BrokerSearchResult {
                    kind: "audio_microphone".to_string(),
                    span_start_ms: Some(192_000),
                    span_end_ms: Some(198_000),
                    aligned_frame_id: Some(4_242),
                    ..search_result("a1.signature")
                },
                search_result("f1.signature"),
            ],
            limit: 2,
            next_cursor: None,
            speaker_coverage: None,
            scope_clamped: false,
            required_scope: None,
        });

        let json = serde_json::to_value(&data).expect("search data should serialize");
        // Without the span an agent can only say "somewhere in this recording".
        assert_eq!(json["results"][0]["spanStartMs"], 192_000);
        assert_eq!(json["results"][0]["spanEndMs"], 198_000);
        // A raw database id, which this surface never emits.
        assert!(json["results"][0].get("alignedFrameId").is_none());
        // A frame result has no sub-segment span; omitted, not null.
        assert!(json["results"][1].get("spanStartMs").is_none());
        assert!(json["results"][1].get("spanEndMs").is_none());
    }

    #[test]
    fn timeline_mapping_carries_speaker_turns_and_coverage() {
        let data = map_timeline_data(app_infra::brokered_access::BrokerTimelineResponse::page(
            vec![app_infra::brokered_access::BrokerTimelineInterval {
                turns: vec![speaker_turn("we ship on Friday")],
                ..timeline_interval("audio_microphone", Some("a1.signature"))
            }],
            1,
            Some(speaker_coverage()),
        ));

        let json = serde_json::to_value(&data).expect("timeline data should serialize");
        assert_eq!(
            json["intervals"][0]["turns"][0]["text"],
            "we ship on Friday"
        );
        assert_eq!(json["speakerCoverage"]["recordingsWithUnnamedVoices"], 3);
        assert_eq!(json["speakerCoverage"]["recordingsWithoutSpeakerData"], 7);

        let unfiltered = serde_json::to_value(map_timeline_data(
            app_infra::brokered_access::BrokerTimelineResponse::page(
                vec![timeline_interval("audio_microphone", Some("a1.signature"))],
                1,
                None,
            ),
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

    /// ONE rule for a nameless voice across both commands, because both are
    /// published in the same sentence of `.agents/skills/mnema-data/SKILL.md`:
    /// `speakers` entries carry "`name` (absent for an unnamed voice)" and
    /// `show-text` speakers carry "an optional `name` (absent for `unknown`)".
    /// `speakers` omits the key (asserted above); `show-text` must too, or an
    /// agent testing presence reads the same voice as named on one command and
    /// unnamed on the other.
    #[test]
    fn show_text_omits_the_name_of_an_unknown_speaker() {
        let data = ShowTextData {
            id: "a1.signature".to_string(),
            kind: "audio_microphone".to_string(),
            text: "we ship on Friday".to_string(),
            speakers: vec![BrokerSpeaker {
                name: None,
                attribution: "unknown".to_string(),
                confidence: None,
                handle: app_infra::brokered_access::BrokerSpeakerHandle {
                    id: "v1.signature".to_string(),
                    kind: "voice".to_string(),
                    start_ms: Some(0),
                    end_ms: Some(30_000),
                },
            }],
            turns: Vec::new(),
        };

        let json = serde_json::to_value(&data).expect("show-text data should serialize");
        assert!(
            json["speakers"][0].get("name").is_none(),
            "an unknown speaker omits `name`, never reports it as null: {json}"
        );
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
        let error = authorization_retry_result::<Option<AuthorizationGrant>>(
            app_unavailable_error(),
            Err(authorization_denied_error()),
        )
        .unwrap_err();

        assert_eq!(error.code, "authorization_denied");
        assert_eq!(error.exit, 10);
    }

    fn search_request_from(from: Option<String>) -> BrokeredCaptureRequest {
        BrokeredCaptureRequest::Search(BrokerSearchRequest {
            query: "invoice".to_string(),
            from,
            to: None,
            limit: None,
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            cursor: None,
            speaker: None,
        })
    }

    fn rfc3339_ms_ago(ms: u64) -> String {
        format_broker_unix_ms(now_unix_ms() - ms)
    }

    fn rfc3339_ms_ahead(ms: u64) -> String {
        format_broker_unix_ms(now_unix_ms() + ms)
    }

    const HOUR_MS: u64 = 60 * 60 * 1000;
    const DAY_MS: u64 = 24 * HOUR_MS;

    /// The `minimum` sent to the approval window. Under-derive it and the window
    /// offers an option that leaves the caller short; the user approves, the CLI
    /// retries, and the answer comes back quietly clipped.
    #[test]
    fn needed_scope_maps_each_from_band() {
        assert_eq!(
            needed_scope_for(&search_request_from(None)),
            BrokerGrantScope::LAST_DAY,
            "no --from asks for nothing older than today"
        );
        assert_eq!(
            needed_scope_for(&search_request_from(Some(rfc3339_ms_ago(3 * HOUR_MS)))),
            BrokerGrantScope::LAST_DAY
        );
        assert_eq!(
            needed_scope_for(&search_request_from(Some(rfc3339_ms_ago(3 * DAY_MS)))),
            BrokerGrantScope::LAST_7_DAYS
        );
        assert_eq!(
            needed_scope_for(&search_request_from(Some(rfc3339_ms_ago(30 * DAY_MS)))),
            BrokerGrantScope::AllRetainedHistory
        );
        // An unparseable bound is the broker's error to report; the CLI must not
        // silently escalate the ask on the strength of a typo.
        assert_eq!(
            needed_scope_for(&search_request_from(Some("last tuesday".to_string()))),
            BrokerGrantScope::LAST_DAY
        );

        let timeline = BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
            from: rfc3339_ms_ago(14 * DAY_MS),
            to: rfc3339_ms_ago(0),
            limit: None,
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            speaker: None,
        });
        assert_eq!(
            needed_scope_for(&timeline),
            BrokerGrantScope::AllRetainedHistory
        );
    }

    /// WHICH bound a date is decides how it is priced, and the two rules are not
    /// interchangeable. A window START is served to within the broker's clamp
    /// slack, so "24 hours ago" — a bound the caller computes and the broker
    /// evaluates milliseconds later, over the band edge by construction — must
    /// still cost `lastDay`: that is the permission which answers it without
    /// reporting a clamp, which is the whole point. Priced on the exact edge it
    /// costs `last7Days`, and that answer is both the `requiredScope` an agent
    /// reads and the approval window's floor — so the most ordinary query in each
    /// band interrupts the user for a standing permission one band wider than the
    /// call needs. A `--to` fallback must NOT get the slack: a `--to` under the
    /// permission's scope start is refused outright rather than clamped, so the
    /// ask has to reach PAST it or the approved retry dies at exit 13 forever.
    #[test]
    fn a_window_start_is_priced_with_the_clamp_slack_and_a_to_bound_is_not() {
        assert_eq!(
            needed_scope_for(&search_request_from(Some(rfc3339_ms_ago(DAY_MS + 250)))),
            BrokerGrantScope::LAST_DAY,
            "a lastDay permission serves `--from` \"24 hours ago\" without clamping it, \
             so that is all the CLI may ask the user for"
        );
        assert_eq!(
            needed_scope_for(&search_request_from(Some(rfc3339_ms_ago(7 * DAY_MS + 250)))),
            BrokerGrantScope::LAST_7_DAYS,
            "`--from` \"7 days ago\" is the query a last7Days permission exists for"
        );

        let mut bounded_only_by_to = search_request_from(None);
        if let BrokeredCaptureRequest::Search(request) = &mut bounded_only_by_to {
            request.to = Some(rfc3339_ms_ago(DAY_MS + 250));
        }
        assert_eq!(
            needed_scope_for(&bounded_only_by_to),
            BrokerGrantScope::LAST_7_DAYS,
            "a `--to` that old is refused, not clamped, by a lastDay permission — the \
             slack belongs on the window start alone, never in `minimum_scope_for_start`"
        );
    }

    fn approved_grant(scope: &str) -> AuthorizationGrant {
        AuthorizationGrant {
            id: "abcdef".to_string(),
            client_label: "Claude Code".to_string(),
            scope: scope.to_string(),
            created: false,
        }
    }

    /// `approved` is not a yes. The window enforces only the request's `minimum`,
    /// so an approval can land narrower than the call needs — retrying on that
    /// returns a clipped page the caller reads as the whole answer.
    #[test]
    fn a_narrowed_grant_fails_instead_of_retrying() {
        let error = verify_granted_scope(
            BrokerGrantScope::AllRetainedHistory,
            Some(&approved_grant("lastDay")),
        )
        .expect_err("a lastDay approval does not cover an all-retained request");
        assert_eq!(error.code, "scope_not_granted");
        assert_eq!(error.exit, 22);
        assert!(!error.retryable);
        assert!(
            error.message.contains("last-day") && error.message.contains("all-retained"),
            "the failure names what was granted AND what was needed: {}",
            error.message
        );

        verify_granted_scope(
            BrokerGrantScope::LAST_7_DAYS,
            Some(&approved_grant("allRetained")),
        )
        .expect("a wider approval covers a narrower request");
        verify_granted_scope(
            BrokerGrantScope::LAST_7_DAYS,
            Some(&approved_grant("last7Days")),
        )
        .expect("an exact approval covers the request");
        // Nothing to verify against: retry and let the broker's clamp marker
        // speak, rather than failing a call the permission may well cover.
        verify_granted_scope(BrokerGrantScope::LAST_DAY, None).expect("no grant field, no verdict");
    }

    /// Every reason gets its own message and exit: collapsing them told a user
    /// with unfinished onboarding that Mnema was unavailable, and had the CLI
    /// relaunch an app that was only mid-approval.
    #[test]
    fn each_authorization_reason_maps_to_its_own_exit() {
        let cases = [
            (
                ("denied", Some("userCancelled")),
                "authorization_denied",
                10,
            ),
            (
                ("denied", Some("closed")),
                "authorization_window_closed",
                14,
            ),
            (("blocked", Some("blocked")), "access_blocked", 15),
            (("unavailable", Some("busy")), "authorization_busy", 16),
            (
                ("unavailable", Some("onboardingRequired")),
                "onboarding_required",
                17,
            ),
            (
                ("unavailable", Some("invalidRequest")),
                "authorization_invalid_request",
                18,
            ),
            (
                ("unavailable", Some("unsupportedVersion")),
                "authorization_unsupported_version",
                19,
            ),
            (("unavailable", None), "app_unavailable", 12),
        ];
        let mut exits = Vec::new();
        for ((decision, reason), code, exit) in cases {
            let error = authorization_response_error(decision, reason);
            assert_eq!(error.code, code, "{decision}/{reason:?}");
            assert_eq!(error.exit, exit, "{decision}/{reason:?}");
            exits.push(error.exit);
        }
        let mut unique = exits.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), exits.len(), "each reason needs its OWN exit");

        // `blocked` is emitted on both fields; either one is the standing
        // rejection, and neither may look retryable.
        for reason in [Some("blocked"), None] {
            assert_eq!(
                authorization_response_error("blocked", reason).code,
                "access_blocked"
            );
        }
        assert!(!authorization_response_error("blocked", Some("blocked")).retryable);
    }

    /// Only a socket that never answered means "the app is not running". `busy`
    /// and `onboardingRequired` are the app ANSWERING, and relaunching it there
    /// steals focus from the approval the user is already looking at.
    #[test]
    fn only_an_unanswered_channel_relaunches_the_app() {
        for reason in ["busy", "onboardingRequired", "invalidRequest", "blocked"] {
            let error = authorization_response_error("unavailable", Some(reason));
            assert!(
                !should_retry_authorization_with_app_launch(&error),
                "{reason} must not relaunch the app"
            );
        }
        assert!(should_retry_authorization_with_app_launch(
            &app_unavailable_error()
        ));
    }

    /// A clamped page is data plus a warning, and the CLI has to carry BOTH: the
    /// marker is the only thing separating "your access stops here" from "nothing
    /// happened in that window".
    #[test]
    fn the_clamp_marker_survives_into_the_envelope() {
        let clamped = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![search_result("f1.signature")],
            limit: 1,
            next_cursor: None,
            speaker_coverage: None,
            scope_clamped: true,
            required_scope: Some("allRetained".to_string()),
        });
        let json = serde_json::to_value(&clamped).expect("search data should serialize");
        assert_eq!(json["scopeClamped"], true);
        assert_eq!(json["requiredScope"], "allRetained");

        let whole = map_search_data(app_infra::brokered_access::BrokerSearchResponse {
            results: vec![search_result("f1.signature")],
            limit: 1,
            next_cursor: None,
            speaker_coverage: None,
            scope_clamped: false,
            required_scope: None,
        });
        let json = serde_json::to_value(&whole).expect("search data should serialize");
        // Always present, never null: an agent testing `scopeClamped === false`
        // must not read a missing key as "unknown".
        assert_eq!(json["scopeClamped"], false);
        assert!(json.get("requiredScope").is_none());
    }

    /// The trigger for the widen prompt. Reading it off the response is what lets
    /// the agent widen NOW instead of on some later run.
    #[test]
    fn a_clamped_response_is_what_triggers_the_widen_prompt() {
        let clamped =
            BrokeredCaptureResponse::Search(app_infra::brokered_access::BrokerSearchResponse {
                results: Vec::new(),
                limit: 1,
                next_cursor: None,
                speaker_coverage: None,
                scope_clamped: true,
                required_scope: Some("last7Days".to_string()),
            });
        assert!(response_scope_clamped(&clamped));
        assert!(!response_requires_authorization(&clamped));

        let timeline = BrokeredCaptureResponse::Timeline(
            app_infra::brokered_access::BrokerTimelineResponse::page(Vec::new(), 1, None),
        );
        assert!(!response_scope_clamped(&timeline));
    }

    /// `status` reports what a standing permission actually has: a scope, a last
    /// use, and whether it is blocked. There is no expiry left to print.
    #[test]
    fn access_status_reports_scope_last_use_and_blocked_state() {
        let mut grant = BrokerGrant {
            id: "abcdef".to_string(),
            label: "Claude Code".to_string(),
            normalized_label: "claude code".to_string(),
            identity_source: BrokerClientIdentitySource::Inferred,
            created_at_unix_ms: 1_700_000_000_000,
            last_used_at_unix_ms: 1_700_000_000_000,
            scope: BrokerGrantScope::LAST_7_DAYS,
            blocked: false,
            blocked_at_unix_ms: None,
        };

        let line = access_status_line(&grant);
        assert!(line.contains("Claude Code"), "{line}");
        assert!(line.contains("last-7-days"), "{line}");
        assert!(line.contains("2023-11-14"), "{line}");
        assert!(!line.contains("expire"), "there is no expiry: {line}");

        grant.blocked = true;
        assert!(access_status_line(&grant).contains("blocked"));
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

    /// The transport door. The search/timeline range check raises this as an
    /// `Err` from app-infra, never as a `BrokerErrorResponse` — it reached the
    /// CLI as `broker_operation_failed` (20) until the classification moved into
    /// `broker_failure`, which told agents Mnema had faulted when the real answer
    /// was "widen the scope".
    #[test]
    fn out_of_scope_window_maps_to_outside_grant_scope() {
        let error = broker_error(app_infra::AppInfraError::InvalidSearchRequest(
            "requested broker time range is outside the grant scope".to_string(),
        ));
        assert_eq!(error.code, "outside_grant_scope");
        assert_eq!(error.exit, 13);
        assert!(!error.retryable);
    }

    /// The response door, and the negative case that keeps the substring from
    /// swallowing every unrelated broker failure into exit 13.
    #[test]
    fn out_of_scope_result_id_maps_to_outside_grant_scope() {
        let error = map_broker_response_error(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "result is unavailable or outside the grant scope".to_string(),
        });
        assert_eq!(error.code, "outside_grant_scope");
        assert_eq!(error.exit, 13);

        let other = broker_failure("unexpected search response");
        assert_eq!(other.code, "broker_operation_failed");
        assert_eq!(other.exit, 20);
    }

    /// A live broker over a throwaway config/index, so the widen flow can be
    /// exercised end to end: `execute_data_request` -> real permission file ->
    /// real query -> approval channel.
    struct BrokerFixture {
        _config: tempfile::TempDir,
        _save: tempfile::TempDir,
        _keys: tempfile::TempDir,
        _env: std::sync::MutexGuard<'static, ()>,
    }

    /// The fixture points three process-global env vars at its temp dirs, so only
    /// one of these tests may run at a time.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    async fn broker_fixture() -> BrokerFixture {
        let guard = env_lock();
        let config = tempfile::tempdir().expect("config dir");
        let save = tempfile::tempdir().expect("save dir");
        let keys = tempfile::tempdir().expect("key dir");
        env::set_var("MNEMA_APP_CONFIG_DIR", config.path());
        env::set_var("MNEMA_SAVE_DIRECTORY", save.path());
        env::set_var("MNEMA_CAPTURE_INDEX_KEY_DIR", keys.path());
        // The owner path creates the schema; the brokered reader never migrates.
        app_infra::AppInfra::initialize(save.path())
            .await
            .expect("owner infra initializes");
        BrokerFixture {
            _config: config,
            _save: save,
            _keys: keys,
            _env: guard,
        }
    }

    fn timeline_request(from: String, to: String) -> BrokeredCaptureRequest {
        BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
            from,
            to,
            limit: None,
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            speaker: None,
        })
    }

    /// Every request the CLI put on the approval channel, oldest first. Recorded
    /// BEFORE the reply is written, so anything the CLI has a verdict for is
    /// already in here by the time its call returns — which is what makes "exactly
    /// one approval window per command" assertable.
    #[cfg(unix)]
    type ApprovalLog = std::sync::Arc<std::sync::Mutex<Vec<AuthorizationRequest>>>;

    /// Stands in for the app's approval channel: answers EVERY request that
    /// arrives with whatever `reply` builds, and records it.
    #[cfg(unix)]
    fn serve_verdicts(
        reply: impl Fn(&AuthorizationRequest) -> serde_json::Value + Send + 'static,
    ) -> ApprovalLog {
        let listener = tokio::net::UnixListener::bind(authorization_socket_path())
            .expect("approval socket should bind");
        let asked: ApprovalLog = Default::default();
        let log = asked.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let mut reader = BufReader::new(stream);
                let mut raw = String::new();
                reader.read_line(&mut raw).await.expect("request line");
                let request: AuthorizationRequest =
                    serde_json::from_str(&raw).expect("the request should parse");
                let response = reply(&request);
                log.lock().expect("approval log").push(request);
                reader
                    .into_inner()
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .expect("response writes");
            }
        });
        asked
    }

    /// Like the real window: upsert the permission, then reply `approved` with
    /// the scope the row now carries.
    #[cfg(unix)]
    fn serve_one_approval(
        identity: BrokerClientIdentity,
        granted: BrokerGrantScope,
    ) -> ApprovalLog {
        serve_verdicts(move |request| {
            let grant = BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
                .expect("broker resolves")
                .upsert_grant_for_identity(identity.clone(), granted)
                .expect("the approval widens the permission")
                .grant;
            serde_json::json!({
                "schemaVersion": 1,
                "requestId": request.request_id,
                "decision": "approved",
                "grant": {
                    "id": grant.id,
                    "clientLabel": grant.label,
                    "scope": grant.scope.wire_name(),
                    "created": false,
                },
            })
        })
    }

    /// Like the real window with its pre-selection accepted: grant exactly what
    /// the request put in `minimum`. The window pre-selects the `minimum` (never
    /// the `preferred` — that would open `--scope all-retained` with full history
    /// already selected), so this is the answer a user gives by clicking Allow
    /// without touching the picker — and the one that shows whether the default
    /// answer can satisfy the call it was opened for.
    #[cfg(unix)]
    fn serve_default_approval(identity: BrokerClientIdentity) -> ApprovalLog {
        serve_verdicts(move |request| {
            let scope = BrokerGrantScope::from_wire_name(&request.scope.minimum)
                .expect("the CLI must ask for a scope the window can grant");
            let grant = BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
                .expect("broker resolves")
                .upsert_grant_for_identity(identity.clone(), scope)
                .expect("the approval widens the permission")
                .grant;
            serde_json::json!({
                "schemaVersion": 1,
                "requestId": request.request_id,
                "decision": "approved",
                "grant": {
                    "id": grant.id,
                    "clientLabel": grant.label,
                    "scope": grant.scope.wire_name(),
                    "created": false,
                },
            })
        })
    }

    /// `search` takes a `--to` with no `--from`, and the broker then starts the
    /// window at the permission's scope start — so a `--to` older than that start
    /// ends the range before it begins and is refused on the transport. The widen
    /// door is reached, but an ask derived from the ABSENT `--from` comes out
    /// `lastDay`: the one scope that cannot satisfy the call. The user would be
    /// shown a consent window, approve the pre-selection, and still die at exit 13
    /// — on this run and every run after it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_search_bounded_only_by_to_asks_for_a_scope_that_can_satisfy_it() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Backdated Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let window = serve_default_approval(identity.clone());
        let mut request = search_request_from(None);
        if let BrokeredCaptureRequest::Search(request) = &mut request {
            request.to = Some(rfc3339_ms_ago(30 * DAY_MS));
        }
        let value = execute_data_request("search", &identity, request, true)
            .await
            .expect("the widened retry must reach a window bounded only by --to");

        let asked = window.lock().expect("approval log");
        let asked = asked.first().expect("the approval window was opened");
        assert_eq!(
            asked.scope.preferred, "allRetained",
            "the ask must cover the requested window's END too, not just its \
             (absent) start: {value}"
        );
    }

    /// `broker_failure` classifies EVERY broker error, and some of those messages
    /// quote caller-supplied text straight back. `execute_data_request` opens the
    /// approval window on the `outside_grant_scope` code alone, so a substring
    /// match there is a forgeable consent trigger: a crafted `--url-regex` pops a
    /// standing-permission window for a request that has nothing to do with scope,
    /// and reports exit 13 for what is really a bad regex.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_broker_error_quoting_the_scope_phrase_opens_no_approval_window() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Crafty Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let window = serve_default_approval(identity.clone());
        let mut request = search_request_from(None);
        if let BrokeredCaptureRequest::Search(request) = &mut request {
            // Invalid on purpose: the broker's message echoes the pattern back.
            request.url_regex = Some("(outside the grant scope".to_string());
        }
        let error = execute_data_request("search", &identity, request, true)
            .await
            .expect_err("an invalid urlRegex is a request error");

        assert!(
            error.message.contains("outside the grant scope"),
            "the message this test forges must actually carry the phrase: {}",
            error.message
        );
        assert!(
            window.lock().expect("approval log").is_empty(),
            "no consent window may open for a request that never asked for scope: {}",
            error.message
        );
        assert_eq!(
            error.code, "broker_operation_failed",
            "a quoted phrase is not the broker's out-of-scope verdict: {}",
            error.message
        );
    }

    /// A window that has not happened yet is as empty as an inverted one: the
    /// broker ends every window at `min(--to, now)`, so a `--from` past that end is
    /// refused with the same "outside the grant scope" a too-narrow permission
    /// produces. Reading that as "widen the permission" interrupts the user with an
    /// approval window that cannot help — an agent asking about "tomorrow" is
    /// enough to fire it — and the approved retry dies at exit 13 anyway.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_empty_by_construction_window_never_opens_an_approval_window() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Future Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::AllRetainedHistory)
            .expect("the widest permission there is — no widen could ever help");

        let window = serve_one_approval(identity.clone(), BrokerGrantScope::AllRetainedHistory);
        let error = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ahead(DAY_MS), rfc3339_ms_ahead(2 * DAY_MS)),
            true,
        )
        .await
        .expect_err("a window that has not happened yet is a usage error");

        assert_eq!(
            window.lock().expect("approval log").len(),
            0,
            "no approval can make tomorrow readable today"
        );
        assert_eq!(
            error.exit, 2,
            "a window with nothing in it by construction is a usage error"
        );

        // The caller's own contradiction takes the same door, and never the widen.
        let inverted = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(DAY_MS), rfc3339_ms_ago(2 * DAY_MS)),
            true,
        )
        .await
        .expect_err("--from after --to is empty by construction");
        assert_eq!(inverted.exit, 2);
        assert_eq!(
            window.lock().expect("approval log").len(),
            0,
            "an inverted window is not a scope problem either"
        );

        // `search` reaches the same refusal with `--from` alone: the broker ends an
        // open window at `now`, so a future start is already past its end. A window
        // that STARTED in the past stays answerable.
        assert!(
            empty_window_message(&search_request_from(Some(rfc3339_ms_ahead(HOUR_MS)))).is_some(),
            "a future --from with no --to is empty by construction too"
        );
        assert!(
            empty_window_message(&search_request_from(Some(rfc3339_ms_ago(2 * DAY_MS)))).is_none(),
            "a window that started in the past is answerable"
        );
    }

    /// A window that ENDS before the permission's scope starts is refused
    /// outright instead of clamped, so it never reaches the clamp door that opens
    /// the approval window. `timeline` REQUIRES `--to`, so every dated timeline
    /// question past the permission dies with no way to widen — while the
    /// identical open-ended window (`--to now`) does get the prompt. Whether a
    /// caller can widen must not depend on whether it bounded its question.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_bounded_out_of_scope_window_still_opens_the_approval_window() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Bounded Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let window = serve_one_approval(identity.clone(), BrokerGrantScope::AllRetainedHistory);
        let value = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(20 * DAY_MS)),
            true,
        )
        .await
        .expect("a window past the permission must ask to widen, not fail outright");

        assert_eq!(
            value["scopeClamped"], false,
            "after the widen the whole requested window is in reach: {value}"
        );
        let asked = window.lock().expect("approval log");
        let asked = asked.first().expect("the approval window was opened");
        assert_eq!(
            asked.scope.preferred, "allRetained",
            "the ask must cover the WHOLE requested window, not just its start"
        );
    }

    /// A clamped page is data the client's STANDING permission already covers,
    /// and the widen is an optional extra on top of it. When that widen never
    /// reaches a verdict — the app is mid-approval for someone else — discarding
    /// the page turns a partial answer into a total failure, which is exactly the
    /// "nothing happened in that window" report the clamp marker exists to stop.
    /// `--no-prompt` on the identical call already hands the page back.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_widen_that_never_answered_keeps_the_clamped_page() {
        let _fixture = broker_fixture().await;
        let identity = BrokerClientIdentity::new("Busy Tool", BrokerClientIdentitySource::Explicit)
            .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let window = serve_verdicts(|request| {
            serde_json::json!({
                "schemaVersion": 1,
                "requestId": request.request_id,
                "decision": "unavailable",
                "reason": "busy",
            })
        });
        let value = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(0)),
            true,
        )
        .await
        .expect("a busy approval channel must not destroy the page already in hand");

        assert_eq!(
            value["scopeClamped"], true,
            "the page is still short of what was asked for, and must say so: {value}"
        );
        assert_eq!(value["requiredScope"], "allRetained");
        assert_eq!(
            window.lock().expect("approval log").len(),
            1,
            "the widen was attempted exactly once"
        );
    }

    /// The scope a request with no `--from` asks for. Every non-dated command
    /// (`show-text`, `open`, `speakers`) lands on this arm, so it decides what
    /// the approval window offers for the commands that carry an opaque id from
    /// an older page — pinned so the choice is deliberate rather than the
    /// leftover of the `_ => None` fall-through.
    #[test]
    fn an_undated_request_asks_only_for_the_last_day() {
        for request in [
            BrokeredCaptureRequest::ShowText {
                opaque_id: "f1.signature".to_string(),
            },
            BrokeredCaptureRequest::OpenInMnema {
                opaque_id: "f1.signature".to_string(),
            },
            BrokeredCaptureRequest::Speakers(app_infra::brokered_access::BrokerSpeakersRequest {
                name: None,
                limit: None,
            }),
        ] {
            assert_eq!(
                needed_scope_for(&request),
                BrokerGrantScope::LAST_DAY,
                "an undated command asks for the narrowest band: {request:?}"
            );
        }
    }

    /// `--no-prompt` says "do not bother anyone", not "throw the answer away".
    /// A permission too narrow for the `--from` still returns real data for the
    /// part it covers, and the clamp marker is what stops an agent reading that
    /// thin page as "nothing happened". Nothing is bound to the approval socket
    /// here, so a prompt attempt would surface as an error rather than a page.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_clamped_page_with_no_prompt_returns_the_data_and_its_marker() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Quiet Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let value = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(0)),
            false,
        )
        .await
        .expect("a clamped page is data, not a failure");

        assert_eq!(
            value["scopeClamped"], true,
            "the page covers less than was asked for and must say so: {value}"
        );
        assert_eq!(value["requiredScope"], "allRetained");
    }

    /// The clamp door, end to end: one approval window, the permission widens,
    /// the call is retried once and comes back whole.
    ///
    /// Also the pin for what that one window is allowed to offer on the WIDEN
    /// door: `minimum` is the derived scope, because the row already stands
    /// narrower than it and an approval SETS the row's scope — so every option
    /// below the ask both fails this call and takes away access the tool already
    /// had. (The first-prompt door keeps the narrow options live; see
    /// `a_first_grant_still_offers_every_narrower_answer`.)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_clamp_widens_and_retries_once_then_returns_the_full_page() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Widening Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER)
            .expect("broker resolves")
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_DAY)
            .expect("the client starts on a one-day permission");

        let window = serve_one_approval(identity.clone(), BrokerGrantScope::AllRetainedHistory);
        let value = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(0)),
            true,
        )
        .await
        .expect("the widened retry must succeed");

        assert_eq!(
            value["scopeClamped"], false,
            "the retry after the widen covers the whole requested window: {value}"
        );
        let asked = window.lock().expect("approval log");
        assert_eq!(
            asked.len(),
            1,
            "at most ONE approval window per command — a second is reflex-clicking"
        );
        let asked = &asked[0];
        assert_eq!(
            asked.scope.minimum, "allRetained",
            "on a widen the floor is what the call needs — a narrower answer can \
             only fail it, and would narrow the row it was opened to widen"
        );
        assert_eq!(
            asked.scope.preferred, "allRetained",
            "preferred carries what the call actually needs"
        );
    }

    /// A widen prompt approved at its DEFAULT must not narrow the permission it
    /// was opened to widen.
    ///
    /// The window pre-selects the request's `minimum`, and an approval SETS the
    /// row's scope. So a `minimum` pinned below what the call needs made the
    /// default answer doubly wrong for a client that already held something: the
    /// row was narrowed (`last7Days` → `lastDay`, access the tool had and did not
    /// lose to any user decision) and the command STILL died at exit 22, because
    /// the granted scope did not cover the request either.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_widen_approved_at_its_default_never_narrows_the_permission() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Weekly Tool", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        let broker =
            BrokeredCaptureAccess::from_app_identifier(APP_IDENTIFIER).expect("broker resolves");
        broker
            .upsert_grant_for_identity(identity.clone(), BrokerGrantScope::LAST_7_DAYS)
            .expect("the client starts on a seven-day permission");

        let window = serve_default_approval(identity.clone());
        let value = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(0)),
            true,
        )
        .await
        .expect("the answer a user gives by clicking Allow must satisfy the call");

        assert_eq!(
            value["scopeClamped"], false,
            "the default answer has to cover the whole requested window: {value}"
        );
        let scope = broker
            .list_grants()
            .expect("grants read")
            .grants
            .iter()
            .find(|grant| grant.normalized_label == identity.normalized_label)
            .expect("the row survives the widen")
            .scope;
        assert_eq!(
            scope,
            BrokerGrantScope::AllRetainedHistory,
            "an approval may widen a permission, never narrow one"
        );
        let asked = window.lock().expect("approval log");
        assert_eq!(
            asked
                .first()
                .expect("the approval window was opened")
                .scope
                .minimum,
            "allRetained",
            "the floor the window enforces is what the call needs"
        );
    }

    /// The other door, and the reason the widen floor cannot simply be the ask
    /// everywhere: a tool with NO permission yet is a different decision. The
    /// window disables every option below `minimum`, so a never-seen tool passing
    /// `--from 1970-01-01` would otherwise make "All retained" the only answer the
    /// user can give on its first prompt. The narrow answers stay live, and an
    /// approval that lands under the ask fails the call (exit 22) instead of
    /// silently under-serving it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_first_grant_still_offers_every_narrower_answer() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Unknown Tool", BrokerClientIdentitySource::Inferred)
                .expect("identity");

        let window = serve_default_approval(identity.clone());
        let error = execute_data_request(
            "timeline",
            &identity,
            timeline_request(rfc3339_ms_ago(30 * DAY_MS), rfc3339_ms_ago(0)),
            true,
        )
        .await
        .expect_err("a grant narrower than the call must not be retried into");

        assert_eq!(error.code, "scope_not_granted");
        assert_eq!(error.exit, 22);
        let asked = window.lock().expect("approval log");
        let asked = asked.first().expect("the approval window was opened");
        assert_eq!(
            asked.scope.minimum, "lastDay",
            "a first prompt must leave the user every narrower answer"
        );
        assert_eq!(
            asked.scope.preferred, "allRetained",
            "preferred still states what the call needs"
        );
    }

    /// `--no-prompt` is the ONE control for "do not bother anyone" now that the
    /// TTY gate is gone, and the MCP door is a door like any other: a server
    /// started with the flag must open no approval window. Authorization is
    /// unaffected either way — all five MCP tools route through
    /// `execute_data_request`.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_mcp_door_honours_no_prompt() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Quiet MCP Client", BrokerClientIdentitySource::Explicit)
                .expect("identity");
        let speakers = || {
            BrokeredCaptureRequest::Speakers(app_infra::brokered_access::BrokerSpeakersRequest {
                name: None,
                limit: None,
            })
        };
        let window = serve_one_approval(identity.clone(), BrokerGrantScope::LAST_DAY);

        let quiet = crate::mcp::call_tool_for_tests(&identity, false, "speakers", speakers()).await;
        assert_eq!(quiet.is_error, Some(true));
        assert!(
            format!("{quiet:?}").contains("authorization_required"),
            "the tool error must name the missing permission: {quiet:?}"
        );
        assert!(
            window.lock().expect("approval log").is_empty(),
            "--no-prompt must not open an approval window on the MCP door"
        );

        // ...and without it the same call does reach the window.
        crate::mcp::call_tool_for_tests(&identity, true, "speakers", speakers()).await;
        assert_eq!(
            window.lock().expect("approval log").len(),
            1,
            "prompting is still the default on the MCP door"
        );
    }

    /// A verdict the app answers from its permission file alone — `blocked`,
    /// `busy`, `onboardingRequired` — lands inside the announce beat, and there is
    /// no window behind it (for `blocked` there never can be). Saying "Opening
    /// Mnema..." there sends the user looking for a window that will not appear.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_instant_verdict_never_announces_a_window() {
        let (mut app, cli) = UnixStream::pair().expect("socket pair");
        app.write_all(b"{\"schemaVersion\":1,\"requestId\":\"r\",\"decision\":\"blocked\"}\n")
            .await
            .expect("the app answers instantly");
        let announced = std::cell::Cell::new(false);
        let mut reader = BufReader::new(cli);
        let line = read_verdict_line(&mut reader, || announced.set(true))
            .await
            .expect("the verdict reads");
        assert!(
            !announced.get(),
            "a windowless verdict must not promise a window: {line}"
        );
        assert!(line.contains("blocked"));

        // The other half of the beat: a request the app is still sitting on DOES
        // have a window, and the announce is the only warning the caller gets.
        let (mut app, cli) = UnixStream::pair().expect("socket pair");
        tokio::spawn(async move {
            tokio::time::sleep(WINDOW_ANNOUNCE_DELAY * 2).await;
            let _ = app
                .write_all(b"{\"schemaVersion\":1,\"requestId\":\"r\",\"decision\":\"approved\"}\n")
                .await;
        });
        let announced = std::cell::Cell::new(false);
        let mut reader = BufReader::new(cli);
        read_verdict_line(&mut reader, || announced.set(true))
            .await
            .expect("the verdict reads");
        assert!(
            announced.get(),
            "a verdict the app sat on means a window is open, and the caller is told"
        );
    }

    /// `access request` is the one command a human runs deliberately, and
    /// `--no-prompt` is the one control for "do not bother anyone" — so the pair
    /// is a contradiction that fails before anything is opened. Nothing is bound
    /// to the approval socket here: an attempt to reach the app would come back
    /// `app_unavailable` (12), not `authorization_required` (10).
    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_access_request_is_allowed_without_interactive_stdio() {
        let _fixture = broker_fixture().await;
        let identity =
            BrokerClientIdentity::new("Deliberate Human", BrokerClientIdentitySource::Explicit)
                .expect("identity");

        let error = run_access_command(
            AccessCommand::Request {
                scope: AccessScope::LastDay,
            },
            &identity,
            true,
        )
        .await
        .expect_err("--no-prompt cannot ask for an approval window");
        assert_eq!(error.code, "authorization_required");
        assert_eq!(error.exit, 10);
        assert!(error.retryable);

        // ...and the same command without it does reach the channel.
        let window = serve_one_approval(identity.clone(), BrokerGrantScope::LAST_DAY);
        run_access_command(
            AccessCommand::Request {
                scope: AccessScope::LastDay,
            },
            &identity,
            false,
        )
        .await
        .expect("a prompting access request is approved");
        assert_eq!(window.lock().expect("approval log").len(), 1);
    }
}
