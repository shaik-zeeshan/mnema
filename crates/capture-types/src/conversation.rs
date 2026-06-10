//! Persistent conversation DTOs (issue #102, ADR 0031) — pure serde data shared
//! across the Tauri layer and frontend.
//!
//! ONE shared conversation store backs both doors (Quick Recall and Chat). These
//! mirror the `conversations` / `conversation_turns` tables in the Encrypted
//! Capture Index. They carry no logic; storage + retention/wipe policy live in
//! `crates/app-infra/src/conversation`.
//!
//! Conventions (matching the rest of `capture-types`): structs use
//! `#[serde(rename_all = "camelCase")]`. Timestamps are `i64` unix milliseconds
//! (serialized to camelCase `*AtMs`). `tool_activities` / `sources` are opaque
//! JSON the frontend round-trips, so they are typed `serde_json::Value`.

use serde::{Deserialize, Serialize};

/// One question/answer turn within a [`Conversation`], in `turn_index` order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub turn_index: i64,
    pub question: String,
    pub answer: String,
    /// Opaque per-turn tool-activity log the frontend round-trips (JSON array).
    pub tool_activities: serde_json::Value,
    /// Opaque per-turn Answer Sources the frontend round-trips (JSON array).
    pub sources: serde_json::Value,
    /// `'streaming'` | `'done'` | `'error'` (frontend-owned phase string).
    pub phase: String,
    pub error_message: Option<String>,
    pub seeded_result_count: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A fully-hydrated persisted conversation: its metadata plus every turn in
/// `turn_index` order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    /// Frontend-generated UUID (the stable cross-restart identity).
    pub conversation_id: String,
    pub title: String,
    /// The door that created it: `'quick_recall'` | `'chat'`.
    pub origin: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The pinned engine provider id (e.g. `"anthropic"` | `"openai"`), or
    /// `None` when unpinned (use the global default engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The pinned model id within [`Self::provider`], or `None` when unpinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub turns: Vec<ConversationTurn>,
}

/// A lightweight conversation row for the history list: metadata plus a turn
/// count and a short preview (the first turn's question, truncated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub title: String,
    pub origin: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub turn_count: i64,
    pub preview: String,
}
