//! Local MCP server over stdio: the same five brokered data commands the CLI
//! exposes, as MCP tools for chat clients (Claude Desktop, Cursor, ...).
//! Consent, redaction, and grant enforcement all stay in the app's broker.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerSearchRequest, BrokerSpeakersRequest, BrokerTimelineRequest,
    BrokeredCaptureRequest,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use serde::Deserialize;

use crate::{broker_error, execute_data_request, CliError};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// Full-text query over captured screen text and audio transcripts.
    query: String,
    /// RFC3339 lower time bound, e.g. 2026-07-21T09:00:00Z.
    from: Option<String>,
    /// RFC3339 upper time bound.
    to: Option<String>,
    /// Maximum number of results.
    limit: Option<u32>,
    /// Filter by application name, e.g. "Linear".
    app: Option<String>,
    /// Filter by window title.
    window_title: Option<String>,
    /// Case-insensitive substring of the page URL, matched against the sanitized
    /// host/path form (query strings and fragments are never indexed).
    url: Option<String>,
    /// Case-sensitive regular expression over the same sanitized host/path URL
    /// (prefix with `(?i)` for case-insensitive matching); mutually exclusive with url.
    url_regex: Option<String>,
    /// Opaque speaker handle from the speakers tool, narrowing to audio that
    /// person or voice was heard in and returning their words inline. Matches
    /// assigned voices AND recognition guesses. Cannot be combined with app,
    /// window_title, url, or url_regex.
    speaker: Option<String>,
    /// nextCursor from a previous search response, to fetch the next page of the
    /// same query. Re-send the identical query and filters alongside it.
    cursor: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TimelineParams {
    /// RFC3339 lower time bound, e.g. 2026-07-21T09:00:00Z.
    from: String,
    /// RFC3339 upper time bound.
    to: String,
    /// Maximum number of intervals.
    limit: Option<u32>,
    /// Filter by application name, e.g. "Linear".
    app: Option<String>,
    /// Filter by window title.
    window_title: Option<String>,
    /// Case-insensitive substring of the page URL, matched against the sanitized
    /// host/path form (query strings and fragments are never indexed).
    url: Option<String>,
    /// Case-sensitive regular expression over the same sanitized host/path URL
    /// (prefix with `(?i)` for case-insensitive matching); mutually exclusive with url.
    url_regex: Option<String>,
    /// Opaque speaker handle from the speakers tool, narrowing the window to
    /// audio that person or voice was heard in. Cannot be combined with app,
    /// window_title, url, or url_regex.
    speaker: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SpeakersParams {
    /// Case-insensitive substring of a person's name; named people only. Use it
    /// to find someone who ranks below the limit.
    name: Option<String>,
    /// Maximum number of speakers to return.
    limit: Option<u32>,
}

/// `url` and `url_regex` are mutually exclusive. Clap enforces that on the CLI
/// door; without this the MCP door had no guard at all and the broker would AND
/// both predicates together — a narrower filter than any client asked for, and
/// usually zero rows, which reads to an agent as "nothing was captured".
fn reject_both_url_filters(
    url: &Option<String>,
    url_regex: &Option<String>,
) -> Result<(), ErrorData> {
    if url.is_some() && url_regex.is_some() {
        return Err(ErrorData::invalid_params(
            "url and url_regex are mutually exclusive; pass only one",
            None,
        ));
    }
    Ok(())
}

/// `speaker` excludes every screen filter. Clap refuses the pair on the CLI door
/// before the round trip; without this the MCP door would let it reach the
/// broker, which rejects it as a plain operation failure — and only after an
/// approval dialog when no grant is active yet, for a request that can never
/// succeed. Both published contracts say the door refuses it, so both doors do.
fn reject_speaker_beside_screen_filters(
    speaker: &Option<String>,
    app: &Option<String>,
    window_title: &Option<String>,
    url: &Option<String>,
    url_regex: &Option<String>,
) -> Result<(), ErrorData> {
    if speaker.is_some()
        && (app.is_some() || window_title.is_some() || url.is_some() || url_regex.is_some())
    {
        return Err(ErrorData::invalid_params(
            "speaker cannot be combined with app, window_title, url, or url_regex: a speaker \
             filter matches recorded audio, and audio carries no app, window title, or url to \
             match against — ask for the speaker first, then search the screen filters over the \
             times it returns",
            None,
        ));
    }
    Ok(())
}

impl SearchParams {
    fn into_request(self) -> Result<BrokeredCaptureRequest, ErrorData> {
        reject_both_url_filters(&self.url, &self.url_regex)?;
        reject_speaker_beside_screen_filters(
            &self.speaker,
            &self.app,
            &self.window_title,
            &self.url,
            &self.url_regex,
        )?;
        Ok(BrokeredCaptureRequest::Search(BrokerSearchRequest {
            query: self.query,
            from: self.from,
            to: self.to,
            limit: self.limit,
            app: self.app,
            window_title: self.window_title,
            url: self.url,
            url_regex: self.url_regex,
            cursor: self.cursor,
            speaker: self.speaker,
        }))
    }
}

impl TimelineParams {
    fn into_request(self) -> Result<BrokeredCaptureRequest, ErrorData> {
        reject_both_url_filters(&self.url, &self.url_regex)?;
        reject_speaker_beside_screen_filters(
            &self.speaker,
            &self.app,
            &self.window_title,
            &self.url,
            &self.url_regex,
        )?;
        Ok(BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
            from: self.from,
            to: self.to,
            limit: self.limit,
            app: self.app,
            window_title: self.window_title,
            url: self.url,
            url_regex: self.url_regex,
            speaker: self.speaker,
        }))
    }
}

