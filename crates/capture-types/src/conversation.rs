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
    /// The model's reasoning ("thinking") text for this turn, or `None` when the
    /// turn carried none (and for legacy turns predating the column).
    pub reasoning: Option<String>,
    /// Render-ready parsed answer blocks for this turn, or `None` for a LEGACY
    /// turn predating the column (whose blocks the backend parses from `answer`
    /// on read). Serializes as `null` when absent (always present on the wire).
    pub blocks: Option<Vec<AnswerBlock>>,
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

// ── Render-ready chat view model (issue #110, Slice 1) ───────────────────────
// These are the BACKEND-OWNED render model for a streaming Ask AI turn. The
// backend decides what a turn looks like (its phases, answer blocks, reasoning,
// tool activity); the frontend only RENDERS them. This replaces the old scheme
// where the frontend re-parsed raw markdown fences (`mnema-bars` etc.) into chart
// data — that parse now lives in the backend, which emits typed [`AnswerBlock`]s.
//
// These types carry no behaviour. The TS mirror in
// `apps/desktop/src/lib/insights/conversation.ts` must agree field-for-field;
// there is no codegen, so the round-trip tests below pin the exact wire shapes.

/// One row of a horizontal-bar answer block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BarsItem {
    pub label: String,
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
}

/// One claim/finding in a dossier answer block. `confidence` is a 0..=1 score.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DossierItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub statement: String,
    pub confidence: f64,
}

/// One interval in a timeline answer block. `start`/`end` are RFC3339-ish
/// strings the frontend formats; `end` absent means an open/instant interval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub label: String,
    pub start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// One render-ready block of a turn's answer, internally tagged on `kind`.
///
/// `Prose` carries RAW markdown — the markdown→HTML pass stays on the frontend
/// (in `AnswerProse`). The graphical variants (`Bars`/`Dossier`/`Timeline`)
/// carry already-parsed data, so the frontend no longer re-parses fenced JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AnswerBlock {
    /// Raw markdown prose: `{ "kind": "prose", "markdown": "..." }`.
    Prose { markdown: String },
    /// A horizontal-bar chart block.
    Bars {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        items: Vec<BarsItem>,
    },
    /// A dossier of claims/findings with confidence scores.
    Dossier { items: Vec<DossierItem> },
    /// A timeline of labelled intervals.
    Timeline {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        items: Vec<TimelineItem>,
    },
}

/// One recorded brokered tool call rendered in the turn's activity rail.
///
/// `kind` is a frontend-known union string (`search` | `timeline` | `show_text`
/// | `other`), kept as a `String` to mirror the TS `AskToolKind` without forcing
/// the backend to model it as an enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityEntry {
    pub kind: String,
    pub label: String,
    /// The app the call was scoped to (bundle id or display name), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// A resolved icon path for [`Self::app`], when the backend could find one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_icon_path: Option<String>,
}

/// The full render-ready view of ONE Ask AI turn. The backend owns every field;
/// the frontend only renders. `phase` is the lifecycle string
/// (`"seeding" | "thinking" | "streaming" | "done" | "error"`). `sources` is the
/// same opaque Answer-Sources JSON the frontend round-trips on a persisted turn.
///
/// Unlike the item/block option fields (which are SKIPPED when absent), this
/// view's nullable fields always serialize — as JSON `null` when empty — so the
/// frontend can mirror them as `T | null` rather than optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub turn_index: i64,
    pub question: String,
    pub phase: String,
    pub blocks: Vec<AnswerBlock>,
    pub reasoning: Option<String>,
    pub tool_activities: Vec<ToolActivityEntry>,
    pub live_activity: Option<ToolActivityEntry>,
    /// Opaque Answer Sources JSON the frontend round-trips (a JSON array).
    pub sources: serde_json::Value,
    pub error_message: Option<String>,
    pub seeded_result_count: Option<i64>,
}

/// A versioned snapshot of a turn's [`TurnView`] for a given conversation. The
/// `version` lets the frontend ignore stale snapshots that race with live
/// [`TurnUpdate`]s.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnSnapshot {
    pub conversation_id: String,
    pub version: u64,
    pub view: TurnView,
}

