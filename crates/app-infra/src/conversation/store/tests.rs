use super::*;
use sqlx::sqlite::SqlitePoolOptions;

mod pin_and_regenerate;

/// Run an async test body on a current-thread runtime (the crate's `tokio`
/// dep does not enable `macros`, so there is no `#[tokio::test]`; this
/// mirrors `user_context/store.rs`'s test pattern).
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(future)
}

/// An in-memory store with just the `0028_*` conversation tables.
async fn test_store() -> ConversationStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory db should open");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("enable foreign keys");
    for statement in [
        "CREATE TABLE conversations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL DEFAULT '',
            origin TEXT NOT NULL DEFAULT 'quick_recall',
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            last_activity_at_ms INTEGER NOT NULL,
            provider TEXT,
            model TEXT,
            generated_title TEXT,
            user_title TEXT,
            pinned INTEGER NOT NULL DEFAULT 0
        )",
        "CREATE TABLE conversation_turns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_row_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
            turn_index INTEGER NOT NULL,
            question TEXT NOT NULL,
            answer TEXT NOT NULL DEFAULT '',
            reasoning TEXT,
            blocks TEXT,
            tool_activities TEXT NOT NULL DEFAULT '[]',
            sources TEXT NOT NULL DEFAULT '[]',
            phase TEXT NOT NULL DEFAULT 'streaming',
            error_message TEXT,
            seeded_result_count INTEGER,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (conversation_row_id, turn_index)
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("conversation test table should be created");
    }
    ConversationStore::new(CaptureDb::single(pool))
}

#[test]
fn save_and_get_round_trips_turns_and_json() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "conv-a",
                "First title",
                "quick_recall",
                0,
                "what did I do?",
                "you coded",
                Some("let me think about what you did"),
                None,
                "[{\"tool\":\"search\"}]",
                "[{\"id\":1}]",
                "done",
                None,
                1_000,
            )
            .await
            .expect("turn 0 saves");
        store
            .save_turn(
                "conv-a",
                "",
                "quick_recall",
                1,
                "and then?",
                "you tested",
                None,
                None,
                "[]",
                "[]",
                "done",
                None,
                2_000,
            )
            .await
            .expect("turn 1 saves");

        let conversation = store
            .get_conversation("conv-a")
            .await
            .expect("get succeeds")
            .expect("conversation exists");

        // First non-empty title wins (the later empty title must not clobber).
        assert_eq!(conversation.title, "First title");
        assert_eq!(conversation.origin, "quick_recall");
        assert_eq!(conversation.turns.len(), 2);
        assert_eq!(conversation.turns[0].question, "what did I do?");
        assert_eq!(conversation.turns[0].answer, "you coded");
        assert_eq!(
            conversation.turns[0].tool_activities,
            serde_json::json!([{ "tool": "search" }])
        );
        assert_eq!(
            conversation.turns[0].sources,
            serde_json::json!([{ "id": 1 }])
        );
        // Reasoning round-trips: `Some(...)` is preserved, `None` stays `None`.
        assert_eq!(
            conversation.turns[0].reasoning.as_deref(),
            Some("let me think about what you did")
        );
        assert_eq!(conversation.turns[1].reasoning, None);
        assert_eq!(conversation.turns[1].turn_index, 1);
    });
}

#[test]
fn save_turn_round_trips_parsed_blocks() {
    block_on(async {
        let store = test_store().await;
        let blocks = vec![
            AnswerBlock::Prose {
                markdown: "Top apps today.".to_string(),
            },
            AnswerBlock::Bars {
                title: Some("Top apps".to_string()),
                items: vec![capture_types::BarsItem {
                    label: "Editor".to_string(),
                    value: 42.0,
                    sublabel: Some("2h".to_string()),
                }],
            },
        ];
        store
            .save_turn(
                "conv-blocks",
                "t",
                "chat",
                0,
                "what did I use?",
                "Top apps today.\n\n```mnema-bars\n...\n```",
                None,
                Some(&blocks),
                "[]",
                "[]",
                "done",
                None,
                1_000,
            )
            .await
            .expect("turn with blocks saves");

        let conversation = store
            .get_conversation("conv-blocks")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        // The render-ready blocks round-trip exactly through the JSON column.
        assert_eq!(conversation.turns[0].blocks.as_deref(), Some(&blocks[..]));
    });
}

#[test]
fn save_turn_with_none_blocks_hydrates_none_and_stores_sql_null() {
    block_on(async {
        let store = test_store().await;
        // A turn saved with `blocks: None` (the LEGACY shape) leaves the
        // column SQL NULL, and `map_turn` hydrates it back as `None`.
        store
            .save_turn(
                "conv-legacy", "t", "chat", 0, "q", "an answer", None, None, "[]", "[]",
                "done", None, 1_000,
            )
            .await
            .expect("legacy turn saves");

        // The raw column is SQL NULL (no JSON written).
        let blocks_col: Option<String> =
            sqlx::query("SELECT blocks FROM conversation_turns WHERE turn_index = 0")
                .fetch_one(store.db.read())
                .await
                .expect("fetch row")
                .get("blocks");
        assert_eq!(blocks_col, None, "None blocks bind SQL NULL");

        // …and `map_turn` distinguishes that NULL as `None` (vs an empty `[]`).
        let conversation = store
            .get_conversation("conv-legacy")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(conversation.turns[0].blocks, None);
    });
}

