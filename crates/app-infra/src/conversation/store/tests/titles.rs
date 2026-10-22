//! Conversation titles: effective-title precedence plus generated and
//! user title writes.

use super::*;

#[test]
fn effective_title_prefers_user_then_generated_then_stored_then_preview() {
    assert_eq!(
        effective_title(
            Some("User".into()),
            Some("Generated".into()),
            "Stored".into(),
            "preview"
        ),
        "User"
    );
    assert_eq!(
        effective_title(None, Some("Generated".into()), "Stored".into(), "preview"),
        "Generated"
    );
    assert_eq!(
        effective_title(None, None, "Stored".into(), "preview"),
        "Stored"
    );
    assert_eq!(effective_title(None, None, "".into(), "preview"), "preview");
    // Blank candidates are skipped, not surfaced.
    assert_eq!(
        effective_title(Some("  ".into()), Some("".into()), " ".into(), "preview"),
        "preview"
    );
}

#[test]
fn generated_title_persists_once_and_surfaces_in_list_and_get() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "conv-t",
                "what did I do yesterday afternoon exactly?",
                "chat",
                0,
                "what did I do yesterday afternoon exactly?",
                "you coded",
                None,
                None,
                "[]",
                "[]",
                "done",
                None,
                1_000,
            )
            .await
            .expect("turn saves");

        // First generated write lands…
        assert!(store
            .set_generated_title_if_unset("conv-t", "Yesterday afternoon recap")
            .await
            .expect("generated title write"));
        // …and a second one is a no-op (generation is once-per-thread).
        assert!(!store
            .set_generated_title_if_unset("conv-t", "Another title")
            .await
            .expect("second generated write"));

        let summaries = store.list_conversations(50, 0).await.expect("list");
        assert_eq!(summaries[0].title, "Yesterday afternoon recap");
        let conversation = store
            .get_conversation("conv-t")
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(conversation.title, "Yesterday afternoon recap");
    });
}

#[test]
fn failed_title_generation_leaves_fallback_title() {
    block_on(async {
        let store = test_store().await;
        // A turn persisted with an EMPTY upsert title and no generated-title
        // write (the engine call failed / was unavailable): the list and get
        // paths fall back to the first-question truncation, and the turn row
        // itself is untouched.
        store
            .save_turn(
                "conv-fb", "", "chat", 0, "what changed?", "a lot", None, None, "[]", "[]", "done",
                None, 1_000,
            )
            .await
            .expect("turn saves");

        let summaries = store.list_conversations(50, 0).await.expect("list");
        assert_eq!(summaries[0].title, "what changed?");
        let conversation = store
            .get_conversation("conv-fb")
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(conversation.title, "what changed?");
        assert_eq!(conversation.turns[0].answer, "a lot");
        assert_eq!(conversation.turns[0].phase, "done");
    });
}

#[test]
fn user_title_persists_and_wins_over_a_later_generated_write() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "conv-r", "first question", "chat", 0, "first question", "a", None, None,
                "[]", "[]", "done", None, 1_000,
            )
            .await
            .expect("turn saves");

        // The user renames while title generation is still in flight…
        assert!(store
            .set_user_title("conv-r", "My renamed thread", 2_000)
            .await
            .expect("user title write"));
        // …so the late generated write is rejected by the WHERE guard.
        assert!(!store
            .set_generated_title_if_unset("conv-r", "Late generated title")
            .await
            .expect("late generated write"));

        let summaries = store.list_conversations(50, 0).await.expect("list");
        assert_eq!(summaries[0].title, "My renamed thread");
    });
}

#[test]
fn user_title_overrides_an_earlier_generated_title() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "conv-o", "q", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None,
                1_000,
            )
            .await
            .expect("turn saves");
        assert!(store
            .set_generated_title_if_unset("conv-o", "Generated title")
            .await
            .expect("generated write"));
        assert!(store
            .set_user_title("conv-o", "User title", 2_000)
            .await
            .expect("user write"));

        let summaries = store.list_conversations(50, 0).await.expect("list");
        assert_eq!(summaries[0].title, "User title");
    });
}

#[test]
fn title_writes_against_missing_conversation_are_rejected() {
    block_on(async {
        let store = test_store().await;
        assert!(!store
            .set_user_title("ghost", "Title", 1_000)
            .await
            .expect("user write"));
        assert!(!store
            .set_generated_title_if_unset("ghost", "Title")
            .await
            .expect("generated write"));
    });
}

#[test]
fn search_matches_user_and_generated_titles() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "c1", "", "chat", 0, "question one", "answer", None, None, "[]", "[]", "done",
                None, 1_000,
            )
            .await
            .expect("c1 saves");
        store
            .save_turn(
                "c2", "", "chat", 0, "question two", "answer", None, None, "[]", "[]", "done",
                None, 2_000,
            )
            .await
            .expect("c2 saves");
        store
            .set_generated_title_if_unset("c1", "Kubernetes debugging")
            .await
            .expect("generated write");
        store
            .set_user_title("c2", "Holiday planning", 3_000)
            .await
            .expect("user write");

        let by_generated = store
            .search_conversations("kubernetes", 50)
            .await
            .expect("search");
        assert_eq!(by_generated.len(), 1);
        assert_eq!(by_generated[0].conversation_id, "c1");

        let by_user = store
            .search_conversations("holiday", 50)
            .await
            .expect("search");
        assert_eq!(by_user.len(), 1);
        assert_eq!(by_user[0].conversation_id, "c2");
    });
}
