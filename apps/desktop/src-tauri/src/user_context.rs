//! Tauri-layer orchestration for **User Context** derivation (CONTEXT.md / ADR
//! 0028–0031).
//!
//! `crates/app-infra` owns the encrypted storage and the deterministic policy;
//! `crates/ai-runtime` (aliased `ai_engine`) owns the LLM round trip. This module
//! is the glue that the Tauri app uniquely can provide: it reads the wire
//! [`AiRuntimeSettings`]/[`UserContextSettings`] out of `RecordingSettingsState`,
//! resolves an [`ai_engine::EngineConfig`] (sourcing the keychain key via
//! `crate::ai_runtime::resolve_engine_config`), reads a redacted capture window
//! through `UserContextStore`, asks the engine to segment it into semantic
//! **Activity** episodes, and persists the results.
//!
//! Privacy invariant: only the redacted OCR/transcript *text* exposed by
//! `UserContextStore::read_capture_window` ever leaves for a cloud engine — never
//! frame images or audio. The provider key is read from the OS keychain at call
//! time and is never logged.
//!
//! Submodules:
//! - [`derivation`] — prompt construction + the structured-extraction call.
//! - [`worker`] — the background, tier-paced derivation loop (OCR Catch-Up style).
//! - [`commands`] — the Tauri command surface (status / list / run-now).

pub mod commands;
pub mod derivation;
pub mod worker;
