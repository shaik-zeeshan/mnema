//! Chat's view models: one [`ChatTurn`] per turn (the backend-owned
//! `TurnView` plus a few UI-only fields), the `TurnUpdate` reducer, and the
//! monotonic-version rule for `ask_ai_update` (docs/agents/ask-ai-streaming.md).
//! Nothing here parses answers or formats tool labels: the backend did that.

use capture_types::{
    AiProviderConfig, AnswerBlock, ConversationSummary, ConversationTurn, ToolActivityEntry,
    TurnSnapshot, TurnUpdate, TurnView,
};
use serde::Deserialize;
use serde_json::Value;

/// The `ask_ai_update` payload (`CoreEvent::AskAiUpdate`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAiUpdateEvent {
    pub conversation_id: String,
    pub version: u64,
    pub turn_index: i64,
    pub update: TurnUpdate,
}

/// `ask_ai_availability`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Availability {
    pub available: bool,
    pub reason: Option<String>,
}

/// One cited Answer Source; the wire carries opaque JSON, narrowed here like
/// `coerceSources` (anything that isn't a frame or audio is dropped).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub kind: String,
    pub frame_id: Option<i64>,
    pub audio_segment_id: Option<i64>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub ended_at: String,
    pub source_kind: Option<String>,
}

pub fn sources(value: &Value) -> Vec<Source> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value::<Source>(v.clone()).ok())
        .filter(|s| s.kind == "frame" || s.kind == "audio")
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatTurn {
    /// The render-ready view; the UI only reads it.
    pub view: TurnView,
    pub error_kind: Option<String>,
    /// Last applied `ask_ai_update` version (0 = hydrated, not live).
    pub version: u64,
    /// UI-only: when this session sent it and how long it took (steps line).
    pub started_at_ms: Option<i64>,
    pub elapsed_ms: Option<i64>,
    /// When the question was asked.
    pub at_ms: Option<i64>,
    /// The scope it was sent with ("this week").
    pub scope_label: Option<String>,
    /// An error caught before the backend saved a row (doesn't count toward turnIndex).
    pub local_only: bool,
}

impl ChatTurn {
    pub fn new(turn_index: i64, question: impl Into<String>, phase: &str) -> Self {
        Self {
            view: TurnView {
                turn_index,
                question: question.into(),
                phase: phase.into(),
                blocks: Vec::new(),
                reasoning: None,
                tool_activities: Vec::new(),
                live_activity: None,
                sources: Value::Array(Vec::new()),
                error_message: None,
                provider: None,
                model: None,
                context_tokens: None,
            },
            error_kind: None,
            version: 0,
            started_at_ms: None,
            elapsed_ms: None,
            at_ms: None,
            scope_label: None,
            local_only: false,
        }
    }

    /// A persisted turn. `get_conversation` fills `blocks` for every turn.
    pub fn hydrate(t: &ConversationTurn) -> Self {
        let mut turn = Self::new(t.turn_index, t.question.clone(), normalize_phase(&t.phase));
        turn.view.blocks = t.blocks.clone().unwrap_or_default();
        turn.view.reasoning = t.reasoning.clone();
        turn.view.tool_activities = coerce_tool_activities(&t.tool_activities);
        turn.view.sources = t.sources.clone();
        turn.view.error_message = t.error_message.clone();
        turn.view.provider = t.provider.clone();
        turn.view.model = t.model.clone();
        turn.error_kind = t.error_kind.clone();
        turn.at_ms = Some(t.created_at_ms);
        turn
    }

    /// Replace the render fields from a snapshot (`adoptView`).
    pub fn adopt(&mut self, snap: &TurnSnapshot) {
        let mut view = snap.view.clone();
        view.phase = normalize_phase(&view.phase).into();
        self.view = view;
        self.version = snap.version;
    }

    pub fn live(&self) -> bool {
        matches!(self.view.phase.as_str(), "thinking" | "streaming")
    }

    pub fn settled(&self) -> bool {
        !self.live()
    }

