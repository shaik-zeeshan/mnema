//! `ConversationStore` — SQLite-backed storage for persistent Quick Recall /
//! Chat conversations (issue #102, ADR 0031).
//!
//! ONE shared store backs both doors. It owns the `0028_*` tables
//! (`conversations`, `conversation_turns`). Conversations OBEY Retention Policy
//! (aged out by the capture-cleanup pass in `capture_retention.rs`, driven by
//! `last_activity_at_ms`) and are CLEARED by Wipe User Context
//! ([`ConversationStore::wipe_all`]).
//!
//! Timestamps are INTEGER unix milliseconds; the caller stamps `now_ms` so the
//! store stays deterministic. `tool_activities` / `sources` are stored verbatim
//! as JSON text and parsed back into `serde_json::Value` on read.

use sqlx::{sqlite::SqliteRow, Row};

use capture_types::{AnswerBlock, Conversation, ConversationSummary, ConversationTurn};

use crate::db::CaptureDb;
use crate::Result;

/// Max characters of the first question kept as a history-list preview.
const PREVIEW_CHAR_CAP: usize = 140;

/// SQLite-backed storage for persistent conversations.
#[derive(Clone)]
pub struct ConversationStore {
    db: CaptureDb,
}

impl ConversationStore {
    pub(crate) fn new(db: CaptureDb) -> Self {
        Self { db }
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
        .execute(self.db.write())
        .await?;

        let row = sqlx::query("SELECT id FROM conversations WHERE conversation_id = ?1")
            .bind(conversation_id)
            .fetch_one(self.db.write())
            .await?;
        Ok(row.get("id"))
    }

