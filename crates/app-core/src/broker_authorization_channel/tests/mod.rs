use super::*;
use app_infra::brokered_access::{BrokerClientIdentitySource, BrokerGrant, BrokerGrantFile};

mod approval;
mod host;

/// The Tauri runtime's `block_on`, for the tests that drive a socket pair.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Runtime::new()
        .expect("test runtime starts")
        .block_on(future)
}

fn test_authorization_request(minimum_scope: &str) -> AuthorizationChannelRequest {
    AuthorizationChannelRequest {
        schema_version: 1,
        request_id: "request-1".to_string(),
        client: AuthorizationChannelClient {
            label: "Test Client".to_string(),
            source: "explicit".to_string(),
        },
        command: "search".to_string(),
        scope: AuthorizationChannelScope {
            minimum: minimum_scope.to_string(),
            preferred: minimum_scope.to_string(),
        },
        created_at: "2026-05-23T00:00:00Z".to_string(),
    }
}

fn request_preferring(preferred_scope: &str) -> AuthorizationChannelRequest {
    let mut request = test_authorization_request("lastDay");
    request.scope.preferred = preferred_scope.to_string();
    request
}

/// The request the CLI serializes is what this side deserializes, and the two
/// structs are hand-mirrored. `interactive` was carried by both and read by
/// neither once the TTY gate went; dropping a field from one side alone would
/// wedge every approval, so this pins BOTH halves of the reason it is safe:
/// the current shape parses, and a field this build does not know is ignored
/// rather than refused (no `deny_unknown_fields` on the wire structs).
#[test]
fn a_request_parses_without_interactive_and_tolerates_an_unknown_field() {
    let current = serde_json::json!({
        "schemaVersion": 1,
        "requestId": "request-1",
        "client": { "label": "Claude Code", "source": "explicit" },
        "command": "search",
        "scope": { "minimum": "lastDay", "preferred": "allRetained" },
        "createdAt": "2026-05-23T00:00:00Z",
    });
    let parsed: AuthorizationChannelRequest =
        serde_json::from_value(current.clone()).expect("the shape the CLI sends must parse");
    assert_eq!(parsed.scope.preferred, "allRetained");

    let mut with_extra = current;
    with_extra["interactive"] = serde_json::Value::Bool(true);
    with_extra["somethingNew"] = serde_json::Value::Bool(true);
    serde_json::from_value::<AuthorizationChannelRequest>(with_extra)
        .expect("an unknown field is ignored, not a parse failure");
}

/// What the window is told the client ALREADY has. An approval sets the row's
/// scope, so a selection below this would take access away — the window says so
/// instead of letting Allow do it quietly. A blocked row is not access.
#[test]
fn the_current_scope_is_the_unblocked_row_this_client_already_holds() {
    let mut grants = BrokerGrantFile {
        schema_version: 1,
        grants: vec![blocked_grant("Claude Code")],
    };
    grants.grants[0].scope = BrokerGrantScope::AllRetainedHistory;

    assert_eq!(
        current_scope_in(&grants, "Claude Code"),
        None,
        "a blocked row grants nothing, so there is nothing to lose"
    );
    grants.grants[0].blocked = false;
    // Keyed on the same normalization the row is stored under, so the notice
    // cannot be dodged by a spelling variant of the tool's own name.
    assert_eq!(
        current_scope_in(&grants, "claude-code").as_deref(),
        Some("allRetained")
    );
    assert_eq!(current_scope_in(&grants, "Some Other Tool"), None);
}

/// `AuthorizationChannelScope.minimum` is wire input from whatever local
/// process opened the socket, and the approval window DISABLES every option
/// below it. So a first-contact tool that declares `allRetained` makes the
/// whole retained history the only answer the sheet can give — under Mnema's
/// own copy asserting the tool requires it. ADR 0059 promises that cannot
/// happen, and the CLI's copy of the rule is not in this path: anything that
/// can open the socket writes this field itself.
#[test]
fn a_client_mnema_has_never_granted_cannot_state_its_own_floor() {
    for claimed in ["allRetained", "last7Days", "lastDay"] {
        assert_eq!(
            enforced_minimum_scope(claimed, None),
            "lastDay",
            "a first prompt offers the narrowest band whatever the tool claims: {claimed}"
        );
    }

    // ...and the floor it lands on is one the window can actually answer: the
    // narrowest selection now validates, where the claimed `allRetained` would
    // have refused every option below itself.
    let mut request = test_authorization_request("allRetained");
    request.scope.minimum = enforced_minimum_scope(&request.scope.minimum, None);
    assert!(
        validate_cli_access_approval(
            &request,
            &ApproveCliAccessRequest {
                request_id: "request-1".to_string(),
                scope: "lastDay".to_string(),
            },
        )
        .is_ok(),
        "one day must be grantable to a tool this machine has never seen"
    );
}

