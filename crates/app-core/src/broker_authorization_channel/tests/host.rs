//! The window seams, on a `FakeHost`: what the channel asks the host to open
//! and close, and what a request is answered when the window never appears.

use super::*;
use crate::onboarding_state::{persist_onboarding_state, OnboardingState, OnboardingStateStore};
use crate::test_support::{core_in, FakeHost};

fn channel_core(host: Arc<FakeHost>, dir: &std::path::Path) -> Arc<AppCore> {
    let core = Arc::new(core_in(host, dir));
    core.manage(BrokerAuthorizationChannelState::default());
    core.manage(OnboardingStateStore::default());
    persist_onboarding_state(
        &core,
        &core.state::<OnboardingStateStore>(),
        OnboardingState::completed_now(),
    )
    .expect("onboarding completes");
    core
}

/// `Host::open_window` failing is the only signal that the consent surface never
/// appeared, and nothing else resolves the waiter: the connection must still be
/// answered, `closed` (retryable), and the channel freed for the next client.
#[test]
fn a_window_the_host_fails_to_open_answers_the_request_closed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let host = Arc::new(FakeHost::default());
    *host.open_window_error.lock().unwrap() = Some("no window".to_string());
    let core = channel_core(Arc::clone(&host), dir.path());

    let response = core.block_on(async {
        let (mut client, server) = UnixStream::pair().expect("socket pair should open");
        let line = serde_json::to_string(&test_authorization_request("lastDay")).unwrap();
        let ask = async move {
            client
                .write_all(format!("{line}\n").as_bytes())
                .await
                .expect("request writes");
            read_response(client).await
        };
        let (_, response) = tokio::join!(handle_connection(Arc::clone(&core), server), ask);
        response
    });

    assert_eq!(response.decision, "denied");
    assert_eq!(response.reason.as_deref(), Some("closed"));
    assert_eq!(
        *host.opened_windows.lock().unwrap(),
        vec![WindowRequest::CliAccessRequest]
    );
    assert!(
        core.state::<BrokerAuthorizationChannelState>()
            .pending
            .lock()
            .unwrap()
            .is_none(),
        "the pending slot must not outlive the request that filled it"
    );
}

/// Answering from the window tears it down through the host's destroy signal,
/// never a close (which would read as the user dismissing the next request).
#[test]
fn cancelling_from_the_window_signals_its_teardown() {
    let dir = tempfile::tempdir().expect("temp dir");
    let host = Arc::new(FakeHost::default());
    let core = channel_core(Arc::clone(&host), dir.path());

    cancel_pending_cli_access_request(&core).expect("cancel answers");

    assert_eq!(
        *host.window_signals.lock().unwrap(),
        vec![WindowSignal::CloseCliAccessRequest]
    );
    assert!(host.opened_windows.lock().unwrap().is_empty());
}
