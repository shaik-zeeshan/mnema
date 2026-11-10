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
            query: String::new(),
            availability,
            engine: settings
                .as_ref()
                .map(super::data::engine)
                .unwrap_or_default(),
            settings,
        },
        open: get(&map, &["get_conversation"])?,
        snapshot: get(&map, &["ask_ai_snapshot"])?,
    })
}

/// What a test's sources answer, and every call they got (`FrameImages` never
/// reads these paths unless a golden points them at real files).
#[derive(Default)]
pub struct FixtureSources {
    /// `get_frame_scrub_previews`: id → path, `None` = no preview.
    pub thumbs: std::collections::HashMap<i64, Option<std::path::PathBuf>>,
    /// `get_frame` sizes; a frame missing here is gone.
    pub frames: std::collections::HashMap<i64, (Option<u32>, Option<u32>)>,
    /// `get_frame` fails with this.
    pub frame_error: Option<String>,
    pub previews: std::collections::HashMap<i64, std::path::PathBuf>,
    pub ocr: std::collections::HashMap<i64, Vec<crate::timeline::model::OcrBox>>,
    /// The receipt's strip over any span.
    pub strip: Vec<(i64, i64)>,
    /// `resolve_app_icons`: requested name → icon PNG.
    pub icons: std::collections::HashMap<String, std::path::PathBuf>,
    /// `ai_runtime_chatgpt_begin_login`: (code, url); `None` fails it.
    pub login: Option<(String, String)>,
    pub calls: std::sync::Mutex<Vec<String>>,
}

impl FixtureSources {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn log(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl super::source_data::SourceBackend for FixtureSources {
    fn thumbs(&self, ids: &[i64]) -> Vec<crate::quick_recall::model::Preview> {
        self.log(format!("thumbs {ids:?}"));
        ids.iter()
            .filter_map(|id| {
                let path = self.thumbs.get(id)?.clone();
                Some(crate::quick_recall::model::Preview {
                    frame_id: *id,
                    missing: path.is_none().then(|| "frameNotFound".into()),
                    path,
                })
            })
            .collect()
    }
    fn frame(&self, id: i64) -> super::source_data::Res<Option<(Option<u32>, Option<u32>)>> {
        self.log(format!("frame {id}"));
        match &self.frame_error {
            Some(e) => Err(e.clone()),
            None => Ok(self.frames.get(&id).copied()),
        }
    }
    fn preview(&self, id: i64) -> Option<std::path::PathBuf> {
        self.log(format!("preview {id}"));
        self.previews.get(&id).cloned()
    }
    fn ocr(&self, id: i64) -> super::source_data::Res<Option<Vec<crate::timeline::model::OcrBox>>> {
        self.log(format!("ocr {id}"));
        Ok(self.ocr.get(&id).cloned())
    }
    fn frames(&self, start: i64, end: i64) -> super::source_data::Res<Vec<(i64, i64)>> {
        self.log(format!("frames {start}..{end}"));
        Ok(self.strip.clone())
    }
    fn app_icons(&self, apps: &[String]) -> Vec<(String, std::path::PathBuf)> {
        self.log(format!("icons {apps:?}"));
        apps.iter()
            .filter_map(|a| Some((a.clone(), self.icons.get(a)?.clone())))
            .collect()
    }
    fn chatgpt_begin(&self, provider: &str) -> super::source_data::Res<(String, String)> {
        self.log(format!("chatgpt_begin {provider}"));
        self.login
            .clone()
            .ok_or_else(|| "OpenAI didn't answer.".into())
    }
    fn chatgpt_cancel(&self, provider: &str) {
        self.log(format!("chatgpt_cancel {provider}"));
    }
}