/// The two tests around this one pin the RULE; this one pins that the rule is
/// actually on the path to the window.
///
/// Both of them exercise `enforced_minimum_scope` directly, so deleting the
/// one line that calls it leaves them green while a first-contact client goes
/// straight back to choosing its own floor — the whole defect. The clamp
/// therefore lives inside `await_user_verdict`, the single function every
/// request reaches the consent surface through, and this reads the request the
/// window is actually handed: the consent surface is opened only AFTER
/// `store_pending`, so the injected opener sees exactly what the page would.
#[tokio::test]
async fn the_floor_is_clamped_before_the_window_is_ever_shown_the_request() {
    let displayed = |current_scope: Option<&'static str>| async move {
        let state = BrokerAuthorizationChannelState::default();
        let seen = Arc::new(Mutex::new(None));
        let probe = seen.clone();
        // A client claiming it needs everything, on a machine that has never
        // approved it.
        let request = test_authorization_request("allRetained");

        let verdict = await_user_verdict(&state, &request, current_scope, || {
            *probe.lock().expect("probe") =
                take_request_for_display(&state).map(|shown| shown.scope.minimum);
            // Refuse the surface so the waiter resolves instead of parking.
            Err("no window in a unit test".to_string())
        })
        .await;

        assert_eq!(verdict.decision, "denied");
        let seen = seen.lock().expect("probe").clone();
        seen.expect("the window is handed the request before it opens")
    };

    assert_eq!(
        displayed(None).await,
        "lastDay",
        "a tool with no standing permission does not get to set the floor the \
             window greys every narrower option out below"
    );
    assert_eq!(
        displayed(Some("lastDay")).await,
        "allRetained",
        "but a widen keeps the floor its call needs — the row already stands \
             narrower, so a narrower answer both fails the call and takes access away"
    );
}

/// The other door. Once a client HOLDS a row, the floor it states is the scope
/// its call needs and it has to survive: the row already stands narrower, and
/// an approval SETS the scope — so clamping a widen back to the narrowest band
/// would offer the user an Allow that both fails the call and takes away access
/// the tool already had.
#[test]
fn a_widen_keeps_the_floor_the_pending_call_actually_needs() {
    assert_eq!(
        enforced_minimum_scope("allRetained", Some("lastDay")),
        "allRetained",
        "the row stands at one day; anything narrower cannot serve this call"
    );
    assert_eq!(
        enforced_minimum_scope("last7Days", Some("lastDay")),
        "last7Days"
    );

    // The narrower selection is exactly the one that must NOT validate here.
    let mut request = test_authorization_request("allRetained");
    request.scope.minimum = enforced_minimum_scope(&request.scope.minimum, Some("lastDay"));
    assert!(
        validate_cli_access_approval(
            &request,
            &ApproveCliAccessRequest {
                request_id: "request-1".to_string(),
                scope: "lastDay".to_string(),
            },
        )
        .is_err(),
        "re-granting the scope the row already carries is not a widen"
    );
}

#[test]
fn socket_path_uses_configured_identifier() {
    let path = socket_path_for_identifier("com.example.mnema-test");
    assert!(path.ends_with("com.example.mnema-test/cli-access.sock"));
}

/// Every scope pair the wire can send, routed through `BrokerGrantScope` so
/// the window, the CLI and the permission row cannot disagree about which
/// spelling outranks which. `last7Days` is the band a string compare missed.
#[test]
fn scope_ranking_matches_the_permission_scopes() {
    assert!(scope_satisfies_minimum("allRetained", "lastDay"));
    assert!(scope_satisfies_minimum("allRetained", "last7Days"));
    assert!(scope_satisfies_minimum("last7Days", "lastDay"));
    assert!(scope_satisfies_minimum("lastDay", "lastDay"));
    assert!(!scope_satisfies_minimum("lastDay", "allRetained"));
    assert!(!scope_satisfies_minimum("lastDay", "last7Days"));
    assert!(!scope_satisfies_minimum("last7Days", "allRetained"));
    // An unrecognized spelling is never a coincidental match.
    assert!(!scope_satisfies_minimum("forever", "forever"));
}

fn blocked_grant(label: &str) -> BrokerGrant {
    BrokerGrant {
        id: "grant-1".to_string(),
        label: label.to_string(),
        normalized_label: app_infra::brokered_access::normalize_client_label(label)
            .expect("test label normalizes"),
        identity_source: BrokerClientIdentitySource::Explicit,
        created_at_unix_ms: 1,
        last_used_at_unix_ms: 1,
        scope: BrokerGrantScope::LAST_DAY,
        blocked: true,
        blocked_at_unix_ms: Some(2),
    }
}