#[test]
fn save_turn_with_empty_blocks_is_some_not_none() {
    block_on(async {
        let store = test_store().await;
        // An EMPTY parsed set (`Some(&[])`) is a NEW turn with no blocks yet —
        // it must hydrate as `Some(vec![])`, NOT `None` (which means legacy).
        store
            .save_turn(
                "conv-empty", "t", "chat", 0, "q", "", None, Some(&[]), "[]", "[]",
                "streaming", None, 1_000,
            )
            .await
            .expect("empty-blocks turn saves");

        let conversation = store
            .get_conversation("conv-empty")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(conversation.turns[0].blocks.as_deref(), Some(&[][..]));
    });
}

#[test]
fn save_turn_upserts_in_place_on_same_index() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "conv-a", "t", "chat", 0, "q", "", None, None, "[]", "[]", "streaming", None,
                1_000,
            )
            .await
            .expect("initial streaming turn");
        store
            .save_turn(
                "conv-a",
                "t",
                "chat",
                0,
                "q",
                "final answer",
                None,
                None,
                "[]",
                "[]",
                "done",
                None,
                2_000,
            )
            .await
            .expect("finalize same turn");

        let conversation = store
            .get_conversation("conv-a")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(conversation.turns.len(), 1, "same index updates in place");
        assert_eq!(conversation.turns[0].answer, "final answer");
        assert_eq!(conversation.turns[0].phase, "done");
    });
}

/// #L4 regression: the load-bearing `WHERE phase NOT IN ('done', 'error')`
/// guard on the turn upsert (store.rs ~line 149) makes a write to an
/// already-TERMINAL turn a no-op, closing the "permanent Writing…" race where
/// a late streaming update could overwrite a finalized answer/phase. This test
/// pins both halves: a write to a 'done' (and an 'error') turn is dropped, and
/// a non-terminal ('streaming') turn IS updated. Dropping the guard re-opens
/// the race while every other test stays green.
#[test]
fn save_turn_guard_rejects_writes_to_terminal_phase_only() {
    block_on(async {
        let store = test_store().await;

        // A turn finalized to 'done'.
        store
            .save_turn(
                "conv-done", "t", "chat", 0, "q", "final answer", None, None, "[]", "[]",
                "done", None, 1_000,
            )
            .await
            .expect("initial done turn");
        // A late update arrives for that same (conversation, index): the guard
        // must drop it — answer/phase stay finalized.
        store
            .save_turn(
                "conv-done", "t", "chat", 0, "q", "LATE overwrite", None, None, "[]", "[]",
                "streaming", None, 2_000,
            )
            .await
            .expect("late write returns Ok (no-op, not an error)");

        let done = store
            .get_conversation("conv-done")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(done.turns.len(), 1);
        assert_eq!(
            done.turns[0].answer, "final answer",
            "terminal 'done' turn is not overwritten by a late write"
        );
        assert_eq!(done.turns[0].phase, "done");

        // Same for a turn already in the terminal 'error' phase.
        store
            .save_turn(
                "conv-error", "t", "chat", 0, "q", "", None, None, "[]", "[]", "error",
                Some("boom"), 1_000,
            )
            .await
            .expect("initial error turn");
        store
            .save_turn(
                "conv-error", "t", "chat", 0, "q", "RECOVERED", None, None, "[]", "[]", "done",
                None, 2_000,
            )
            .await
            .expect("late write returns Ok (no-op)");

        let errored = store
            .get_conversation("conv-error")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(
            errored.turns[0].phase, "error",
            "terminal 'error' turn is not revived by a late write"
        );
        assert_eq!(errored.turns[0].answer, "");

        // Control: a NON-terminal ('streaming') turn IS updated in place, so
        // the guard rejects only terminal phases (not all updates).
        store
            .save_turn(
                "conv-live", "t", "chat", 0, "q", "partial", None, None, "[]", "[]", "streaming",
                None, 1_000,
            )
            .await
            .expect("initial streaming turn");
        store
            .save_turn(
                "conv-live", "t", "chat", 0, "q", "final answer", None, None, "[]", "[]", "done",
                None, 2_000,
            )
            .await
            .expect("finalize streaming turn");

        let live = store
            .get_conversation("conv-live")
            .await
            .expect("get succeeds")
            .expect("conversation exists");
        assert_eq!(
            live.turns[0].answer, "final answer",
            "non-terminal turn IS updated through the guard"
        );
        assert_eq!(live.turns[0].phase, "done");
    });
}

#[test]
fn list_orders_newest_updated_first_with_preview() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "older", "Older", "chat", 0, "old question", "", None, None, "[]", "[]",
                "done", None, 1_000,
            )
            .await
            .expect("older saves");
        store
            .save_turn(
                "newer", "Newer", "chat", 0, "new question", "", None, None, "[]", "[]",
                "done", None, 5_000,
            )
            .await
            .expect("newer saves");

        let summaries = store.list_conversations(50, 0).await.expect("list succeeds");
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].conversation_id, "newer");
        assert_eq!(summaries[0].preview, "new question");
        assert_eq!(summaries[0].turn_count, 1);
        assert_eq!(summaries[1].conversation_id, "older");
    });
}