/// One incremental mutation to a [`TurnView`], internally tagged on `op`. The
/// backend emits a stream of these as a turn streams; the frontend applies them
/// to its local view. (Slice 4 wires the emit path; these are types only.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum TurnUpdate {
    /// Advance the turn's lifecycle phase.
    Phase { phase: String },
    /// Append streamed text to the current open prose block.
    AppendProse { text: String },
    /// Open a new answer block (closing any current prose block).
    OpenBlock { block: AnswerBlock },
    /// Append streamed reasoning ("thinking") text.
    Reasoning { text: String },
    /// Record a completed tool call in the activity rail.
    ToolActivity { entry: ToolActivityEntry },
    /// Set or CLEAR the transient "live" activity line. `entry: null` clears it,
    /// so the `entry` key always serializes (never skipped).
    LiveActivity { entry: Option<ToolActivityEntry> },
    /// Replace the turn's Answer Sources with new opaque JSON.
    Sources { sources: serde_json::Value },
    /// Fail the turn with a message.
    Error { message: String },
    /// Mark the turn complete.
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answer_block_prose_exact_shape() {
        let block = AnswerBlock::Prose {
            markdown: "hi".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&block).unwrap(),
            json!({ "kind": "prose", "markdown": "hi" })
        );
    }

    #[test]
    fn answer_block_bars_exact_shape() {
        let block = AnswerBlock::Bars {
            title: Some("Top apps".to_string()),
            items: vec![BarsItem {
                label: "Editor".to_string(),
                value: 42.0,
                sublabel: Some("2h".to_string()),
            }],
        };
        assert_eq!(
            serde_json::to_value(&block).unwrap(),
            json!({
                "kind": "bars",
                "title": "Top apps",
                "items": [
                    { "label": "Editor", "value": 42.0, "sublabel": "2h" }
                ]
            })
        );
    }

    #[test]
    fn answer_block_bars_skips_absent_options() {
        // No title; item without a sublabel — both keys should be absent.
        let block = AnswerBlock::Bars {
            title: None,
            items: vec![BarsItem {
                label: "Editor".to_string(),
                value: 1.5,
                sublabel: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&block).unwrap(),
            json!({
                "kind": "bars",
                "items": [ { "label": "Editor", "value": 1.5 } ]
            })
        );
    }

    #[test]
    fn answer_block_dossier_and_timeline_shapes() {
        let dossier = AnswerBlock::Dossier {
            items: vec![DossierItem {
                subject: None,
                statement: "ships fast".to_string(),
                confidence: 0.5,
            }],
        };
        assert_eq!(
            serde_json::to_value(&dossier).unwrap(),
            json!({
                "kind": "dossier",
                "items": [ { "statement": "ships fast", "confidence": 0.5 } ]
            })
        );

        let timeline = AnswerBlock::Timeline {
            title: None,
            items: vec![TimelineItem {
                label: "Coding".to_string(),
                start: "2026-06-13T10:00:00Z".to_string(),
                end: None,
                app: None,
                category: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&timeline).unwrap(),
            json!({
                "kind": "timeline",
                "items": [
                    { "label": "Coding", "start": "2026-06-13T10:00:00Z" }
                ]
            })
        );
    }

    #[test]
    fn turn_update_exact_shapes() {
        assert_eq!(
            serde_json::to_value(TurnUpdate::AppendProse {
                text: "abc".to_string()
            })
            .unwrap(),
            json!({ "op": "appendProse", "text": "abc" })
        );

        assert_eq!(
            serde_json::to_value(TurnUpdate::Done).unwrap(),
            json!({ "op": "done" })
        );

        // None must serialize the `entry` key as null (clear the live line).
        assert_eq!(
            serde_json::to_value(TurnUpdate::LiveActivity { entry: None }).unwrap(),
            json!({ "op": "liveActivity", "entry": null })
        );

        assert_eq!(
            serde_json::to_value(TurnUpdate::Phase {
                phase: "streaming".to_string()
            })
            .unwrap(),
            json!({ "op": "phase", "phase": "streaming" })
        );
    }

    #[test]
    fn turn_view_serializes_camel_case_keys() {
        let view = TurnView {
            turn_index: 3,
            question: "what did I do?".to_string(),
            phase: "done".to_string(),
            blocks: vec![AnswerBlock::Prose {
                markdown: "stuff".to_string(),
            }],
            reasoning: None,
            tool_activities: vec![ToolActivityEntry {
                kind: "search".to_string(),
                label: "Searched".to_string(),
                app: None,
                app_icon_path: None,
            }],
            live_activity: None,
            sources: json!([]),
            error_message: None,
            seeded_result_count: None,
        };
        let value = serde_json::to_value(&view).unwrap();
        let obj = value.as_object().unwrap();
        // Nullable view fields serialize (as null), not skipped.
        assert!(obj.contains_key("turnIndex"));
        assert!(obj.contains_key("toolActivities"));
        assert!(obj.contains_key("liveActivity"));
        assert!(obj.contains_key("errorMessage"));
        assert!(obj.contains_key("seededResultCount"));
        assert_eq!(obj["liveActivity"], json!(null));
        assert_eq!(obj["errorMessage"], json!(null));
        assert_eq!(obj["seededResultCount"], json!(null));
        // The tool-activity entry omits its absent app/appIconPath options.
        assert_eq!(
            obj["toolActivities"],
            json!([ { "kind": "search", "label": "Searched" } ])
        );
    }

    #[test]
    fn turn_snapshot_round_trips_and_shape() {
        let snapshot = TurnSnapshot {
            conversation_id: "conv-1".to_string(),
            version: 7,
            view: TurnView {
                turn_index: 0,
                question: "q".to_string(),
                phase: "thinking".to_string(),
                blocks: vec![],
                reasoning: Some("hmm".to_string()),
                tool_activities: vec![],
                live_activity: Some(ToolActivityEntry {
                    kind: "timeline".to_string(),
                    label: "Building timeline".to_string(),
                    app: Some("com.example.app".to_string()),
                    app_icon_path: Some("/tmp/icon.png".to_string()),
                }),
                sources: json!([{ "kind": "frame" }]),
                error_message: Some("boom".to_string()),
                seeded_result_count: Some(5),
            },
        };

        let value = serde_json::to_value(&snapshot).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("conversationId"));
        assert!(obj.contains_key("version"));
        assert!(obj.contains_key("view"));

        let round_tripped: TurnSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, snapshot);
    }
}
