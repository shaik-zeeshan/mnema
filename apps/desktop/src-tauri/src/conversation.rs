//! Tauri command surface for persistent conversations (issue #102, ADR 0031).
//!
//! ONE shared store (`app_infra::ConversationStore`) backs both doors — Quick
//! Recall and Chat — so conversations persist across app restarts in the
//! Encrypted Capture Index, OBEY Retention Policy (aged out by the same
//! local-calendar cutoff as captures), and are CLEARED by Wipe User Context.
//!
//! This module is the thin command adapter; the storage + retention/wipe policy
//! lives in `crates/app-infra/src/conversation`. The frontend (issues #110 /
//! #111) consumes these commands and the `conversation_changed` refresh event.

pub mod commands;