impl SpeakersParams {
    fn into_request(self) -> BrokeredCaptureRequest {
        BrokeredCaptureRequest::Speakers(BrokerSpeakersRequest {
            name: self.name,
            limit: self.limit,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ResultIdParams {
    /// Opaque result id returned by a previous search.
    opaque_result_id: String,
}

#[derive(Clone)]
struct MnemaMcp {
    identity: BrokerClientIdentity,
    /// `--no-prompt`, carried through to every tool call. The flag is the ONE
    /// control for "do not bother anyone" now that the TTY gate is gone, so the
    /// MCP door has to honour it too — a server started with it opens no approval
    /// window and fails `authorization_required` instead. Authorization is
    /// unaffected either way: every tool routes through `execute_data_request`.
    allow_prompt: bool,
}

#[tool_router]
impl MnemaMcp {
    #[tool(
        description = "Search the user's captured screen text and audio transcripts. Returns snippets with opaque result ids; use show_text for the full text behind a result and open to reveal it in the Mnema app. Pass `speaker` (a handle from the speakers tool) to get only what one person said, their words included as `turns` — absent on a matched result when the voice was heard but no words could be attributed to it (overlapped speech), never a claim they were silent. An `id` names the recording, not the match, so one `id` can appear twice in a page with different `spanStartMs`; dedupe on (`id`, `spanStartMs`), never on `id` alone. `spanStartMs`/`spanEndMs` and `turns` timestamps are media-relative while `startedAt`/`endedAt` are wall-clock, so they can disagree by a few hundred ms — not an error. A speaker-filtered response also carries `speakerCoverage`: `recordingsWithUnnamedVoices` (recordings holding a voice nobody has named — any could be this person, and labeling that voice in Mnema brings the recording into reach) and `recordingsWithoutSpeakerData` (recordings where speaker detection found nothing at all, which no speaker filter can ever reach). Either count above zero makes the answer PARTIAL: say what you could attribute, and never report an empty or short filtered result as proof the person said nothing. `scopeClamped: true` means the user's access permission does not reach as far back as `from` asked and these results cover a SHORTER window than requested — `requiredScope` names the access that would cover it; say so rather than reporting the window as empty."
    )]
    async fn search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run("search", p.into_request()?).await
    }

    /// The MCP door serves `speakerCoverage` too, and a chat client never reads
    /// `SKILL.md` — these descriptions are its whole contract. See `search`.
    #[tool(
        description = "List the user's capture activity intervals between two RFC3339 timestamps. Pass `speaker` (a handle from the speakers tool) for when one person was talking; matching intervals carry that speaker's words as `turns`, absent when the voice was heard but no words could be attributed to it — never a claim they were silent. That response also carries `speakerCoverage` (`recordingsWithUnnamedVoices` + `recordingsWithoutSpeakerData`) counting audio the filter could not check, so either count above zero makes the answer PARTIAL rather than proof the person was silent. `scopeClamped: true` means the user's access permission does not reach as far back as `from` asked and these intervals cover a SHORTER window than requested — `requiredScope` names the access that would cover it; say so rather than reporting the window as empty."
    )]
    async fn timeline(
        &self,
        Parameters(p): Parameters<TimelineParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run("timeline", p.into_request()?).await
    }

    #[tool(
        description = "List who was heard in the user's audio, longest-speaking first, each with the opaque handle that search and timeline take as `speaker`. A `person` handle is one human, stable across recordings; a `voice` handle is one voice inside ONE capture session, not a person — a session is a continuous sitting and recordings are capped at 5 minutes, so filtering on one `voice` handle returns every consecutive recording in that sitting, and the same human gets an unrelated handle in the next one. Never store a `voice` handle or treat two of them as the same human. `truncated` means this is not everyone; narrow with `name`."
    )]
    async fn speakers(
        &self,
        Parameters(p): Parameters<SpeakersParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run("speakers", p.into_request()).await
    }

    #[tool(
        description = "Fetch the full captured text behind a search result id. Audio results also list the speakers heard, each named only when the user assigned that voice to a person or it was recognized (with a confidence), plus `turns` saying who said which words. Missing `turns` means the words could NOT be attributed — never that nobody spoke; the text is still there."
    )]
    async fn show_text(
        &self,
        Parameters(p): Parameters<ResultIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "show-text",
            BrokeredCaptureRequest::ShowText {
                opaque_id: p.opaque_result_id,
            },
        )
        .await
    }

    #[tool(description = "Open a result in the Mnema app so the user can view it.")]
    async fn open(
        &self,
        Parameters(p): Parameters<ResultIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(
            "open",
            BrokeredCaptureRequest::OpenInMnema {
                opaque_id: p.opaque_result_id,
            },
        )
        .await
    }

    async fn run(
        &self,
        command: &str,
        request: BrokeredCaptureRequest,
    ) -> Result<CallToolResult, ErrorData> {
        match execute_data_request(command, &self.identity, request, self.allow_prompt).await {
            Ok(value) => {
                let text = serde_json::to_string(&value)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            // Broker/auth failures are tool-level errors so the model can
            // relay them to the user, not protocol failures.
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "{} ({})",
                error.message, error.code
            ))])),
        }
    }
}