    /// Apply one op: a 1:1 mirror of the Rust `apply_update_to_view` (esp. the
    /// `AppendProse` coalescing), plus the UI-only error kind and elapsed time.
    /// shortcut: a port of src-tauri's reducer; share it once S3 moves ask_ai to app-core.
    pub fn apply(&mut self, update: &TurnUpdate, now_ms: i64) {
        let v = &mut self.view;
        match update {
            TurnUpdate::Phase { phase } => v.phase = normalize_phase(phase).into(),
            TurnUpdate::AppendProse { text } => match v.blocks.last_mut() {
                Some(AnswerBlock::Prose { markdown }) => markdown.push_str(text),
                _ => v.blocks.push(AnswerBlock::Prose {
                    markdown: text.clone(),
                }),
            },
            TurnUpdate::OpenBlock { block } => v.blocks.push(block.clone()),
            TurnUpdate::Reasoning { text } => match v.reasoning.as_mut() {
                Some(r) => r.push_str(text),
                None => v.reasoning = Some(text.clone()),
            },
            TurnUpdate::ToolActivity { entry } => v.tool_activities.push(entry.clone()),
            TurnUpdate::LiveActivity { entry } => v.live_activity = entry.clone(),
            TurnUpdate::Sources { sources } => v.sources = sources.clone(),
            TurnUpdate::ContextTokens { tokens } => v.context_tokens = Some(*tokens),
            TurnUpdate::Engine { provider, model } => {
                v.provider = Some(provider.clone());
                v.model = Some(model.clone());
            }
            TurnUpdate::Error { message, kind } => {
                v.error_message = Some(message.clone());
                v.phase = "error".into();
                self.error_kind = kind.clone();
            }
            TurnUpdate::Done => {
                v.phase = "done".into();
                v.live_activity = None;
                if let Some(start) = self.started_at_ms {
                    self.elapsed_ms = Some(now_ms - start);
                }
            }
        }
    }

    /// The copyable answer: the prose blocks' raw markdown.
    pub fn plain_text(&self) -> String {
        self.view
            .blocks
            .iter()
            .filter_map(|b| match b {
                AnswerBlock::Prose { markdown } => Some(markdown.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
            .trim()
            .to_string()
    }
}

/// What an incoming versioned update did to its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receive {
    /// `version == last + 1`: applied.
    Applied,
    /// `version <= last`: stale or duplicate, ignored.
    Stale,
    /// We missed updates: re-snapshot.
    Gap,
}

/// The versioned-update contract: only the exactly-next version applies.
pub fn receive(turn: &mut ChatTurn, version: u64, update: &TurnUpdate, now_ms: i64) -> Receive {
    if version == turn.version + 1 {
        turn.apply(update, now_ms);
        turn.version = version;
        Receive::Applied
    } else if version <= turn.version {
        Receive::Stale
    } else {
        Receive::Gap
    }
}

pub fn normalize_phase(phase: &str) -> &'static str {
    match phase {
        "thinking" => "thinking",
        "streaming" => "streaming",
        "error" => "error",
        _ => "done",
    }
}

/// Persisted tool activity is still the raw `{tool, params}` shape; map it to
/// render-ready entries (live and snapshot views already are).
pub fn coerce_tool_activities(value: &Value) -> Vec<ToolActivityEntry> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|e| {
            let e = e.as_object()?;
            if let (Some(Value::String(kind)), Some(Value::String(label))) =
                (e.get("kind"), e.get("label"))
            {
                return Some(ToolActivityEntry {
                    kind: kind.clone(),
                    label: label.clone(),
                    app: e.get("app").and_then(Value::as_str).map(str::to_owned),
                    app_icon_path: e
                        .get("appIconPath")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
            }
            let tool = e.get("tool").and_then(Value::as_str);
            let (kind, label) = match tool {
                Some("search") => ("search", "Searched your captures".to_string()),
                Some("timeline") => ("timeline", "Scanned timeline".to_string()),
                Some("show_text") => ("show_text", "Read a capture".to_string()),
                Some("recall_context") => (
                    "recall_context",
                    "Recalled what I know about you".to_string(),
                ),
                Some(t) => ("other", format!("Ran {t}")),
                None => ("other", "Working".to_string()),
            };
            Some(ToolActivityEntry {
                kind: kind.into(),
                label,
                app: None,
                app_icon_path: None,
            })
        })
        .collect()
}

/// The history list column's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum History {
    #[default]
    Loading,
    Error,
    Rows(Vec<ConversationSummary>),
}

/// The engine facts the composer, picker and scope panel show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Engine {
    pub providers: Vec<AiProviderConfig>,
    /// What "Default" runs in Chat: the Ask AI model on its provider, else the global default.
    pub default: Option<(String, String)>,
    pub favorites: Vec<(String, String)>,
    pub recents: Vec<(String, String)>,
    pub web_fetch: bool,
    pub connectors: usize,
}

/// Everything `data::load` answers for the surface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatData {
    pub history: History,
    /// None while availability loads.
    pub availability: Option<Availability>,
    pub engine: Engine,
}