#[test]
fn search_matches_title_question_and_answer() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "c1",
                "Rust refactor",
                "chat",
                0,
                "how do I borrow?",
                "use a reference",
                None,
                None,
                "[]",
                "[]",
                "done",
                None,
                1_000,
            )
            .await
            .expect("c1 saves");
        store
            .save_turn(
                "c2", "Cooking", "chat", 0, "pasta recipe", "boil water", None, None, "[]", "[]",
                "done", None, 2_000,
            )
            .await
            .expect("c2 saves");

        // Title match.
        let by_title = store.search_conversations("rust", 50).await.expect("search");
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].conversation_id, "c1");

        // Question match.
        let by_question = store.search_conversations("BORROW", 50).await.expect("search");
        assert_eq!(by_question.len(), 1);
        assert_eq!(by_question[0].conversation_id, "c1");

        // Answer match.
        let by_answer = store.search_conversations("boil", 50).await.expect("search");
        assert_eq!(by_answer.len(), 1);
        assert_eq!(by_answer[0].conversation_id, "c2");

        // No match.
        let none = store.search_conversations("zzz", 50).await.expect("search");
        assert!(none.is_empty());
    });
}

#[test]
fn search_dedupes_per_conversation_on_multiple_turn_matches() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "c1", "t", "chat", 0, "alpha one", "", None, None, "[]", "[]", "done", None,
                1_000,
            )
            .await
            .expect("turn 0");
        store
            .save_turn(
                "c1", "t", "chat", 1, "alpha two", "", None, None, "[]", "[]", "done", None,
                2_000,
            )
            .await
            .expect("turn 1");

        let hits = store.search_conversations("alpha", 50).await.expect("search");
        assert_eq!(hits.len(), 1, "two matching turns yield one conversation row");
    });
}

#[test]
fn delete_removes_conversation_and_cascades_turns() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "c1", "t", "chat", 0, "q", "", None, None, "[]", "[]", "done", None, 1_000,
            )
            .await
            .expect("saves");
        store.delete_conversation("c1").await.expect("delete");
        assert!(store
            .get_conversation("c1")
            .await
            .expect("get")
            .is_none());
    });
}

#[test]
fn wipe_clears_all() {
    block_on(async {
        let store = test_store().await;
        for id in ["a", "b", "c"] {
            store
                .save_turn(
                    id, "t", "chat", 0, "q", "", None, None, "[]", "[]", "done", None, 1_000,
                )
                .await
                .expect("saves");
        }
        store.wipe_all().await.expect("wipe");
        assert!(store
            .list_conversations(50, 0)
            .await
            .expect("list")
            .is_empty());
    });
}

#[test]
fn engine_pin_round_trips_and_survives_a_later_turn() {
    block_on(async {
        let store = test_store().await;

        // A conversation that does not exist yet reads no pin.
        assert!(store
            .get_conversation_engine("conv-pin")
            .await
            .expect("read engine")
            .is_none());

        // Pinning before any turn creates the row and stores the identity.
        store
            .set_conversation_engine("conv-pin", Some("anthropic"), Some("claude-x"), 1_000)
            .await
            .expect("set engine pin");

        assert_eq!(
            store
                .get_conversation_engine("conv-pin")
                .await
                .expect("read engine"),
            Some((Some("anthropic".to_string()), Some("claude-x".to_string()))),
        );

        // The pin is also hydrated onto the full conversation.
        let pinned = store
            .get_conversation("conv-pin")
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(pinned.provider.as_deref(), Some("anthropic"));
        assert_eq!(pinned.model.as_deref(), Some("claude-x"));

        // A turn saved AFTER pinning must not clobber the pin.
        store
            .save_turn(
                "conv-pin", "Pinned", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None,
                2_000,
            )
            .await
            .expect("turn saves");
        assert_eq!(
            store
                .get_conversation_engine("conv-pin")
                .await
                .expect("read engine"),
            Some((Some("anthropic".to_string()), Some("claude-x".to_string()))),
            "save_turn must not clear an existing pin",
        );

        // Clearing the pin (None/None) leaves the row but unpins it.
        store
            .set_conversation_engine("conv-pin", None, None, 3_000)
            .await
            .expect("clear engine pin");
        assert_eq!(
            store
                .get_conversation_engine("conv-pin")
                .await
                .expect("read engine"),
            Some((None, None)),
        );
    });
}

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

#[test]
fn unpinned_conversation_reads_no_engine() {
    block_on(async {
        let store = test_store().await;
        store
            .save_turn(
                "plain", "Plain", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None,
                1_000,
            )
            .await
            .expect("turn saves");
        // An existing-but-unpinned conversation returns Some((None, None)).
        assert_eq!(
            store
                .get_conversation_engine("plain")
                .await
                .expect("read engine"),
            Some((None, None)),
        );
    });
}