#[tool_handler]
impl ServerHandler for MnemaMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        // Told the truth about prompting: a server started with `--no-prompt`
        // never opens the approval window, so promising a pause that will not
        // happen leaves the model waiting on consent that is never asked for.
        info.instructions = Some(
            if self.allow_prompt {
                "Brokered read access to the user's Mnema screen and audio capture history. \
                 The first call may pause while the user approves access in the Mnema app."
            } else {
                // Settings -> Data -> Access can only block and unblock tools Mnema
                // already has a row for; it cannot grant a first permission, and a
                // client it has never seen has no row there at all. Naming it as
                // the fix leaves the model waiting on a grant that cannot arrive,
                // and pushes the operator to drop the one flag that guarantees no
                // consent window.
                "Brokered read access to the user's Mnema screen and audio capture history. \
                 This server was started with --no-prompt: calls without an existing access \
                 permission fail instead of asking the user. Settings -> Data -> Access only \
                 blocks and unblocks tools Mnema has already seen, so a first permission is \
                 granted by running `mnema access request` in a terminal as this tool (same \
                 --client name or agent environment). Report that and stop; retrying without \
                 it cannot succeed."
            }
            .to_string(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("mnema", env!("CARGO_PKG_VERSION"));
        info
    }
}

/// The one seam a test can reach the MCP door through. `run` owns the
/// `allow_prompt` decision for all five tools, and nothing observable from
/// outside this module records which way it went.
#[cfg(test)]
pub(crate) async fn call_tool_for_tests(
    identity: &BrokerClientIdentity,
    allow_prompt: bool,
    command: &str,
    request: BrokeredCaptureRequest,
) -> CallToolResult {
    MnemaMcp {
        identity: identity.clone(),
        allow_prompt,
    }
    .run(command, request)
    .await
    .expect("the MCP door maps broker failures to tool errors, never protocol errors")
}

