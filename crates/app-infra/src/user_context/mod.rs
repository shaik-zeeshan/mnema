//! User Context (issue #88) storage + capture-window reader.
//!
//! This module owns the **Encrypted Capture Index** storage for the User
//! Context dossier (Activities, their evidence, and the derivation-run ledger)
//! plus the capture-window reader that assembles already-redacted OCR /
//! transcript text for the derivation worker. It does **not** depend on
//! `ai-runtime`/`rig-core`: the LLM orchestration lives in the Tauri layer and
//! funnels its results back through these stores.
//!
//! The `confidence` submodule (this slice, #95) holds the pure, unit-tested
//! Confidence Policy math. A later slice adds the `guardrail` submodule (#96).

pub mod capture_source;
pub mod confidence;
pub mod store;

pub use capture_source::{CaptureWindow, CaptureWindowItem};
pub use store::{
    NewActivity, NewActivityEvidence, NewConclusion, NewConclusionEvidence, NewDerivationRun,
    UserContextStore,
};
