use super::*;
use tokio::sync::oneshot::error::TryRecvError;

/// Widening a permission MUST keep the row's id: opaque result ids are
/// HMAC-signed against the issuing grant id, so a new id would invalidate
/// every id already handed to a running agent, mid-task. The response also
/// has to report the scope the ROW now carries.
#[test]
fn approving_an_existing_client_reuses_the_row_and_keeps_its_id() {
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let request = request_preferring("lastDay");

    let first = create_grant_response_in(config_dir.path(), &request, BrokerGrantScope::LAST_DAY)
        .expect("first approval should mint a permission");
    let first_grant = first.grant.expect("an approval carries its grant");
    assert!(first_grant.created, "the first approval creates the row");
    assert_eq!(first_grant.scope, "lastDay");

    let second = create_grant_response_in(
        config_dir.path(),
        &request,
        BrokerGrantScope::AllRetainedHistory,
    )
    .expect("widening should succeed");
    let second_grant = second.grant.expect("an approval carries its grant");

    assert_eq!(
        second_grant.id, first_grant.id,
        "a widen must reuse the row, so ids already issued keep resolving"
    );
    assert!(
        !second_grant.created,
        "the second approval widened an existing row"
    );
    assert_eq!(second_grant.scope, "allRetained");
    assert_eq!(second.decision, "approved");

    let stored = BrokeredCaptureAccess::from_config_dir(config_dir.path())
        .list_grants()
        .expect("permissions should load");
    assert_eq!(stored.grants.len(), 1, "one row per client, never a second");
}

/// An approval SETS the row's scope, so it can hand back less than the client
/// asked for — the user picks in the window, and the picker only enforces the
/// request's minimum. That is exactly why the response has to state the scope
/// and why the caller has to verify it covers the request rather than reading
/// `approved` as a yes.
#[test]
fn an_approval_reports_the_scope_the_row_actually_carries() {
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let request = request_preferring("allRetained");

    create_grant_response_in(
        config_dir.path(),
        &request,
        BrokerGrantScope::AllRetainedHistory,
    )
    .expect("first approval should mint a permission");
    let narrowed =
        create_grant_response_in(config_dir.path(), &request, BrokerGrantScope::LAST_DAY)
            .expect("a narrower approval still succeeds")
            .grant
            .expect("an approval carries its grant");

    assert_eq!(
        narrowed.scope, "lastDay",
        "the response states what the row carries, not what was asked for"
    );
    let stored = BrokeredCaptureAccess::from_config_dir(config_dir.path())
        .list_grants()
        .expect("permissions should load");
    assert_eq!(stored.grants.len(), 1);
    assert_eq!(stored.grants[0].scope, BrokerGrantScope::LAST_DAY);
    assert_eq!(stored.grants[0].id, narrowed.id, "still the same row");
}

/// The standing-block check runs when the SOCKET CONNECTS; the approval it
/// waves through arrives whenever the user acts on the window — seconds or
/// minutes later. A tool blocked in Settings during that gap must be refused
/// by the approval that stale window then sends, and the client must be told
/// `blocked` rather than handed an `approved` receipt for a permission the
/// file does not carry.
#[test]
fn an_approval_that_lands_after_a_block_is_refused_not_granted() {
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let request = request_preferring("lastDay");
    create_grant_response_in(config_dir.path(), &request, BrokerGrantScope::LAST_DAY)
        .expect("the first approval mints the permission");

    let access = BrokeredCaptureAccess::from_config_dir(config_dir.path());
    assert!(
        access.block_client("Test Client").expect("block applies"),
        "Settings blocks the tool while its window is still open"
    );

    let response = create_grant_response_in(
        config_dir.path(),
        &request,
        BrokerGrantScope::AllRetainedHistory,
    )
    .expect("the stale window's approval still answers");

    assert_eq!(response.decision, "blocked");
    assert_eq!(response.reason.as_deref(), Some("blocked"));
    assert!(response.grant.is_none(), "nothing was granted");

    let stored = access.list_grants().expect("permissions load");
    assert_eq!(stored.grants.len(), 1);
    assert!(
        stored.grants[0].blocked,
        "the newer decision survives: {stored:?}"
    );
    assert_eq!(
        stored.grants[0].scope,
        BrokerGrantScope::LAST_DAY,
        "a refused approval never widens the row it was refused on: {stored:?}"
    );
}

