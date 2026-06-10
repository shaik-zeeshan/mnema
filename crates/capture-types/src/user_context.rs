//! User Context DTOs (issue #88) — pure serde data shared across frontend,
//! Tauri, and native layers.
//!
//! These mirror the **Activity** (evidence) and **Conclusion** (distilled
//! belief) layers of the **User Context** dossier plus their supporting status,
//! token usage, and dismissal-state shapes. They carry no logic; the
//! deterministic policy lives in `crates/app-infra/src/user_context` and the
//! LLM orchestration lives in the Tauri layer.
//!
//! Conventions (matching the rest of `capture-types`): structs use
//! `#[serde(rename_all = "camelCase")]` and enums use
//! `#[serde(rename_all = "snake_case")]`. Timestamps are `i64` unix
//! milliseconds (serialized to camelCase `*AtMs`). Confidence is an `f64` in
//! `[0.0, 1.0]`, so any struct carrying one derives `PartialEq` but **not**
//! `Eq`.

use serde::{Deserialize, Serialize};

use crate::DerivationBudgetTier;

/// Fixed taxonomy of profession-neutral work modes (CONTEXT.md "Activity
/// Category", ADR 0032). Engine-tier; may be `None` on a tracer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityCategory {
    Creating,
    Communication,
    Meetings,
    Research,
    Learning,
    Organizing,
    Personal,
    Entertainment,
}

/// Per-Activity **Focus Classification** (issue #105): how focused the episode
/// was, driving the focus/distraction heatmap on the Overview. Fixed v1
/// taxonomy mapped to the design's deep / mid / distracted bands. Engine-tier;
/// may be `None` on a tracer or when the engine is unsure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FocusLevel {
    /// Sustained, single-thread deep work (the design's "deep" band).
    Deep,
    /// Some focus, but context-switching or interleaved (the design's "mid" band).
    Mixed,
    /// Scattered / interrupted / off-task (the design's "distracted" band).
    Distracted,
}

/// Visibility status of a [`Conclusion`] in the dossier. `faded` means the
/// Conclusion sits below the display floor but keeps its history.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConclusionStatus {
    Visible,
    Faded,
    Dismissed,
}

/// Whether a piece of evidence supports or contradicts a [`Conclusion`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStance {
    Support,
    Contradict,
}

/// A reference from an [`Activity`] back to a raw capture subject (a frame or an
/// audio segment). `subject_type` mirrors `processing_jobs` subject types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvidenceRef {
    /// `"frame"` | `"audio_segment"`.
    pub subject_type: String,
    pub subject_id: i64,
    pub captured_at_ms: Option<i64>,
}

/// A derived episode of what the user did and how (the evidence layer).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: i64,
    pub title: String,
    pub summary: String,
    /// Effective Activity Category: the user's correction if one exists, else
    /// the engine's label (issue #105/#108). `None` when neither is set.
    pub category: Option<ActivityCategory>,
    /// Effective Focus Classification: the user's correction if one exists, else
    /// the engine's label (issue #105/#108). `None` when neither is set.
    pub focus: Option<FocusLevel>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub created_at_ms: i64,
    #[serde(default)]
    pub evidence: Vec<ActivityEvidenceRef>,
}

/// A reference from a [`Conclusion`] to the [`Activity`] that is its evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConclusionEvidenceRef {
    pub activity_id: i64,
    pub stance: EvidenceStance,
    pub activity_title: Option<String>,
    pub activity_started_at_ms: Option<i64>,
}

/// A distilled, plain-language belief about the user, grounded in Activities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conclusion {
    pub id: i64,
    pub subject: String,
    pub statement: String,
    pub confidence: f64,
    pub status: ConclusionStatus,
    pub pinned: bool,
    pub formed_at_ms: i64,
    pub last_supported_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub evidence: Vec<ConclusionEvidenceRef>,
}

/// A single point on a [`Conclusion`]'s confidence-over-time line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfidenceSnapshot {
    pub confidence: f64,
    pub snapshot_at_ms: i64,
}

/// The Subject page: every [`Conclusion`] about a Subject plus its trajectories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubjectView {
    pub subject: String,
    pub conclusions: Vec<Conclusion>,
    pub trajectories: Vec<SubjectTrajectory>,
}

/// A single Conclusion's confidence trajectory for the Subject page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubjectTrajectory {
    pub conclusion_id: i64,
    pub statement: String,
    pub history: Vec<ConfidenceSnapshot>,
}

/// Aggregated token usage across derivation runs (estimated; rig-core's
/// extractor does not surface exact usage).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserContextTokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub run_count: i64,
}

/// The most recent completed Conclusion-distillation pass: when it ran, what it
/// upserted, and how many drafts each persist gate withheld (ungrounded /
/// guardrail / formation bar / resurface). Powers the settings readout's
/// "why is my dossier thin?" line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserContextDistillationSummary {
    pub at_ms: i64,
    pub conclusions_derived: i64,
    pub ungrounded: i64,
    pub guardrail_suppressed: i64,
    pub below_formation_bar: i64,
    pub resurface_blocked: i64,
}

/// Availability + progress readout for the User Context settings surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserContextStatus {
    pub engine_available: bool,
    /// Mirrors `ai_runtime` reason codes when the engine is unavailable.
    pub reason: Option<String>,
    pub activity_count: i64,
    pub conclusion_count: i64,
    pub last_derived_at_ms: Option<i64>,
    /// "building your understanding…" progress state while older windows remain.
    pub backfilling: bool,
    pub token_usage: UserContextTokenUsage,
    pub budget_tier: DerivationBudgetTier,
    /// `None` until the first Conclusion distillation completes.
    pub last_distillation: Option<UserContextDistillationSummary>,
}

/// The engine-written narrative lede for one Insights Overview range (the
/// **User Context Digest**, issue #89): 2–4 sentences of second-person prose
/// summarizing what the range's [`Activity`] episodes amount to. Generated
/// lazily by the Tauri layer (`get_user_context_digest`) and cached per
/// `(rangeKind, rangeStartMs)` keyed on a fingerprint of the in-range
/// Activities, so an unchanged range never re-bills the engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserContextDigest {
    /// `"day"` | `"week"` | `"month"`.
    pub range_kind: String,
    pub range_start_ms: i64,
    /// Exclusive: the digest covers `[rangeStartMs, rangeEndMs)`.
    pub range_end_ms: i64,
    pub narrative: String,
    /// Short generated title rendered in large type above the narrative
    /// (e.g. "A deep week in the editor"); `None` when generation produced no
    /// usable headline (narrative-only stays a valid digest).
    pub headline: Option<String>,
    pub generated_at_ms: i64,
}

/// A standing, user-authored Context statement (issue #107): something the user
/// asserted about themselves ("I'm a designer", "I care about X"), stored
/// verbatim. It is user-asserted rather than derived, so it carries no
/// confidence and never decays; it is fed to the Reasoning Engine alongside
/// derived User Context to steer derivation, survives Retention Policy aging and
/// the Delete Recent Capture cascade, and is cleared only by Wipe User Context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthoredContext {
    pub id: i64,
    pub text: String,
    /// Optional short grouping handle (mirrors a [`Conclusion`]'s Subject).
    pub topic: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Engine-carried state recording that the user rejected a particular
/// [`Conclusion`], with which evidence and when, fed as input to every
/// derivation pass so the engine can honor the high-bar-resurface rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DismissalState {
    pub subject: String,
    pub statement: String,
    /// Stable hash of the evidence activity-id set.
    pub evidence_fingerprint: String,
    pub evidence_activity_count: i64,
    pub dismissed_at_ms: i64,
}
