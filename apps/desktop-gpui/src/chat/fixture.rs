//! Parity fixtures -> Chat. `apps/desktop/parity/fixtures/chat/<state>.json` is
//! what the Svelte stub serves (command -> JSON); the goldens read the same map
//! through the real `capture-types` structs, so both sides show one conversation.

use capture_types::{Conversation, ConversationSummary, RecordingSettings, TurnSnapshot};

use super::model::{Availability, ChatData, History};
use crate::shell::fixture::{get, load};

/// The Chat parity states (and golden names), in `surfaces.ts` order.
pub const STATES: [&str; 5] = ["empty", "list", "mid-stream", "done", "error"];

pub struct ChatFixture {
    pub data: ChatData,
    /// The open conversation (`get_conversation`), if the state opens one.
    pub open: Option<Conversation>,
    /// The live turn (`ask_ai_snapshot`) for a mid-stream state.
    pub snapshot: Option<TurnSnapshot>,
}

pub fn chat_fixture(state: &str) -> anyhow::Result<ChatFixture> {
    let map = load("chat", state)?;
    let rows: Vec<ConversationSummary> = get(&map, &["list_conversations"])?.unwrap_or_default();
    let availability: Option<Availability> = get(&map, &["ask_ai_availability"])?;
    let settings: Option<RecordingSettings> = get(&map, &["get_recording_settings"])?;
    Ok(ChatFixture {
        data: ChatData {
            history: History::Rows(rows),
            availability,
            engine: settings
                .as_ref()
                .map(super::data::engine)
                .unwrap_or_default(),
        },
        open: get(&map, &["get_conversation"])?,
        snapshot: get(&map, &["ask_ai_snapshot"])?,
    })
}