/// A name that renders as nothing is not a name — whether it is blank or
/// merely invisible. Whitespace collapses to empty through
/// `split_whitespace()`; the invisible characters need the filter, because
/// `is_control()` does not cover them, so an all-zero-width label survives the
/// empty check and the approval window renders with NO visible requester at
/// all — defeating the fallback that exists to guarantee one.
#[test]
fn a_client_name_that_renders_as_nothing_falls_back_to_the_unnamed_wording() {
    for label in [
        "",
        "   ",
        "\t\n",
        "\u{200B}\u{200C}\u{200D}\u{FEFF}\u{2060}",
        "\u{3164}",                 // HANGUL FILLER
        "\u{2800}",                 // BRAILLE PATTERN BLANK
        "\u{FFF9}\u{FFFA}\u{FFFB}", // interlinear annotation controls
        "\u{1D173}\u{1D17A}",       // musical symbol format controls
        "\u{E0100}",                // variation selector supplement
    ] {
        assert_eq!(
            client_label_display(label),
            "An unnamed local tool",
            "a name that renders as nothing must fall back: {label:?}"
        );
    }
}

/// Bidi overrides and isolates reorder every character after them when the
/// window renders, so a client name must not be able to smuggle one into the
/// requester chip and restructure the consent copy around it.
#[test]
fn bidi_reordering_never_reaches_the_requester_chip() {
    let display = client_label_display("Mnema CLI\u{202E}\u{2066}\u{2067}");

    assert!(
        !display.contains(['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}']),
        "no bidi embedding/override may reach the window: {display:?}"
    );
    assert!(
        !display.contains(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}']),
        "no bidi isolate may reach the window: {display:?}"
    );
    assert_eq!(display, "Mnema CLI");
}

/// The truncation boundary itself: a label of exactly
/// [`CLIENT_LABEL_MAX_CHARS`] chars survives verbatim, and one char more is
/// cut to exactly that many chars plus a single '…'. A label long enough to
/// push the consent controls off the sheet is not a name.
#[test]
fn a_client_label_is_capped_before_it_reaches_the_window() {
    let at_cap = "A".repeat(CLIENT_LABEL_MAX_CHARS);
    assert_eq!(client_label_display(&at_cap), at_cap);

    let over_cap = "A".repeat(CLIENT_LABEL_MAX_CHARS + 1);
    let display = client_label_display(&over_cap);
    assert_eq!(
        display,
        format!("{}…", "A".repeat(CLIENT_LABEL_MAX_CHARS)),
        "one char over the cap truncates to the cap plus a single ellipsis"
    );
    assert_eq!(display.chars().count(), CLIENT_LABEL_MAX_CHARS + 1);

    // Newlines would let a name fabricate the window's own copy.
    assert_eq!(
        client_label_display("Mnema Helper\n\nAllow grants everything"),
        "Mnema Helper Allow grants everything"
    );
}

#[test]
fn request_line_reader_rejects_oversized_requests() {
    block_on(async {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair should open");
        let request = vec![b'a'; REQUEST_MAX_BYTES + 1];
        let writer = tokio::spawn(async move { client.write_all(&request).await });

        let result = read_request_line(&mut server).await;

        assert!(result.is_err());
        writer
            .await
            .expect("writer task should finish")
            .expect("oversized request should write");
    });
}

#[test]
fn invalid_approval_scope_preserves_pending_request_and_waiter() {
    let (respond, mut receive) = oneshot::channel();
    let mut pending = Some(PendingAuthorizationRequest {
        // The window showed this one; that is what makes a Deny an answer.
        rendered: true,
        request: test_authorization_request("allRetained"),
        respond,
    });

    let result = take_validated_pending_request(
        &mut pending,
        &ApproveCliAccessRequest {
            request_id: "request-1".to_string(),
            scope: "lastDay".to_string(),
        },
    );

    assert!(result.is_err());
    assert!(pending.is_some());
    assert!(matches!(receive.try_recv(), Err(TryRecvError::Empty)));
}

