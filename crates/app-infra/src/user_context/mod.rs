//! User Context (issue #88) storage + capture-window reader.
//!
//! This module owns the **Encrypted Capture Index** storage for the User
//! Context dossier (Activities, their evidence, and the derivation-run ledger)
//! plus the capture-window reader that assembles already-redacted OCR /
//! transcript text for the derivation worker. It does **not** depend on
//! `ai-runtime`/`rig-core`: the LLM orchestration lives in the Tauri layer and
//! funnels its results back through these stores.
//!
//! The `confidence` submodule (#95) holds the pure, unit-tested Confidence Policy
//! math, and the `guardrail` submodule (#96) holds the pure Sensitive Category
//! Guardrail (the soft instruction text + the hard `is_sensitive` post-filter).

pub mod capture_source;
pub mod confidence;
pub mod guardrail;
pub mod store;
pub mod subject_vectors;

pub use capture_source::{CaptureWindow, CaptureWindowItem};
pub use subject_vectors::SubjectVectorStore;
pub use store::{
    cascade_derived_for_deleted_subjects_in, digest_input_fingerprint, evidence_fingerprint,
    ActivityCorrection, DistillationGateDrops, FailedDerivationWindow, NewActivity,
    NewActivityEvidence, NewConclusion, NewConclusionEvidence, NewDerivationRun, StoredDigest,
    UserContextCascadeSummary, UserContextStore,
};
