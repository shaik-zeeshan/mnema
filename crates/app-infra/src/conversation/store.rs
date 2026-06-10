//! `ConversationStore` — SQLite-backed storage for persistent Quick Recall /
//! Chat conversations (issue #102, ADR 0031).
//!
//! ONE shared store backs both doors. It owns the `0028_*` tables
//! (`conversations`, `conversation_turns`). Conversations OBEY Retention Policy
//! (aged out via [`ConversationStore::delete_conversations_older_than`], driven
//! by `last_activity_at_ms`) and are CLEARED by Wipe User Context
//! ([`ConversationStore::wipe_all`]).
//!
//! Timestamps are INTEGER unix milliseconds; the caller stamps `now_ms` so the
//! store stays deterministic. `tool_activities` / `sources` are stored verbatim
//! as JSON text and parsed back into `serde_json::Value` on read.

use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

use capture_types::{Conversation, ConversationSummary, ConversationTurn};

use crate::Result;

/// Max characters of the first question kept as a history-list preview.
const PREVIEW_CHAR_CAP: usize = 140;

/// SQLite-backed storage for persistent conversations.
#[derive(Clone)]
pub struct ConversationStore {
    pool: SqlitePool,
}

impl ConversationStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert (or refresh) a conversation row, returning its `conversations.id`.
    ///
    /// On conflict with an existing `conversation_id`: bump `updated_at_ms` /
    /// `last_activity_at_ms`, and set `title` only when the stored title is still
    /// empty (so the FIRST non-empty title wins and later empty titles never
    /// clobber it). `origin` is preserved from the creating door (never
    /// overwritten on conflict).
    pub async fn upsert_conversation(
        &self,
        conversation_id: &str,
        title: &str,
        origin: &str,
        now_ms: i64,
    ) -> Result<i64> {
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, title, origin, created_at_ms, updated_at_ms, last_activity_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?4, ?4) \
             ON CONFLICT(conversation_id) DO UPDATE SET \
                title = CASE WHEN conversations.title = '' THEN excluded.title ELSE conversations.title END, \
                updated_at_ms = excluded.updated_at_ms, \
                last_activity_at_ms = excluded.last_activity_at_ms",
        )
        .bind(conversation_id)
        .bind(title)
        .bind(origin)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query("SELECT id FROM conversations WHERE conversation_id = ?1")
            .bind(conversation_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("id"))
    }

    /// Upsert one turn of a conversation. The conversation row is ensured first
    /// (via [`Self::upsert_conversation`], which also bumps its activity stamps),
    /// then the turn is inserted or — on conflict with an existing
    /// `(conversation_row_id, turn_index)` — updated in place.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_turn(
        &self,
        conversation_id: &str,
        title: &str,
        origin: &str,
        turn_index: i64,
        question: &str,
        answer: &str,
        tool_activities_json: &str,
        sources_json: &str,
        phase: &str,
        error_message: Option<&str>,
        seeded_result_count: Option<i64>,
        now_ms: i64,
    ) -> Result<()> {
        // Ensure the conversation exists (and bump its activity stamps).
        let conversation_row_id = self
            .upsert_conversation(conversation_id, title, origin, now_ms)
            .await?;

        sqlx::query(
            "INSERT INTO conversation_turns \
                (conversation_row_id, turn_index, question, answer, tool_activities, sources, \
                 phase, error_message, seeded_result_count, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
             ON CONFLICT(conversation_row_id, turn_index) DO UPDATE SET \
                question = excluded.question, \
                answer = excluded.answer, \
                tool_activities = excluded.tool_activities, \
                sources = excluded.sources, \
                phase = excluded.phase, \
                error_message = excluded.error_message, \
                seeded_result_count = excluded.seeded_result_count, \
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(conversation_row_id)
        .bind(turn_index)
        .bind(question)
        .bind(answer)
        .bind(tool_activities_json)
        .bind(sources_json)
        .bind(phase)
        .bind(error_message)
        .bind(seeded_result_count)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List conversations newest-first (by `updated_at_ms`), each as a summary
    /// carrying its turn count + a short preview (first turn's question).
    pub async fn list_conversations(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ConversationSummary>> {
        let rows = sqlx::query(
            "SELECT c.conversation_id AS conversation_id, c.title AS title, c.origin AS origin, \
                    c.created_at_ms AS created_at_ms, c.updated_at_ms AS updated_at_ms, \
                    (SELECT COUNT(*) FROM conversation_turns t WHERE t.conversation_row_id = c.id) AS turn_count, \
                    (SELECT t.question FROM conversation_turns t \
                     WHERE t.conversation_row_id = c.id \
                     ORDER BY t.turn_index ASC LIMIT 1) AS preview \
             FROM conversations c \
             ORDER BY c.updated_at_ms DESC, c.id DESC \
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(map_summary).collect())
    }

    /// Hydrate one conversation (with its turns in `turn_index` order) by its
    /// frontend UUID. `None` when absent.
    pub async fn get_conversation(&self, conversation_id: &str) -> Result<Option<Conversation>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, title, origin, created_at_ms, updated_at_ms, \
                    provider, model \
             FROM conversations WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let row_id: i64 = row.get("id");
        let turns = self.list_turns(row_id).await?;
        Ok(Some(Conversation {
            conversation_id: row.get("conversation_id"),
            title: row.get("title"),
            origin: row.get("origin"),
            created_at_ms: row.get("created_at_ms"),
            updated_at_ms: row.get("updated_at_ms"),
            provider: row.get("provider"),
            model: row.get("model"),
            turns,
        }))
    }

    /// Pin (or clear) the engine identity for a conversation. UPDATEs the
    /// `provider`/`model` columns and bumps `updated_at_ms` / `last_activity_at_ms`
    /// (a pin is an activity). The conversation row is ensured first (a pin may be
    /// set before the first turn); a `None` provider/model clears the pin →
    /// unpinned (use the global default engine).
    ///
    /// This is the ONLY writer of `provider`/`model`: [`Self::upsert_conversation`]
    /// (and `save_turn` through it) deliberately leaves the pin untouched on
    /// conflict so a later turn never clobbers an earlier pin.
    pub async fn set_conversation_engine(
        &self,
        conversation_id: &str,
        provider: Option<&str>,
        model: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        // Ensure the row exists (and bump its activity stamps). `title`/`origin`
        // here are upsert defaults that only apply when the row is newly created;
        // an existing row keeps its first non-empty title and original origin.
        self.upsert_conversation(conversation_id, "", "quick_recall", now_ms)
            .await?;

        sqlx::query(
            "UPDATE conversations SET \
                provider = ?2, model = ?3, \
                updated_at_ms = ?4, last_activity_at_ms = ?4 \
             WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .bind(provider)
        .bind(model)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read the engine pin `(provider, model)` for a conversation without
    /// hydrating its turns. `None` when the conversation row does not exist; an
    /// existing-but-unpinned row returns `Some((None, None))`. The Ask AI slice
    /// uses this to resolve a thread's engine identity cheaply.
    pub async fn get_conversation_engine(
        &self,
        conversation_id: &str,
    ) -> Result<Option<(Option<String>, Option<String>)>> {
        let row = sqlx::query(
            "SELECT provider, model FROM conversations WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (row.get("provider"), row.get("model"))))
    }

    async fn list_turns(&self, conversation_row_id: i64) -> Result<Vec<ConversationTurn>> {
        let rows = sqlx::query(
            "SELECT turn_index, question, answer, tool_activities, sources, phase, \
                    error_message, seeded_result_count, created_at_ms, updated_at_ms \
             FROM conversation_turns \
             WHERE conversation_row_id = ?1 \
             ORDER BY turn_index ASC",
        )
        .bind(conversation_row_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(map_turn).collect())
    }

    /// Case-insensitive search across conversation titles and any turn's
    /// question/answer. Newest-first (by `updated_at_ms`), deduped per
    /// conversation, capped at `limit`.
    pub async fn search_conversations(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>> {
        // Escape LIKE wildcards in the user term so `%`/`_` match literally.
        let pattern = format!("%{}%", escape_like(query));
        let rows = sqlx::query(
            "SELECT c.conversation_id AS conversation_id, c.title AS title, c.origin AS origin, \
                    c.created_at_ms AS created_at_ms, c.updated_at_ms AS updated_at_ms, \
                    (SELECT COUNT(*) FROM conversation_turns t WHERE t.conversation_row_id = c.id) AS turn_count, \
                    (SELECT t.question FROM conversation_turns t \
                     WHERE t.conversation_row_id = c.id \
                     ORDER BY t.turn_index ASC LIMIT 1) AS preview \
             FROM conversations c \
             WHERE c.title LIKE ?1 ESCAPE '\\' COLLATE NOCASE \
                OR EXISTS (\
                    SELECT 1 FROM conversation_turns t \
                    WHERE t.conversation_row_id = c.id \
                      AND (t.question LIKE ?1 ESCAPE '\\' COLLATE NOCASE \
                           OR t.answer LIKE ?1 ESCAPE '\\' COLLATE NOCASE)\
                ) \
             ORDER BY c.updated_at_ms DESC, c.id DESC \
             LIMIT ?2",
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(map_summary).collect())
    }

    /// Delete a conversation (its turns cascade via FK). A no-op when absent.
    pub async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM conversations WHERE conversation_id = ?1")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// **Wipe User Context** clears all conversations too: in ONE transaction,
    /// delete every turn then every conversation (children first so it is
    /// correct regardless of FK enforcement).
    pub async fn wipe_all(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM conversation_turns")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Retention aging: delete every conversation whose `last_activity_at_ms`
    /// falls before `cutoff_ms` (turns cascade via FK). Returns the number of
    /// conversations deleted. Mirrors the local-calendar cutoff capture cleanup
    /// uses (the caller converts the RFC3339 cutoff to unix millis).
    pub async fn delete_conversations_older_than(&self, cutoff_ms: i64) -> Result<u64> {
        let result = sqlx::query("DELETE FROM conversations WHERE last_activity_at_ms < ?1")
            .bind(cutoff_ms)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

/// Parse a JSON column back into a `serde_json::Value`, falling back to JSON
/// `null` on a parse failure (a corrupt/legacy value never breaks hydration).
fn parse_json(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or(serde_json::Value::Null)
}

/// Truncate a preview question to [`PREVIEW_CHAR_CAP`] chars on a char boundary.
fn truncate_preview(question: &str) -> String {
    if question.chars().count() <= PREVIEW_CHAR_CAP {
        return question.to_string();
    }
    question.chars().take(PREVIEW_CHAR_CAP).collect()
}

/// Escape `%`, `_`, and `\` in a user search term so they match literally under
/// a `LIKE ... ESCAPE '\\'`.
fn escape_like(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for ch in term.chars() {
        match ch {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// Map a `conversation_turns` row onto a [`ConversationTurn`].
fn map_turn(row: SqliteRow) -> ConversationTurn {
    let tool_activities: String = row.get("tool_activities");
    let sources: String = row.get("sources");
    ConversationTurn {
        turn_index: row.get("turn_index"),
        question: row.get("question"),
        answer: row.get("answer"),
        tool_activities: parse_json(&tool_activities),
        sources: parse_json(&sources),
        phase: row.get("phase"),
        error_message: row.get("error_message"),
        seeded_result_count: row.get("seeded_result_count"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    }
}

/// Map a history-list row onto a [`ConversationSummary`].
fn map_summary(row: SqliteRow) -> ConversationSummary {
    let preview: Option<String> = row.get("preview");
    ConversationSummary {
        conversation_id: row.get("conversation_id"),
        title: row.get("title"),
        origin: row.get("origin"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
        turn_count: row.get("turn_count"),
        preview: preview.as_deref().map(truncate_preview).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

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
                model TEXT
            )",
            "CREATE TABLE conversation_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_row_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                turn_index INTEGER NOT NULL,
                question TEXT NOT NULL,
                answer TEXT NOT NULL DEFAULT '',
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
        ConversationStore::new(pool)
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
                    "[{\"tool\":\"search\"}]",
                    "[{\"id\":1}]",
                    "done",
                    None,
                    Some(3),
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
                    "[]",
                    "[]",
                    "done",
                    None,
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
            assert_eq!(conversation.turns[0].seeded_result_count, Some(3));
            assert_eq!(conversation.turns[1].turn_index, 1);
        });
    }

    #[test]
    fn save_turn_upserts_in_place_on_same_index() {
        block_on(async {
            let store = test_store().await;
            store
                .save_turn(
                    "conv-a", "t", "chat", 0, "q", "", "[]", "[]", "streaming", None, None, 1_000,
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
                    "[]",
                    "[]",
                    "done",
                    None,
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

    #[test]
    fn list_orders_newest_updated_first_with_preview() {
        block_on(async {
            let store = test_store().await;
            store
                .save_turn(
                    "older", "Older", "chat", 0, "old question", "", "[]", "[]", "done", None,
                    None, 1_000,
                )
                .await
                .expect("older saves");
            store
                .save_turn(
                    "newer", "Newer", "chat", 0, "new question", "", "[]", "[]", "done", None,
                    None, 5_000,
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
                    "[]",
                    "[]",
                    "done",
                    None,
                    None,
                    1_000,
                )
                .await
                .expect("c1 saves");
            store
                .save_turn(
                    "c2", "Cooking", "chat", 0, "pasta recipe", "boil water", "[]", "[]", "done",
                    None, None, 2_000,
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
                    "c1", "t", "chat", 0, "alpha one", "", "[]", "[]", "done", None, None, 1_000,
                )
                .await
                .expect("turn 0");
            store
                .save_turn(
                    "c1", "t", "chat", 1, "alpha two", "", "[]", "[]", "done", None, None, 2_000,
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
                    "c1", "t", "chat", 0, "q", "", "[]", "[]", "done", None, None, 1_000,
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
                        id, "t", "chat", 0, "q", "", "[]", "[]", "done", None, None, 1_000,
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
    fn delete_older_than_ages_out_old_keeps_recent() {
        block_on(async {
            let store = test_store().await;
            // Old conversation (last activity before cutoff).
            store
                .save_turn(
                    "old", "Old", "chat", 0, "q", "", "[]", "[]", "done", None, None, 1_000,
                )
                .await
                .expect("old saves");
            // Recent conversation (last activity after cutoff).
            store
                .save_turn(
                    "recent", "Recent", "chat", 0, "q", "", "[]", "[]", "done", None, None,
                    10_000,
                )
                .await
                .expect("recent saves");

            let deleted = store
                .delete_conversations_older_than(5_000)
                .await
                .expect("delete older than");
            assert_eq!(deleted, 1);

            assert!(store.get_conversation("old").await.expect("get").is_none());
            assert!(store
                .get_conversation("recent")
                .await
                .expect("get")
                .is_some());
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
                    "conv-pin", "Pinned", "chat", 0, "q", "a", "[]", "[]", "done", None, None,
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
    fn unpinned_conversation_reads_no_engine() {
        block_on(async {
            let store = test_store().await;
            store
                .save_turn(
                    "plain", "Plain", "chat", 0, "q", "a", "[]", "[]", "done", None, None, 1_000,
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
}