/// The pending slot is single-flight but not single-tenant over time: the
/// moment a hung-up client frees it, the NEXT request can fill it while the
/// reused window still renders the previous one (it only re-reads on focus,
/// which never fires on an already-frontmost window). So the approval must
/// bind to the request the window displayed — otherwise the user's click
/// grants a tool they were never shown.
#[test]
fn an_approval_for_a_request_other_than_the_displayed_one_is_refused() {
    let (respond, mut receive) = oneshot::channel();
    let mut swapped_in = test_authorization_request("lastDay");
    swapped_in.request_id = "request-2".to_string();
    swapped_in.client.label = "A Different Tool".to_string();
    let mut pending = Some(PendingAuthorizationRequest {
        // The window showed this one; that is what makes a Deny an answer.
        rendered: true,
        request: swapped_in,
        respond,
    });

    // The window rendered request-1; request-2 has since taken the slot.
    let result = take_validated_pending_request(
        &mut pending,
        &ApproveCliAccessRequest {
            request_id: "request-1".to_string(),
            scope: "allRetained".to_string(),
        },
    );

    assert!(result.is_err(), "a click can only approve what was shown");
    assert!(
        pending.is_some(),
        "the swapped-in request stays pending for its own window render"
    );
    assert!(matches!(receive.try_recv(), Err(TryRecvError::Empty)));

    // The same click against the request it actually rendered still works.
    let approved = take_validated_pending_request(
        &mut pending,
        &ApproveCliAccessRequest {
            request_id: "request-2".to_string(),
            scope: "allRetained".to_string(),
        },
    );
    assert!(approved.is_ok());
}

/// A permission that could not be WRITTEN is no verdict on the client's
/// request. Consuming the slot on a failed disk write spent the CLI's one
/// question on `unavailable`/`invalidRequest` — which it reports as "Mnema
/// rejected the access request as malformed", exit 18, `retryable: false` —
/// over a full disk or a read-only config dir, while the window told the user
/// to "Try again" on a slot that was already empty. The retry has to land on
/// the client still waiting, not on whatever took the slot meanwhile.
#[test]
fn a_grant_that_could_not_be_written_is_not_spent_on_the_waiting_client() {
    let state = BrokerAuthorizationChannelState::default();
    let (respond, mut receive) = oneshot::channel();
    assert!(store_pending(
        &state,
        test_authorization_request("lastDay"),
        respond
    ));
    let approval = ApproveCliAccessRequest {
        request_id: "request-1".to_string(),
        scope: "lastDay".to_string(),
    };

    let failed = approve_pending_request_in(&state, &approval, |_, _| {
        Err("No space left on device".to_string())
    });

    assert_eq!(
        failed.unwrap_err(),
        GRANT_WRITE_FAILED,
        "the window names the failed write instead of showing a granted receipt"
    );
    assert!(
        matches!(receive.try_recv(), Err(TryRecvError::Empty)),
        "a disk that would not take the permission is not an answer to the tool"
    );
    assert!(
        state
            .pending
            .lock()
            .expect("the pending slot is readable")
            .is_some(),
        "the 'Try again' the window offers has to have something left to retry"
    );

    // The retry writes the permission for real, and answers the SAME client.
    let config_dir = tempfile::tempdir().expect("temp config dir");
    approve_pending_request_in(&state, &approval, |request, scope| {
        create_grant_response_in(config_dir.path(), request, scope)
    })
    .expect("the second attempt grants");

    let response = receive
        .try_recv()
        .expect("the client that waited through the failure is the one answered");
    assert_eq!(response.request_id, "request-1");
    assert_eq!(response.decision, "approved");
    assert_eq!(
        response.grant.expect("an approval carries its grant").scope,
        "lastDay"
    );
}