/// A blocked client is a STANDING rejection: the connection is answered from
/// the permission file alone, so no approval window ever opens and the user
/// is never asked again. This is the check `handle_connection` runs before it
/// even takes the single-flight guard.
#[test]
fn a_blocked_client_is_refused_from_the_permission_file_alone() {
    let grants = BrokerGrantFile {
        schema_version: 1,
        grants: vec![blocked_grant("Claude Code")],
    };

    // Matched by the same normalization the permission row is keyed on, so
    // case and separator variants of the name stay blocked.
    assert!(grant_file_blocks(&grants, "Claude Code"));
    assert!(grant_file_blocks(&grants, "claude-code"));
    assert!(!grant_file_blocks(&grants, "Some Other Tool"));

    let mut unblocked = grants.clone();
    unblocked.grants[0].blocked = false;
    assert!(!grant_file_blocks(&unblocked, "Claude Code"));
}

/// A standing block is keyed on the row's `normalized_label`; the approval
/// window and the Settings list both render the name through the display
/// collapse. If the two ever disagree again, a name that READS as the blocked
/// tool keys a different identity and the user's rejection is lifted by a
/// character they cannot see. The existing case/hyphen test cannot see this:
/// both spellings already agreed.
#[test]
fn a_name_that_reads_as_a_blocked_one_stays_blocked() {
    let grants = BrokerGrantFile {
        schema_version: 1,
        grants: vec![blocked_grant("Claude Code")],
    };

    for raw in [
        "Claude\u{200B} Code", // ZERO WIDTH SPACE
        "Claude Code\u{200D}", // ZERO WIDTH JOINER
        "\u{202E}Claude Code", // RIGHT-TO-LEFT OVERRIDE
        "Claude\u{FE00} Code", // VARIATION SELECTOR-1
        "Claude\u{3164} Code", // HANGUL FILLER
        "Claude\tCode",        // control, folded to the space it renders as
        "Claude\nCode",
    ] {
        assert_eq!(
            client_label_display(raw),
            "Claude Code",
            "the window names this requester as the blocked tool: {raw:?}"
        );
        assert!(
            grant_file_blocks(&grants, raw),
            "a name the user cannot tell apart from the blocked one stays \
                 blocked: {raw:?}"
        );
    }

    // The other direction: a row minted under a disguised spelling still
    // covers the plain name, so blocking what Settings SHOWS you holds.
    let smuggled = BrokerGrantFile {
        schema_version: 1,
        grants: vec![blocked_grant("Claude\tCode")],
    };
    assert!(grant_file_blocks(&smuggled, "Claude Code"));
}

/// A pending approval must not outlive the client that asked for it. The
/// socket task parks on the oneshot and never polls the stream, so a CLI that
/// hit its own 120 s timeout, was killed, or whose window was silently reused
/// and torn down by a previous request's teardown would otherwise leave the
/// single-flight guard held and the slot occupied for the life of the app —
/// answering every later request `busy` with no window on screen and nothing
/// able to resolve the waiter.
#[test]
fn a_client_that_hangs_up_frees_the_channel_instead_of_wedging_it() {
    block_on(async {
        let state = BrokerAuthorizationChannelState::default();
        let request = test_authorization_request("lastDay");
        let guard = ActiveRequestGuard::acquire(state.active.clone())
            .expect("the first connection owns the channel");
        let (client, mut server) = UnixStream::pair().expect("socket pair should open");

        // The consent surface opens fine and is simply never answered — the
        // user walked away, or it was a window that no longer shows this
        // request. Meanwhile the client gives up.
        let verdict = {
            let wait = await_user_verdict(&state, &request, None, || Ok(()));
            tokio::pin!(wait);
            drop(client);
            tokio::time::timeout(Duration::from_secs(2), async {
                tokio::select! {
                    response = &mut wait => Some(response),
                    () = peer_hung_up(&mut server) => None,
                }
            })
            .await
            .expect("a hung-up client must be noticed, not waited on forever")
        };

        assert!(
            verdict.is_none(),
            "there is nobody left to write an answer to"
        );
        cancel_pending(&state, "closed");
        drop(guard);

        assert!(
            state
                .pending
                .lock()
                .expect("the pending slot is readable")
                .is_none(),
            "the slot must not outlive the client that filled it"
        );
        assert!(
            ActiveRequestGuard::acquire(state.active.clone()).is_some(),
            "the next client must get a window, not `busy` forever"
        );
    });
}

#[test]
fn a_blocked_response_names_the_block_on_both_fields() {
    block_on(async {
        let (client, server) = UnixStream::pair().expect("socket pair should open");
        write_blocked(server, "request-1".to_string())
            .await
            .expect("blocked response should write");

        let response = read_response(client).await;
        assert_eq!(response.decision, "blocked");
        assert_eq!(response.reason.as_deref(), Some("blocked"));
        assert!(
            response.grant.is_none(),
            "a blocked client is granted nothing"
        );
    });
}

async fn read_response(mut stream: UnixStream) -> AuthorizationChannelResponse {
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .expect("response should read");
    serde_json::from_slice(&raw).expect("response should parse")
}
