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

/// Fixed v1 taxonomy (CONTEXT.md "Activity Category"). Engine-tier; may be
/// `None` on a tracer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityCategory {
    Coding,
    Research,
    Communication,
    Design,
    Testing,
    Personal,
    Distractions,
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
    pub category: Option<ActivityCategory>,
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
