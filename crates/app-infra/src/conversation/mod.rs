//! Persistent conversation storage (issue #102, ADR 0031).
//!
//! ONE shared `ConversationStore` backs both doors — Quick Recall and Chat — so
//! conversations persist across app restarts in the Encrypted Capture Index.
//! Unlike the User Context dossier (which OUTLIVES retention), conversations
//! OBEY Retention Policy: they are aged out by the same local-calendar cutoff
//! capture cleanup uses (`delete_conversations_older_than`, called from
//! `capture_retention.rs`) and CLEARED by Wipe User Context (`wipe_all`).
//!
//! Timestamps are INTEGER unix milliseconds set from Rust (see migration
//! `0028_conversations.sql`), read/written as raw `i64` columns.

pub mod store;

pub use store::ConversationStore;