pub(crate) async fn serve(
    identity: BrokerClientIdentity,
    allow_prompt: bool,
) -> Result<(), CliError> {
    let service = MnemaMcp {
        identity,
        allow_prompt,
    }
    .serve(stdio())
    .await
    .map_err(broker_error)?;
    service.waiting().await.map_err(broker_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `url` and `url_regex` are documented as mutually exclusive — SKILL.md step
    /// 8, the params' own doc comments, and `conflicts_with = "url"` on the CLI
    /// door. The MCP door has to refuse the pair too: the broker ANDs both
    /// predicates, so forwarding them silently answers a narrower query than any
    /// client asked for.
    #[test]
    fn mcp_rejects_both_url_filters_in_one_call() {
        let search: SearchParams = serde_json::from_value(serde_json::json!({
            "query": "review",
            "url": "github.com",
            "url_regex": "^github\\."
        }))
        .expect("search params should deserialize");
        assert_eq!(
            search
                .into_request()
                .expect_err("both url filters in one search call must be rejected")
                .code,
            rmcp::model::ErrorCode::INVALID_PARAMS
        );

        let timeline: TimelineParams = serde_json::from_value(serde_json::json!({
            "from": "2026-05-22T10:00:00Z",
            "to": "2026-05-22T11:00:00Z",
            "url": "github.com",
            "url_regex": "^github\\."
        }))
        .expect("timeline params should deserialize");
        assert_eq!(
            timeline
                .into_request()
                .expect_err("both url filters in one timeline call must be rejected")
                .code,
            rmcp::model::ErrorCode::INVALID_PARAMS
        );
    }

    #[test]
    fn mcp_forwards_a_single_url_filter() {
        let search: SearchParams = serde_json::from_value(serde_json::json!({
            "query": "review",
            "url_regex": "(?i)^github\\.com/"
        }))
        .expect("search params should deserialize");
        let BrokeredCaptureRequest::Search(request) =
            search.into_request().expect("one filter is allowed")
        else {
            panic!("search params must build a search request");
        };
        assert_eq!(request.url, None);
        assert_eq!(request.url_regex.as_deref(), Some("(?i)^github\\.com/"));
    }

    /// Both doors publish the same refusal — SKILL.md ("`--speaker` ... cannot be
    /// combined with any of them"), `crates/cli/CONTEXT.md` ("the CLI rejects the
    /// pair at the door rather than returning an empty page"), and these params'
    /// own schema descriptions ("Cannot be combined with app, window_title, url,
    /// or url_regex"). Clap enforces it on the CLI door; delegating it to the
    /// broker here makes the MCP door answer the same call differently — and,
    /// with no active grant yet, only *after* firing the user's approval dialog
    /// for a request that can never succeed.
    #[test]
    fn mcp_rejects_a_speaker_filter_beside_every_screen_filter() {
        for (field, value) in [
            ("app", "Zoom"),
            ("window_title", "Standup"),
            ("url", "zoom.us"),
            ("url_regex", "^zoom\\."),
        ] {
            let search: SearchParams = serde_json::from_value(serde_json::json!({
                "query": "standup",
                "speaker": "p1.deadbeef",
                field: value,
            }))
            .expect("search params should deserialize");
            assert_eq!(
                search
                    .into_request()
                    .expect_err(&format!("search speaker + {field} must be rejected"))
                    .code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "search speaker + {field}"
            );

            let timeline: TimelineParams = serde_json::from_value(serde_json::json!({
                "from": "2026-05-22T10:00:00Z",
                "to": "2026-05-22T11:00:00Z",
                "speaker": "p1.deadbeef",
                field: value,
            }))
            .expect("timeline params should deserialize");
            assert_eq!(
                timeline
                    .into_request()
                    .expect_err(&format!("timeline speaker + {field} must be rejected"))
                    .code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "timeline speaker + {field}"
            );
        }
    }

    /// Settings -> Data -> Access can only block and unblock clients Mnema
    /// already holds a row for — a client it has never seen has no row there at
    /// all — so it cannot grant a FIRST permission. Naming it as the fix under
    /// `--no-prompt` (which is precisely the mode with no row and no window)
    /// leaves the model waiting on a grant that can never arrive, and pushes the
    /// operator to drop the one flag that guarantees no consent window.
    /// `mnema access request` is the only door that creates the row.
    #[test]
    fn the_no_prompt_server_names_the_command_that_can_actually_grant_access() {
        let identity = BrokerClientIdentity::new(
            "Claude Desktop",
            app_infra::brokered_access::BrokerClientIdentitySource::Explicit,
        )
        .expect("identity");

        let no_prompt = MnemaMcp {
            identity: identity.clone(),
            allow_prompt: false,
        }
        .get_info()
        .instructions
        .expect("the server states its terms");
        assert!(
            no_prompt.contains("mnema access request"),
            "the only door that can create a first permission has to be the one named: \
             {no_prompt}"
        );
        assert!(
            !no_prompt.contains("must grant access in the Mnema app"),
            "Settings cannot grant anything, so it must not be left standing as the \
             instruction: {no_prompt}"
        );

        let prompting = MnemaMcp {
            identity,
            allow_prompt: true,
        }
        .get_info()
        .instructions
        .expect("the server states its terms");
        assert!(
            prompting.contains("pause while the user approves access in the Mnema app"),
            "a prompting server still gets its permission from the approval window: {prompting}"
        );
        assert!(
            !prompting.contains("mnema access request"),
            "sending a model to a terminal it does not have, for a window that will open \
             on its own, is the same dead end in reverse: {prompting}"
        );
    }

    #[test]
    fn mcp_router_exposes_exactly_the_five_data_tools() {
        let mut names: Vec<String> = MnemaMcp::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["open", "search", "show_text", "speakers", "timeline"]
        );
    }

    /// A speaker filter can only match through DETECTED speaker turns, so an empty
    /// filtered result is routinely "this audio could not be checked", not "they
    /// said nothing" — `speakerCoverage` is the only thing that tells the two
    /// apart, and `map_search_data`/`map_timeline_data` now hand it to this door
    /// too. An MCP client never reads `.agents/skills/mnema-data/SKILL.md` (that is
    /// the CLI skill's contract); these tool descriptions are the whole contract it
    /// gets. The Ask AI door asserts exactly this
    /// (`ask_ai.rs::speaker_filtered_tools_tell_the_model_an_empty_result_is_not_silence`).
    #[test]
    fn mcp_speaker_filterable_tools_tell_the_client_an_empty_result_is_not_silence() {
        let tools = MnemaMcp::tool_router().list_all();
        for name in ["search", "timeline"] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("`{name}` must be offered"));
            let description = tool.description.clone().unwrap_or_default();
            assert!(
                description.contains("speakerCoverage"),
                "`{name}` hands the client `speakerCoverage` but never names it: {description}"
            );
            assert!(
                description.to_lowercase().contains("partial"),
                "`{name}` must say a non-zero coverage count makes the answer partial: {description}"
            );
        }
    }

    /// Both tools can now return a page the user's permission cut short. A chat
    /// client reads only these descriptions, so a `scopeClamped` field it was
    /// never told about is exactly the silent under-report ADR 0059 exists to end.
    #[test]
    fn mcp_range_filterable_tools_name_the_clamp_marker() {
        let tools = MnemaMcp::tool_router().list_all();
        for name in ["search", "timeline"] {
            let description = tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("`{name}` must be offered"))
                .description
                .clone()
                .unwrap_or_default();
            assert!(
                description.contains("scopeClamped") && description.contains("requiredScope"),
                "`{name}` can return a clamped page but never names the marker: {description}"
            );
        }
    }

    /// The handle is the whole point of discovery: without `speaker` on these two
    /// params the MCP door can list people it has no way to filter by.
    #[test]
    fn mcp_forwards_the_speaker_handle() {
        let search: SearchParams = serde_json::from_value(serde_json::json!({
            "query": "standup",
            "speaker": "p1.deadbeef"
        }))
        .expect("search params should deserialize");
        let BrokeredCaptureRequest::Search(request) =
            search.into_request().expect("speaker alone is allowed")
        else {
            panic!("search params must build a search request");
        };
        assert_eq!(request.speaker.as_deref(), Some("p1.deadbeef"));

        let timeline: TimelineParams = serde_json::from_value(serde_json::json!({
            "from": "2026-05-22T10:00:00Z",
            "to": "2026-05-22T11:00:00Z",
            "speaker": "p1.deadbeef"
        }))
        .expect("timeline params should deserialize");
        let BrokeredCaptureRequest::Timeline(request) =
            timeline.into_request().expect("speaker alone is allowed")
        else {
            panic!("timeline params must build a timeline request");
        };
        assert_eq!(request.speaker.as_deref(), Some("p1.deadbeef"));

        let speakers: SpeakersParams =
            serde_json::from_value(serde_json::json!({ "name": "priya", "limit": 5 }))
                .expect("speakers params should deserialize");
        let BrokeredCaptureRequest::Speakers(request) = speakers.into_request() else {
            panic!("speakers params must build a speakers request");
        };
        assert_eq!(request.name.as_deref(), Some("priya"));
        assert_eq!(request.limit, Some(5));
    }
}