/// Block answers only the request the window rendered, persists a standing
/// rejection for a never-granted tool, and tells the client `blocked`.
#[test]
fn blocking_the_rendered_request_persists_a_row_and_answers_blocked() {
    let state = BrokerAuthorizationChannelState::default();
    let (respond, mut receive) = oneshot::channel();
    assert!(store_pending(
        &state,
        test_authorization_request("lastDay"),
        respond
    ));
    let config_dir = tempfile::tempdir().expect("temp config dir");
    let block = |request: &AuthorizationChannelRequest| {
        BrokeredCaptureAccess::from_config_dir(config_dir.path())
            .block_identity(identity_for_request(request)?)
            .map(drop)
            .map_err(|error| error.to_string())
    };

    assert!(block_pending_request_in(&state, "request-2", block).is_err());
    assert!(matches!(receive.try_recv(), Err(TryRecvError::Empty)));

    block_pending_request_in(&state, "request-1", block).expect("block lands");
    let response = receive.try_recv().expect("the client is answered");
    assert_eq!(response.decision, "blocked");
    let stored = BrokeredCaptureAccess::from_config_dir(config_dir.path())
        .list_grants()
        .expect("permissions load");
    assert_eq!(stored.grants.len(), 1);
    assert!(stored.grants[0].blocked);
    assert_eq!(stored.grants[0].last_used_at_unix_ms, 0);
    assert!(grant_file_blocks(&stored, "Test Client"));
}

/// Deny and Esc may only answer the request the window actually DISPLAYED.
/// The slot is single-flight but not stable over time — an approval that fails
/// leaves the window up on its inline error, the socket task drops the guard,
/// and the next client refills the slot while the window (already frontmost,
/// so `set_focus` fires no `focus` event) keeps rendering the previous
/// request. Answering that unseen successor `userCancelled` spends a permanent
/// refusal — exit 10, `retryable: false` — on a tool nobody was asked about.
/// It still has to be answered or it waits on a window that is going away, and
/// `closed` is the verdict-free reason the CLI retries.
#[test]
fn dismissing_the_window_never_refuses_a_request_it_did_not_show() {
    let state = BrokerAuthorizationChannelState::default();
    let (respond, mut receive) = oneshot::channel();
    assert!(store_pending(
        &state,
        test_authorization_request("lastDay"),
        respond
    ));

    cancel_pending(&state, "userCancelled");

    let response = receive
        .try_recv()
        .expect("a request the window is going away on must still be answered");
    assert_eq!(response.decision, "denied");
    assert_eq!(
        response.reason.as_deref(),
        Some("closed"),
        "nobody refused this tool: it was never on screen to refuse"
    );
}

/// The other direction, and the reason being shown has to be RECORDED rather
/// than assumed: a refusal the user really gave is final, and the CLI must not
/// come back with the same ask. `take_request_for_display` is what
/// `get_pending_cli_access_request` calls, so having been read through it is
/// exactly "the sheet the user is looking at is about this request".
#[test]
fn denying_the_request_on_screen_is_still_a_final_refusal() {
    let state = BrokerAuthorizationChannelState::default();
    let (respond, mut receive) = oneshot::channel();
    assert!(store_pending(
        &state,
        test_authorization_request("lastDay"),
        respond
    ));

    let displayed = take_request_for_display(&state).expect("the window reads the slot");
    assert_eq!(displayed.request_id, "request-1");

    cancel_pending(&state, "userCancelled");

    let response = receive.try_recv().expect("Deny answers the waiting client");
    assert_eq!(response.decision, "denied");
    assert_eq!(
        response.reason.as_deref(),
        Some("userCancelled"),
        "a refusal the user actually gave must not come back as retryable"
    );
}

/// The window renders its granted receipt when — and only when — the approve
/// command resolves Ok, so Ok must mean the permission file really carries the
/// grant. A failed write answers the CLI `unavailable` and MUST come back
/// `Err`, or the user is shown "Access granted" for a permission that does
/// not exist.
#[test]
fn a_failed_grant_write_is_an_error_not_a_granted_receipt() {
    assert!(
        approval_command_outcome(&unavailable_response(
            "request-1".to_string(),
            "invalidRequest"
        ))
        .is_err(),
        "a grant that was never written must not resolve Ok"
    );
    assert!(approval_command_outcome(&blocked_response("request-1".to_string())).is_err());
    assert!(approval_command_outcome(&denied_response("request-1".to_string(), "closed")).is_err());
    assert!(approval_command_outcome(&AuthorizationChannelResponse {
        schema_version: 1,
        request_id: "request-1".to_string(),
        decision: "approved".to_string(),
        reason: None,
        grant: Some(AuthorizationChannelGrant {
            id: "grant-1".to_string(),
            client_label: "Test Client".to_string(),
            scope: "lastDay".to_string(),
            created: true,
        }),
    })
    .is_ok());
}

