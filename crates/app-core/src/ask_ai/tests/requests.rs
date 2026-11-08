use super::*;

#[test]
fn followup_request_deserializes_camel_case() {
    let request: AskAiFollowupRequest =
        serde_json::from_str(r#"{"conversationId":"conv-1","question":"what about in Slack?"}"#)
            .expect("follow-up request should deserialize");
    assert_eq!(request.conversation_id, "conv-1");
    assert_eq!(request.question, "what about in Slack?");

    let request: AskAiFollowupRequest = serde_json::from_str(
        r#"{"conversationId":"conv-2","question":"more","unknownField":"ignored"}"#,
    )
    .expect("extra fields are ignored");
    assert_eq!(request.conversation_id, "conv-2");
    assert_eq!(request.question, "more");
    assert_eq!(request.scope, None);

    let request: AskAiFollowupRequest = serde_json::from_str(
        r#"{"conversationId":"c","question":"q","scope":{"fromMs":1,"toMs":2,"aboutYou":false}}"#,
    )
    .expect("a scoped follow-up deserializes");
    assert_eq!(
        request.scope,
        Some(AskAiScope {
            from_ms: Some(1),
            to_ms: Some(2),
            about_you: false
        })
    );
}

#[test]
fn start_request_deserializes_without_optional_fields() {
    // Existing callers (no origin/title/priorTranscript) keep working.
    let request: AskAiStartRequest =
        serde_json::from_str(r#"{"conversationId":"conv-1","question":"what did I do?"}"#)
            .expect("start request without optional fields should deserialize");
    assert_eq!(request.conversation_id, "conv-1");
    assert_eq!(request.question, "what did I do?");
    assert!(request.origin.is_none());
    assert!(request.title.is_none());
    assert!(request.prior_transcript.is_none());
}

#[test]
fn start_request_accepts_origin_title_and_ignores_prior_transcript() {
    let request: AskAiStartRequest = serde_json::from_str(
        r#"{"conversationId":"c","question":"q","origin":"chat","title":"My chat","priorTranscript":[{"question":"q1","answer":"a1"}]}"#,
    )
    .expect("start request with origin/title/priorTranscript should deserialize");
    assert_eq!(request.origin.as_deref(), Some("chat"));
    assert_eq!(request.title.as_deref(), Some("My chat"));
    // priorTranscript still deserializes (as opaque JSON) but is ignored.
    assert!(request.prior_transcript.is_some());
}

#[test]
fn title_generation_is_first_turn_only() {
    // Eligibility rule: only the FIRST turn of a thread generates a title;
    // a follow-up (second turn onward) is a no-op.
    assert!(title_generation_eligible(0));
    assert!(!title_generation_eligible(1));
    assert!(!title_generation_eligible(7));
}

#[test]
fn title_prompt_sees_only_the_question_text() {
    let prompt = build_title_prompt("what did I work on yesterday?");
    assert!(prompt.contains("what did I work on yesterday?"));
    // No capture data / transcript / tool text leaks into the title call.
    assert!(!prompt.contains("Context from the user's captures"));

    let preamble = build_title_preamble();
    assert!(preamble.contains("3-6 word"));
}

#[test]
fn normalize_generated_title_accepts_a_clean_short_title() {
    assert_eq!(
        normalize_generated_title("Yesterday's coding session"),
        Some("Yesterday's coding session".to_string())
    );
    // Wrapping quotes, trailing punctuation, and ragged whitespace are
    // stripped/collapsed rather than rejected.
    assert_eq!(
        normalize_generated_title("\u{201c}Rust borrow checker help.\u{201d}"),
        Some("Rust borrow checker help".to_string())
    );
    assert_eq!(
        normalize_generated_title("  Weekly   planning  recap \n"),
        Some("Weekly planning recap".to_string())
    );
}

#[test]
fn normalize_generated_title_rejects_empty_and_overlong() {
    // Empty / whitespace / quote-only results are failures.
    assert_eq!(normalize_generated_title(""), None);
    assert_eq!(normalize_generated_title("   "), None);
    assert_eq!(normalize_generated_title("\"\""), None);
    // Too many words (the model ignored the 3–6 word instruction).
    assert_eq!(
        normalize_generated_title(
            "This is a much too long title that rambles on and on about everything"
        ),
        None
    );
    // Few words but absurdly long characters-wise.
    let overlong = format!("{} {}", "a".repeat(40), "b".repeat(40));
    assert_eq!(normalize_generated_title(&overlong), None);
}

#[test]
fn inflight_registry_register_take_roundtrip() {
    let id = "inflight-roundtrip-conv";
    let cancel = register_inflight(id);
    assert!(!cancel.load(Ordering::SeqCst));
    let taken = take_inflight(id).expect("registered flag should be takeable");
    assert!(Arc::ptr_eq(&cancel, &taken));
    // Once taken, the registry no longer holds it.
    assert!(take_inflight(id).is_none());
}

#[test]
fn register_cancels_displaced_inflight_turn() {
    let id = "inflight-displace-conv";
    let first = register_inflight(id);
    assert!(!first.load(Ordering::SeqCst));
    // A racing second start/follow-up displaces the first, cancelling it.
    let second = register_inflight(id);
    assert!(
        first.load(Ordering::SeqCst),
        "displacing an in-flight turn must set its cancel flag"
    );
    assert!(!second.load(Ordering::SeqCst));
    // Cleanup so the static map does not leak into other tests.
    let _ = take_inflight(id);
}

#[test]
fn cancel_before_register_is_honored_by_next_register() {
    // The "cancel at the very start of a request" race: the frontend shows the
    // Stop button the instant a turn is sent, but the backend registers the
    // turn's cancel flag only after an async access-readiness check. A cancel
    // landing in that window finds no flag — it must NOT be dropped. Instead it
    // is recorded as pending, and the imminent registration must start its flag
    // already cancelled so the turn short-circuits before any model call.
    let id = "inflight-cancel-before-register-conv";
    // No flag registered yet — this is what `ask_ai_cancel` does in the race.
    record_cancel_before_register(id);
    // The turn finally registers; it must come up already cancelled.
    let cancel = register_inflight(id);
    assert!(
        cancel.load(Ordering::SeqCst),
        "a cancel that arrived before registration must carry over to the turn's flag"
    );
    // The pending cancel is consumed exactly once: a *subsequent* fresh turn for
    // the same conversation must start uncancelled.
    let _ = take_inflight(id);
    let next = register_inflight(id);
    assert!(
        !next.load(Ordering::SeqCst),
        "a pending cancel must apply to one registration only, not future turns"
    );
    let _ = take_inflight(id);
}

#[test]
fn a_stale_cancel_for_a_finished_turn_does_not_cancel_a_later_follow_up() {
    // The blank Chat follow-up: Quick Recall's teardown cancels its thread
    // after the answer finished and "Continue in Chat" handed it over. No
    // flag is registered, so `ask_ai_cancel` records it as pending — and the
    // user's first follow-up in Chat, seconds later, came up cancelled: Done
    // with no answer, no row saved, Retry → "turn 1 is not the last turn".
    let id = "inflight-stale-cancel-conv";
    record_cancel_before_register(id);
    let follow_up = register_inflight_at(id, std::time::Instant::now() + PENDING_CANCEL_TTL);
    assert!(
        !follow_up.load(Ordering::SeqCst),
        "a cancel older than the start/cancel race window must not cancel a new turn"
    );
    let _ = take_inflight(id);
    // And it is consumed, not left to hit a turn after this one.
    let next = register_inflight(id);
    assert!(!next.load(Ordering::SeqCst));
    let _ = take_inflight(id);
}

#[test]
fn cancel_with_live_turn_takes_flag_and_skips_pending() {
    // When a flag IS registered, cancel takes + sets it (existing behavior) and
    // must NOT leave a pending entry that would poison the next turn.
    let id = "inflight-cancel-live-conv";
    let cancel = register_inflight(id);
    // Mirror `ask_ai_cancel`'s body: take wins, so no pending recorded.
    let taken = take_inflight(id);
    if let Some(flag) = taken {
        flag.store(true, Ordering::SeqCst);
    } else {
        record_cancel_before_register(id);
    }
    assert!(cancel.load(Ordering::SeqCst));
    // Next turn must be clean — no leaked pending cancel.
    let next = register_inflight(id);
    assert!(!next.load(Ordering::SeqCst));
    let _ = take_inflight(id);
}

#[test]
fn turn_in_flight_tracks_the_registered_flag() {
    let id = "inflight-delete-guard-conv";
    assert!(!turn_in_flight(id));
    let cancel = register_inflight(id);
    assert!(turn_in_flight(id), "delete_last_turn must refuse mid-turn");
    remove_inflight_if_owner(id, &cancel);
    assert!(!turn_in_flight(id));
}

#[test]
fn remove_if_owner_spares_a_displacing_turn() {
    let id = "inflight-owner-conv";
    let first = register_inflight(id);
    let second = register_inflight(id);
    // The first (displaced) turn finishing must NOT evict the newer flag.
    remove_inflight_if_owner(id, &first);
    let still = take_inflight(id).expect("newer flag should still be registered");
    assert!(Arc::ptr_eq(&still, &second));
}

#[test]
fn a_late_superseded_mark_is_dropped_with_the_turns_flag() {
    let id = "inflight-late-superseded-conv";
    let ours = register_inflight(id);
    // A displaced turn finishing leaves the newer turn's mark alone.
    let newer = register_inflight(id);
    abandoned::mark_superseded(id);
    remove_inflight_if_owner(id, &ours);
    assert!(abandoned::take_superseded(id));
    // The owner finishing drops a mark that arrived after its finalize.
    abandoned::mark_superseded(id);
    remove_inflight_if_owner(id, &newer);
    assert!(!abandoned::take_superseded(id));
}
