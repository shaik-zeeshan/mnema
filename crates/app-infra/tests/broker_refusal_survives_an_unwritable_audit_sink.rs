//! The one answer the CLI's approval flow keys off must survive its own audit
//! line (ADR 0059).
//!
//! A refusal is now recorded like any other outcome, and that write happens on
//! the path of a caller holding NO permission at all — the exact call whose
//! purpose is to open the approval window. When the sink is unusable (full disk,
//! unwritable config dir, a leftover directory where a lock file belongs), a
//! propagated `Io` error replaces `authorizationRequired` with a broker error.
//! The CLI matches on `outside_grant_scope` before `response_requires_authorization`
//! ever runs, so a first-time tool gets no approval window: CLI Access is bricked
//! with no in-app way back, on the one call that exists to unbrick it. There is no
//! permission here to protect by failing closed.
//!
//! Public API only, no database: an identity with no permission is refused before
//! any capture data is touched, so this exercises exactly the refuse-and-log path.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerErrorResponse, BrokerSearchRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest, BrokeredCaptureResponse,
};
use std::path::PathBuf;
use std::time::SystemTime;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-broker-audit-sink-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn search_request() -> BrokeredCaptureRequest {
    BrokeredCaptureRequest::Search(BrokerSearchRequest {
        query: "quarterly plan".to_string(),
        from: None,
        to: None,
        limit: Some(1),
        app: None,
        window_title: None,
        url: None,
        url_regex: None,
        cursor: None,
        speaker: None,
    })
}

#[test]
fn a_refusal_still_answers_authorization_required_when_the_audit_sink_cannot_be_written() {
    let config_dir = temp_dir("unwritable");
    // A deterministic stand-in for the full disk: the audit lock path is opened
    // for WRITING, and a directory can never be. Nothing about the failure is
    // specific to this shape — it is simply the one a test can guarantee.
    std::fs::create_dir_all(config_dir.join("broker-audit.lock")).expect("lock path is blocked");

    let access = BrokeredCaptureAccess::from_config_dir(&config_dir);
    let who = BrokerClientIdentity::new("Claude Code", BrokerClientIdentitySource::Explicit)
        .expect("label normalizes");
    let response = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds")
        .block_on(access.execute_for_identity(who, search_request()))
        .expect(
            "an unwritable audit sink must not fail the refusal it is trying to log: \
             the CLI keys its approval window off the `authorizationRequired` answer, \
             and a broker error instead means a first-time tool is never offered one",
        );

    assert_eq!(
        response,
        BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required()),
        "the caller needs the one answer that drives the approval flow"
    );
    // Not vacuous: the sink really was unusable, so the line really was lost.
    // Losing an unauthenticated denial line is the deliberate trade — the served
    // reads that name what a tool actually saw still fail on an unrecorded access.
    assert!(
        !config_dir.join("broker-audit.json").exists(),
        "the audit sink was writable after all, so this test proves nothing"
    );
}