/// A request the approval flow could never turn into a grant is refused at
/// the socket: otherwise it opens a consent window whose Allow button always
/// errors, holding the single-flight guard while every other tool is told
/// `busy`.
#[test]
fn a_request_no_click_could_grant_is_refused_at_the_door() {
    assert!(request_is_well_formed(&test_authorization_request(
        "lastDay"
    )));

    let mut invisible_name = test_authorization_request("lastDay");
    invisible_name.client.label = "\u{200B}\u{FEFF}\u{3164}".to_string();
    assert!(
        !request_is_well_formed(&invisible_name),
        "a name that normalizes to nothing cannot key a permission row"
    );

    let mut garbage_minimum = test_authorization_request("forever");
    garbage_minimum.scope.preferred = "lastDay".to_string();
    assert!(
        !request_is_well_formed(&garbage_minimum),
        "no picker selection can ever satisfy an unknown minimum"
    );

    assert!(!request_is_well_formed(&request_preferring("everything")));
}

/// The "Requested via `mnema …`" line is wire input exactly like the label:
/// bidi controls must not reorder the consent copy around it, and its length
/// is capped before it reaches the window.
#[test]
fn the_command_line_is_sanitized_before_it_reaches_the_window() {
    assert_eq!(command_display("search"), "search");
    assert_eq!(
        command_display("search\u{202E}\u{200B} --query\nfoo"),
        "search --query foo"
    );
    let oversized = "a".repeat(CLIENT_LABEL_MAX_CHARS * 4);
    assert_eq!(
        command_display(&oversized).chars().count(),
        CLIENT_LABEL_MAX_CHARS + 1,
        "capped plus a single ellipsis"
    );
    // No name fallback for a command: an empty one renders as bare `mnema`.
    assert_eq!(command_display(""), "");
}

/// The approval window is now the ONLY thing that can resolve a waiter — the
/// native message-box fast path that used to answer inline is gone. So when
/// the window fails to open, nothing resolves it: the socket task awaits
/// forever while holding the single-flight guard, and the pending slot it
/// filled is never emptied, so every later CLI request is answered `busy`
/// for the life of the app. A consent surface that never appears has to be
/// answered here.
#[test]
fn a_consent_surface_that_never_opens_still_answers_and_frees_the_channel() {
    block_on(async {
        let state = BrokerAuthorizationChannelState::default();
        let request = test_authorization_request("lastDay");
        let guard = ActiveRequestGuard::acquire(state.active.clone())
            .expect("the first connection owns the channel");

        let verdict = tokio::time::timeout(
            Duration::from_secs(2),
            await_user_verdict(&state, &request, None, || {
                Err("failed to build the approval window".to_string())
            }),
        )
        .await
        .expect("a request whose consent surface never opened must still be answered");

        assert_eq!(verdict.decision, "denied");
        assert_eq!(verdict.reason.as_deref(), Some("closed"));
        drop(guard);

        // ...and the channel is usable again rather than wedged on `busy`.
        assert!(
            state
                .pending
                .lock()
                .expect("the pending slot is readable")
                .is_none(),
            "the pending slot must not outlive the request that filled it"
        );
        let next = ActiveRequestGuard::acquire(state.active.clone())
            .expect("the next connection can take the guard");
        let (send, _receive) = oneshot::channel();
        assert!(
            store_pending(&state, request, send),
            "the next connection must not be told `busy` forever"
        );
        drop(next);
    });
}

#[test]
fn active_request_guard_allows_only_one_authorization_flow() {
    let active = Arc::new(AtomicBool::new(false));
    let first = ActiveRequestGuard::acquire(active.clone());

    assert!(first.is_some());
    assert!(ActiveRequestGuard::acquire(active.clone()).is_none());

    drop(first);
    assert!(ActiveRequestGuard::acquire(active).is_some());
}
