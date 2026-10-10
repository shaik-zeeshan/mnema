//! Pinned chats (CH2) and Regenerate's `delete_last_turn` (CH4).

use super::*;

/// One finished turn at `turn_index`, stamped `now_ms`.
async fn done_turn(
    store: &ConversationStore,
    id: &str,
    turn_index: i64,
    question: &str,
    now_ms: i64,
) {
    store
        .save_turn(
            id, "", "chat", turn_index, question, "a", None, None, "[]", "[]", "done", None, now_ms,
        )
        .await
        .expect("turn saves");
}

async fn stamps(store: &ConversationStore, id: &str) -> (i64, i64) {
    let row = sqlx::query(
        "SELECT updated_at_ms, last_activity_at_ms FROM conversations WHERE conversation_id = ?1",
    )
    .bind(id)
    .fetch_one(store.db.read())
    .await
    .expect("row");
    (row.get("updated_at_ms"), row.get("last_activity_at_ms"))
}

#[test]
fn pinned_sorts_first_in_list_and_search() {
    block_on(async {
        let store = test_store().await;
        done_turn(&store, "old", 0, "alpha old", 1_000).await;
        done_turn(&store, "new", 0, "alpha new", 5_000).await;

        assert!(store.set_pinned("old", true).await.expect("pin"));

        let list = store.list_conversations(50, 0).await.expect("list");
        let ids: Vec<&str> = list.iter().map(|s| s.conversation_id.as_str()).collect();
        assert_eq!(ids, ["old", "new"], "pinned sorts above a newer chat");
        assert!(list[0].pinned && !list[1].pinned);

        let hits = store
            .search_conversations("alpha", 50)
            .await
            .expect("search");
        let ids: Vec<&str> = hits.iter().map(|s| s.conversation_id.as_str()).collect();
        assert_eq!(ids, ["old", "new"], "search sorts pinned first too");

        assert!(store.set_pinned("old", false).await.expect("unpin"));
        let list = store.list_conversations(50, 0).await.expect("list");
        assert_eq!(
            list[0].conversation_id, "new",
            "unpinned falls back to recency"
        );
    });
}

#[test]
fn set_pinned_touches_no_timestamp_and_never_creates_a_row() {
    block_on(async {
        let store = test_store().await;
        done_turn(&store, "c", 0, "q", 1_000).await;
        let before = stamps(&store, "c").await;

        store.set_pinned("c", true).await.expect("pin");
        assert_eq!(stamps(&store, "c").await, before);

        assert!(!store.set_pinned("ghost", true).await.expect("pin missing"));
        assert!(store
            .get_conversation("ghost")
            .await
            .expect("get")
            .is_none());
    });
}
