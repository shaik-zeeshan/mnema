//! A Quick Recall question cancelled before any answer arrived is not kept.
//!
//! Quick Recall's auto-answer starts a turn on every typing pause and cancels it
//! on the next edit, each under a fresh conversation id. Keeping those would
//! leave a trail of empty threads in Chat history, so the cancel path discards
//! the conversation instead — but only when nothing a user could want is lost:
//! it is the conversation's first and only turn, no answer text or block
//! arrived (tool activity alone is not an answer) unless the question was
//! edited into a new one (the partial answers a question no longer asked), and
//! neither the caller nor the stored row says the thread is a Chat one (a Chat
//! user who stops their first message keeps their chat). Stop is not an edit:
//! a stopped partial stays.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use capture_types::AnswerBlock;

use crate::app_infra::AppInfraState;

/// Running turns whose question was edited into a new one (`ask_ai_cancel`
/// with `discard`). Consumed by the turn's finalize; a mark landing after that
/// is cleared when the turn drops its in-flight flag.
static SUPERSEDED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn superseded() -> &'static Mutex<HashSet<String>> {
    SUPERSEDED.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn mark_superseded(conversation_id: &str) {
    if let Ok(mut set) = superseded().lock() {
        set.insert(conversation_id.to_string());
    }
}

pub(super) fn take_superseded(conversation_id: &str) -> bool {
    superseded()
        .lock()
        .is_ok_and(|mut set| set.remove(conversation_id))
}

/// Whether a newer turn has taken over this conversation's in-flight slot (a
/// Chat follow-up displacing ours). Its rows must survive our teardown. A plain
/// `ask_ai_cancel` removes our flag rather than replacing it, so it is not
/// displacement.
pub(super) fn displaced(conversation_id: &str, cancel: &Arc<AtomicBool>) -> bool {
    super::ask_ai_inflight()
        .lock()
        .map(|map| {
            map.get(conversation_id)
                .is_some_and(|flag| !Arc::ptr_eq(flag, cancel))
        })
        .unwrap_or(false)
}

/// Delete the conversation when its cancelled turn is an abandoned Quick
/// Recall question; returns whether it was deleted. `origin` is what this turn
/// was started with, `stored_origin` the row's origin before the turn (if the
/// row existed) — either saying `"chat"` keeps it. `superseded` = the question
/// was edited into a new one, so a partial answer doesn't keep it.
///
/// Race: the caller must skip its terminal persist and title generation when
/// this returns `true`, and must await its spawned partial persists first. No
/// other writer can re-create the row afterwards: the initial persist is
/// awaited before the model runs.
pub(super) async fn discard_if_abandoned(
    infra: &AppInfraState,
    conversation_id: &str,
    origin: &str,
    stored_origin: Option<&str>,
    turn_index: i64,
    superseded: bool,
    answer: &str,
    blocks: &[AnswerBlock],
) -> bool {
    let abandoned = turn_index == 0
        && origin != "chat"
        && stored_origin != Some("chat")
        && (superseded || (answer.trim().is_empty() && blocks.is_empty()));
    if !abandoned {
        return false;
    }
    match infra
        .conversation()
        .delete_conversation(conversation_id)
        .await
    {
        Ok(()) => true,
        Err(error) => {
            tauri_plugin_log::log::warn!(
                "Ask AI failed to discard abandoned conversation {conversation_id}: {error}"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(infra: &AppInfraState, id: &str, origin: &str, turn_index: i64, answer: &str) {
        infra
            .conversation()
            .save_turn(
                id,
                "t",
                origin,
                turn_index,
                "q",
                answer,
                None,
                Some(&[]),
                "[]",
                "[]",
                "streaming",
                None,
                1_000,
            )
            .await
            .expect("seed turn");
    }

    async fn exists(infra: &AppInfraState, id: &str) -> bool {
        infra
            .conversation()
            .get_conversation(id)
            .await
            .expect("read")
            .is_some()
    }

    #[test]
    fn only_an_empty_first_quick_recall_turn_is_discarded() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let dir = tempfile::tempdir().expect("tempdir");
                let infra: AppInfraState = Arc::new(
                    ::app_infra::AppInfra::initialize(dir.path())
                        .await
                        .expect("infra"),
                );
                let prose = [AnswerBlock::Prose {
                    markdown: "Partial".into(),
                }];

                // Quick Recall, only turn, nothing answered → deleted.
                seed(&infra, "qr", "quick_recall", 0, "").await;
                assert!(
                    discard_if_abandoned(&infra, "qr", "quick_recall", None, 0, false, " ", &[])
                        .await
                );
                assert!(!exists(&infra, "qr").await);

                // Chat origin (passed, or stored on the row) → kept.
                seed(&infra, "chat", "chat", 0, "").await;
                assert!(
                    !discard_if_abandoned(&infra, "chat", "chat", None, 0, true, "", &[]).await
                );
                assert!(
                    !discard_if_abandoned(
                        &infra,
                        "chat",
                        "quick_recall",
                        Some("chat"),
                        0,
                        true,
                        "",
                        &[]
                    )
                    .await
                );
                assert!(exists(&infra, "chat").await);

                // Quick Recall with a partial answer → kept.
                seed(&infra, "partial", "quick_recall", 0, "Partial").await;
                assert!(
                    !discard_if_abandoned(
                        &infra,
                        "partial",
                        "quick_recall",
                        None,
                        0,
                        false,
                        "Partial",
                        &prose
                    )
                    .await
                );
                assert!(exists(&infra, "partial").await);

                // …unless the question was edited into a new one → deleted.
                assert!(
                    discard_if_abandoned(
                        &infra,
                        "partial",
                        "quick_recall",
                        None,
                        0,
                        true,
                        "Partial",
                        &prose
                    )
                    .await
                );
                assert!(!exists(&infra, "partial").await);

                // Quick Recall thread with an earlier completed turn → kept.
                seed(&infra, "earlier", "quick_recall", 0, "a").await;
                seed(&infra, "earlier", "quick_recall", 1, "").await;
                assert!(
                    !discard_if_abandoned(
                        &infra,
                        "earlier",
                        "quick_recall",
                        Some("quick_recall"),
                        1,
                        true,
                        "",
                        &[]
                    )
                    .await
                );
                assert!(exists(&infra, "earlier").await);
            });
    }

    #[test]
    fn a_superseded_mark_is_taken_once() {
        let id = "abandoned-superseded-conv";
        assert!(!take_superseded(id));
        mark_superseded(id);
        assert!(take_superseded(id));
        assert!(!take_superseded(id));
    }

    #[test]
    fn a_plain_cancel_is_not_displacement_but_a_newer_turn_is() {
        let id = "abandoned-displaced-conv";
        let ours = super::super::register_inflight(id);
        assert!(!displaced(id, &ours));
        let newer = super::super::register_inflight(id);
        assert!(displaced(id, &ours));
        assert!(!displaced(id, &newer));
        super::super::take_inflight(id);
        assert!(
            !displaced(id, &ours),
            "a cancel takes the flag, nothing displaced us"
        );
    }
}