    /// Upsert one turn of a conversation. The conversation row is ensured first
    /// (mirroring [`Self::upsert_conversation`], which also bumps its activity
    /// stamps), then the turn is inserted or — on conflict with an existing
    /// `(conversation_row_id, turn_index)` — updated in place.
    ///
    /// Both writes run in ONE transaction so a crash/error between them can
    /// never leave the conversation row activity-bumped without its turn (or an
    /// orphan turn against a half-written conversation row): either both land or
    /// neither does.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_turn(
        &self,
        conversation_id: &str,
        title: &str,
        origin: &str,
        turn_index: i64,
        question: &str,
        answer: &str,
        reasoning: Option<&str>,
        blocks: Option<&[AnswerBlock]>,
        tool_activities_json: &str,
        sources_json: &str,
        phase: &str,
        error_message: Option<&str>,
        seeded_result_count: Option<i64>,
        now_ms: i64,
    ) -> Result<()> {
        // Round-trip the parsed blocks as opaque JSON text (the store does NO
        // parsing): `Some(slice)` → a JSON array; `None` → SQL NULL (legacy).
        let blocks_json: Option<String> = match blocks {
            Some(slice) => Some(serde_json::to_string(slice)?),
            None => None,
        };

        let mut tx = self.db.begin_write().await?;

        // Ensure the conversation exists (and bump its activity stamps). Inlined
        // from `upsert_conversation` so it shares this transaction; the conflict
        // semantics (first non-empty title wins; pin/origin preserved) match.
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
        .execute(&mut *tx)
        .await?;

        let conversation_row_id: i64 =
            sqlx::query("SELECT id FROM conversations WHERE conversation_id = ?1")
                .bind(conversation_id)
                .fetch_one(&mut *tx)
                .await?
                .get("id");

        sqlx::query(
            "INSERT INTO conversation_turns \
                (conversation_row_id, turn_index, question, answer, reasoning, blocks, tool_activities, sources, \
                 phase, error_message, seeded_result_count, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12) \
             ON CONFLICT(conversation_row_id, turn_index) DO UPDATE SET \
                question = excluded.question, \
                answer = excluded.answer, \
                reasoning = excluded.reasoning, \
                blocks = excluded.blocks, \
                tool_activities = excluded.tool_activities, \
                sources = excluded.sources, \
                phase = excluded.phase, \
                error_message = excluded.error_message, \
                seeded_result_count = excluded.seeded_result_count, \
                updated_at_ms = excluded.updated_at_ms \
             WHERE conversation_turns.phase NOT IN ('done', 'error')",
        )
        .bind(conversation_row_id)
        .bind(turn_index)
        .bind(question)
        .bind(answer)
        .bind(reasoning)
        .bind(blocks_json)
        .bind(tool_activities_json)
        .bind(sources_json)
        .bind(phase)
        .bind(error_message)
        .bind(seeded_result_count)
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// List conversations newest-first (by `updated_at_ms`), each as a summary
    /// carrying its turn count + a short preview (first turn's question). The
    /// summary `title` is the EFFECTIVE title (see [`effective_title`]):
    /// user-set → generated → stored → preview truncation.
    pub async fn list_conversations(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ConversationSummary>> {
        let rows = sqlx::query(
            "SELECT c.conversation_id AS conversation_id, c.title AS title, \
                    c.user_title AS user_title, c.generated_title AS generated_title, \
                    c.origin AS origin, \
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
        .fetch_all(self.db.read())
        .await?;

        Ok(rows.into_iter().map(map_summary).collect())
    }

    /// Hydrate one conversation (with its turns in `turn_index` order) by its
    /// frontend UUID. `None` when absent. The hydrated `title` is the EFFECTIVE
    /// title (see [`effective_title`]).
    pub async fn get_conversation(&self, conversation_id: &str) -> Result<Option<Conversation>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, title, user_title, generated_title, origin, \
                    created_at_ms, updated_at_ms, provider, model \
             FROM conversations WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .fetch_optional(self.db.read())
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let row_id: i64 = row.get("id");
        let turns = self.list_turns(row_id).await?;
        let preview = turns
            .first()
            .map(|turn| truncate_preview(&turn.question))
            .unwrap_or_default();
        Ok(Some(Conversation {
            conversation_id: row.get("conversation_id"),
            title: effective_title(
                row.get("user_title"),
                row.get("generated_title"),
                row.get("title"),
                &preview,
            ),
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
        .execute(self.db.write())
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
        .fetch_optional(self.db.read())
        .await?;
        Ok(row.map(|row| (row.get("provider"), row.get("model"))))
    }

    /// Set the USER-SET title for a conversation (an explicit rename). Once set
    /// it wins forever: the read path prefers it over any generated title, and
    /// the generated-title writer ([`Self::set_generated_title_if_unset`]) is
    /// conditional on `user_title` still being NULL. Bumps `updated_at_ms` /
    /// `last_activity_at_ms` (a rename is user activity). Returns `false` when
    /// the conversation does not exist (a rename never creates a row). The
    /// caller passes a trimmed, non-empty title.
    pub async fn set_user_title(
        &self,
        conversation_id: &str,
        title: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE conversations SET \
                user_title = ?2, \
                updated_at_ms = ?3, last_activity_at_ms = ?3 \
             WHERE conversation_id = ?1",
        )
        .bind(conversation_id)
        .bind(title)
        .bind(now_ms)
        .execute(self.db.write())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Persist a model-GENERATED title, but only while the conversation is
    /// still eligible: the row exists AND has neither a user-set title (a
    /// rename — even one racing the in-flight generation — wins forever) nor an
    /// earlier generated title (generation is once-per-thread). The guard lives
    /// in the WHERE clause so the check-and-write is one atomic statement.
    /// Deliberately does NOT bump the activity stamps: this is a cosmetic
    /// background write, not user activity, so it never re-sorts the history
    /// list or extends retention. Returns whether the title was written.
    pub async fn set_generated_title_if_unset(
        &self,
        conversation_id: &str,
        title: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE conversations SET generated_title = ?2 \
             WHERE conversation_id = ?1 \
               AND user_title IS NULL \
               AND generated_title IS NULL",
        )
        .bind(conversation_id)
        .bind(title)
        .execute(self.db.write())
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_turns(&self, conversation_row_id: i64) -> Result<Vec<ConversationTurn>> {
        let rows = sqlx::query(
            "SELECT turn_index, question, answer, reasoning, blocks, tool_activities, sources, phase, \
                    error_message, seeded_result_count, created_at_ms, updated_at_ms \
             FROM conversation_turns \
             WHERE conversation_row_id = ?1 \
             ORDER BY turn_index ASC",
        )
        .bind(conversation_row_id)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows.into_iter().map(map_turn).collect())
    }

    /// Case-insensitive search across conversation titles (user-set, generated,
    /// and stored) and any turn's question/answer. Newest-first (by
    /// `updated_at_ms`), deduped per conversation, capped at `limit`.
    pub async fn search_conversations(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>> {
        // Escape LIKE wildcards in the user term so `%`/`_` match literally.
        let pattern = format!("%{}%", escape_like(query));
        let rows = sqlx::query(
            "SELECT c.conversation_id AS conversation_id, c.title AS title, \
                    c.user_title AS user_title, c.generated_title AS generated_title, \
                    c.origin AS origin, \
                    c.created_at_ms AS created_at_ms, c.updated_at_ms AS updated_at_ms, \
                    (SELECT COUNT(*) FROM conversation_turns t WHERE t.conversation_row_id = c.id) AS turn_count, \
                    (SELECT t.question FROM conversation_turns t \
                     WHERE t.conversation_row_id = c.id \
                     ORDER BY t.turn_index ASC LIMIT 1) AS preview \
             FROM conversations c \
             WHERE c.title LIKE ?1 ESCAPE '\\' COLLATE NOCASE \
                OR c.user_title LIKE ?1 ESCAPE '\\' COLLATE NOCASE \
                OR c.generated_title LIKE ?1 ESCAPE '\\' COLLATE NOCASE \
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
        .fetch_all(self.db.read())
        .await?;
        Ok(rows.into_iter().map(map_summary).collect())
    }

    /// Delete a conversation (its turns cascade via FK). A no-op when absent.
    pub async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM conversations WHERE conversation_id = ?1")
            .bind(conversation_id)
            .execute(self.db.write())
            .await?;
        Ok(())
    }

    /// **Wipe User Context** clears all conversations too: in ONE transaction,
    /// delete every turn then every conversation (children first so it is
    /// correct regardless of FK enforcement).
    pub async fn wipe_all(&self) -> Result<()> {
        let mut tx = self.db.begin_write().await?;
        sqlx::query("DELETE FROM conversation_turns")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

}

/// Parse a JSON column back into a `serde_json::Value`, falling back to JSON
/// `null` on a parse failure (a corrupt/legacy value never breaks hydration).
fn parse_json(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or(serde_json::Value::Null)
}

/// Resolve the EFFECTIVE display title for a conversation: the first non-blank
/// of user-set title → generated title → stored title (the legacy upsert title,
/// historically the frontend's first-question truncation) → the first-question
/// preview truncation. "User-set wins forever" is this ordering plus the
/// conditional generated-title write ([`ConversationStore::set_generated_title_if_unset`]).
fn effective_title(
    user_title: Option<String>,
    generated_title: Option<String>,
    stored_title: String,
    preview: &str,
) -> String {
    for candidate in [
        user_title.as_deref(),
        generated_title.as_deref(),
        Some(stored_title.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let candidate = candidate.trim();
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }
    preview.to_string()
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
    // `blocks` is the opaque round-tripped render model: SQL NULL → `None`
    // (a LEGACY turn the desktop layer parses from `answer` on read); a stored
    // JSON array → `Some(vec)`. A corrupt value tolerantly falls back to `None`
    // (mirroring `parse_json`), which the desktop layer then re-parses.
    let blocks: Option<String> = row.get("blocks");
    let blocks =
        blocks.and_then(|text| serde_json::from_str::<Vec<AnswerBlock>>(&text).ok());
    ConversationTurn {
        turn_index: row.get("turn_index"),
        question: row.get("question"),
        answer: row.get("answer"),
        reasoning: row.get("reasoning"),
        blocks,
        tool_activities: parse_json(&tool_activities),
        sources: parse_json(&sources),
        phase: row.get("phase"),
        error_message: row.get("error_message"),
        seeded_result_count: row.get("seeded_result_count"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    }
}

/// Map a history-list row onto a [`ConversationSummary`]. The summary `title`
/// is the EFFECTIVE title (see [`effective_title`]), so the frontend list can
/// render `title` directly.
fn map_summary(row: SqliteRow) -> ConversationSummary {
    let preview: Option<String> = row.get("preview");
    let preview = preview.as_deref().map(truncate_preview).unwrap_or_default();
    ConversationSummary {
        conversation_id: row.get("conversation_id"),
        title: effective_title(
            row.get("user_title"),
            row.get("generated_title"),
            row.get("title"),
            &preview,
        ),
        origin: row.get("origin"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
        turn_count: row.get("turn_count"),
        preview,
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
                model TEXT,
                generated_title TEXT,
                user_title TEXT
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
                    None,
                    None,
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
                    "done", None, None, 1_000,
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
                    "streaming", None, None, 1_000,
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
                    "conv-a", "t", "chat", 0, "q", "", None, None, "[]", "[]", "streaming", None, None,
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
                    "done", None, None, 1_000,
                )
                .await
                .expect("initial done turn");
            // A late update arrives for that same (conversation, index): the guard
            // must drop it — answer/phase stay finalized.
            store
                .save_turn(
                    "conv-done", "t", "chat", 0, "q", "LATE overwrite", None, None, "[]", "[]",
                    "streaming", None, None, 2_000,
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
                    Some("boom"), None, 1_000,
                )
                .await
                .expect("initial error turn");
            store
                .save_turn(
                    "conv-error", "t", "chat", 0, "q", "RECOVERED", None, None, "[]", "[]", "done",
                    None, None, 2_000,
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
                    None, None, 1_000,
                )
                .await
                .expect("initial streaming turn");
            store
                .save_turn(
                    "conv-live", "t", "chat", 0, "q", "final answer", None, None, "[]", "[]", "done",
                    None, None, 2_000,
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
                    "older", "Older", "chat", 0, "old question", "", None, None, "[]", "[]", "done", None,
                    None, 1_000,
                )
                .await
                .expect("older saves");
            store
                .save_turn(
                    "newer", "Newer", "chat", 0, "new question", "", None, None, "[]", "[]", "done", None,
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
                    None,
                    None,
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
                    "c2", "Cooking", "chat", 0, "pasta recipe", "boil water", None, None, "[]", "[]",
                    "done", None, None, 2_000,
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
                    "c1", "t", "chat", 0, "alpha one", "", None, None, "[]", "[]", "done", None, None,
                    1_000,
                )
                .await
                .expect("turn 0");
            store
                .save_turn(
                    "c1", "t", "chat", 1, "alpha two", "", None, None, "[]", "[]", "done", None, None,
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
                    "c1", "t", "chat", 0, "q", "", None, None, "[]", "[]", "done", None, None, 1_000,
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
                        id, "t", "chat", 0, "q", "", None, None, "[]", "[]", "done", None, None, 1_000,
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
                    "conv-pin", "Pinned", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None, None,
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
                    None, None, 1_000,
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
                    "[]", "[]", "done", None, None, 1_000,
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
                    "conv-o", "q", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None, None,
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
                    None, None, 1_000,
                )
                .await
                .expect("c1 saves");
            store
                .save_turn(
                    "c2", "", "chat", 0, "question two", "answer", None, None, "[]", "[]", "done",
                    None, None, 2_000,
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
                    "plain", "Plain", "chat", 0, "q", "a", None, None, "[]", "[]", "done", None, None,
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
}
