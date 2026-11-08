//! What an Ask AI turn tells the host: the versioned `ask_ai_update` stream and
//! the `conversation_changed` invalidation, on a FakeHost.

use super::*;
use crate::recording_settings::RecordingSettingsState;
use crate::test_support::{core_in, FakeHost};

fn update_events(host: &FakeHost) -> Vec<AskAiUpdateEvent> {
    host.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::AskAiUpdate(update) => Some(update.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_live_turn_reaches_the_host_as_versioned_updates_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let core = core_in(host.clone(), dir.path());
    let conv = "host-events-live-turn";
    let token = next_turn_token();
    register_live_turn(conv, token, fresh_view(2));

    for update in [
        TurnUpdate::Phase {
            phase: "writing".to_string(),
        },
        TurnUpdate::AppendProse {
            text: "Hello".to_string(),
        },
        TurnUpdate::Done,
    ] {
        emit_live_update(&core, conv, token, update);
    }
    // A displaced turn's update never reaches the host.
    assert_eq!(
        emit_live_update(&core, conv, token + 1, TurnUpdate::Done),
        None
    );
    remove_live_turn_if_owner(conv, token);

    let updates = update_events(&host);
    assert_eq!(updates.len(), 3);
    let versions: Vec<u64> = updates.iter().map(|update| update.version).collect();
    assert!(
        versions.windows(2).all(|pair| pair[0] < pair[1]),
        "versions must increase: {versions:?}"
    );
    assert!(updates
        .iter()
        .all(|update| update.conversation_id == conv && update.turn_index == 2));
    assert_eq!(updates[2].update, TurnUpdate::Done);
}

#[test]
fn a_turn_whose_engine_cannot_resolve_saves_then_ends_with_one_terminal_update() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let core = Arc::new(core_in(host.clone(), dir.path()));
    let infra: AppInfraState = Arc::new(
        core.block_on(::app_infra::AppInfra::initialize(&dir.path().join("infra")))
            .expect("infra"),
    );
    core.manage(infra);
    core.manage(RecordingSettingsState::default());
    let conv = "host-events-resolve-failure";
    let cancel = register_inflight(conv);

    core.block_on(run_ask_ai_turn(
        core.clone(),
        conv.to_string(),
        "what did I do?".to_string(),
        "chat".to_string(),
        "Title".to_string(),
        ClientClock::default(),
        None,
        cancel,
    ));

    let events = host.events.lock().unwrap().clone();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0], CoreEvent::ConversationChanged);
    let CoreEvent::AskAiUpdate(update) = &events[1] else {
        panic!("expected the terminal ask_ai_update, got {:?}", events[1]);
    };
    assert_eq!(update.conversation_id, conv);
    assert_eq!((update.version, update.turn_index), (1, 0));
    assert!(matches!(update.update, TurnUpdate::Error { .. }));
    assert!(!turn_in_flight(conv));
}
