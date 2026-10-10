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
                 phase, error_message, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
             ON CONFLICT(conversation_row_id, turn_index) DO UPDATE SET \
                question = excluded.question, \
                answer = excluded.answer, \
                reasoning = excluded.reasoning, \
                blocks = excluded.blocks, \
                tool_activities = excluded.tool_activities, \
                sources = excluded.sources, \
                phase = excluded.phase, \
                error_message = excluded.error_message, \
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
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// List conversations pinned-first, then newest-first (by `updated_at_ms`),
    /// each as a summary carrying its turn count + a short preview (first turn's question). The
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
                    c.origin AS origin, c.pinned AS pinned, \
                    c.created_at_ms AS created_at_ms, c.updated_at_ms AS updated_at_ms, \
                    (SELECT COUNT(*) FROM conversation_turns t WHERE t.conversation_row_id = c.id) AS turn_count, \
                    (SELECT t.question FROM conversation_turns t \
                     WHERE t.conversation_row_id = c.id \
                     ORDER BY t.turn_index ASC LIMIT 1) AS preview \
             FROM conversations c \
             ORDER BY c.pinned DESC, c.updated_at_ms DESC, c.id DESC \
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

    /// Pin (or unpin) a chat in the history list. Deliberately touches NO
    /// timestamp: pinning is not activity, so it never re-sorts the chat into
    /// "Today" or extends its retention. Returns `false` when the conversation
    /// does not exist (a pin never creates a row).
    pub async fn set_pinned(&self, conversation_id: &str, pinned: bool) -> Result<bool> {
        let result = sqlx::query("UPDATE conversations SET pinned = ?2 WHERE conversation_id = ?1")
            .bind(conversation_id)
            .bind(pinned)
            .execute(self.db.write())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_turns(&self, conversation_row_id: i64) -> Result<Vec<ConversationTurn>> {
        let rows = sqlx::query(
            "SELECT turn_index, question, answer, reasoning, blocks, tool_activities, sources, phase, \
                    error_message, created_at_ms, updated_at_ms \
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
    /// and stored) and any turn's question/answer. Pinned-first, then newest-first
    /// (by `updated_at_ms`), deduped per conversation, capped at `limit`.
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
                    c.origin AS origin, c.pinned AS pinned, \
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
             ORDER BY c.pinned DESC, c.updated_at_ms DESC, c.id DESC \
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
        pinned: row.get::<i64, _>("pinned") != 0,
    }
}

#[cfg(test)]
mod tests;
