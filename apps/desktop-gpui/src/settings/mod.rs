//! The Settings surface (`routes/settings/+page.svelte` + `lib/settings/*`): a
//! full window with no titlebar or status bar. The rail (`rail.rs`), the bar
//! (`bar.rs`) and one scrolling column of the active group's sections
//! (`view.rs`, `sections/*`). Edits change drafts; `autosave.rs` debounces them
//! per domain and saves through `data::Backend` off the main thread.

mod actions;
pub mod autosave;
mod bar;
mod commands;
pub mod data;
#[cfg(feature = "fixtures")]
pub mod fixture;
pub mod groups;
pub mod live;
pub mod model;
pub mod privacy_prompt;
mod rail;
pub mod reload;
pub mod row;
mod saving;
pub(crate) mod sections;
#[cfg(test)]
mod tests;
mod view;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use capture_types::{MicrophoneControllerState, RecordingSettings, ScreenResolution};
use gpui_kit::base::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClickEvent, Context, Entity, ScrollHandle, Subscription, Task, Window};
use serde_json::Value;

use crate::shell::model::StatusBarModel;
use autosave::{Engine, Unit};
pub use commands::on_event;
use data::Backend;
use groups::{GroupId, SectionId};
pub use model::SettingsData;
pub use saving::domain_value;
pub use view::SettingsEvent;

/// Lucide paths the sections use (served by `assets::MnemaAssets`).
pub mod icons {
    pub const ALERT: &str = "icons/triangle-alert.svg";
    pub const INFO: &str = "icons/info.svg";
    pub const CHECK: &str = "icons/check.svg";
    pub const RETRY: &str = "icons/rotate-cw.svg";
    pub const RELOAD: &str = "icons/rotate-ccw.svg";
    pub const LOCK: &str = "icons/lock.svg";
    pub const EXTERNAL: &str = "icons/external-link.svg";
    pub const PLUS: &str = "icons/plus.svg";
    pub const X: &str = "icons/x.svg";
    pub const FOLDER: &str = "icons/folder.svg";
    pub const DOWNLOAD: &str = "icons/download.svg";
    pub const TRASH: &str = "icons/trash.svg";
    pub const CLOUD: &str = "icons/cloud.svg";
    pub const CLOUD_OFF: &str = "icons/cloud-off.svg";
    pub const CHIP: &str = "icons/cpu.svg";
    pub const OFF: &str = "icons/ban.svg";
    pub const WAVE: &str = "icons/audio-waveform.svg";
    pub const FEATHER: &str = "icons/feather.svg";
    pub const SHIELD: &str = "icons/shield.svg";
    pub const LINK: &str = "icons/link.svg";
    pub const MIC: &str = "icons/mic.svg";
    pub const PLUG: &str = "icons/plug.svg";
    pub const SPARKLES: &str = "icons/sparkles.svg";
    pub const TERMINAL: &str = "icons/square-terminal.svg";
    pub const SPEECH: &str = "icons/speech.svg";
    pub const LOADER: &str = "icons/loader-circle.svg";
}

/// The Delete Recent Capture windows (`DELETE_WINDOW_SECONDS`).
pub const DELETE_WINDOWS: [u64; 3] = [60, 300, 900];

/// View-only state the sections share.
#[derive(Default)]
pub struct Ui {
    /// The provider whose Manage fields are open.
    pub provider_open: Option<String>,
    pub add_provider_open: bool,
    /// Per-provider fields, key / sign-in / removal state and the runtime test.
    pub(crate) pv: sections::provider_row::PvUi,
    pub mcp_picker_open: bool,
    /// The local reads (model statuses among them) are in flight: a missing
    /// status reads "Checking installed … models…", not "unavailable".
    pub models_loading: bool,
    /// A model row's own outcome line, by family: `(ok, words)` ("Deleted …",
    /// "Speaker model action failed: …"); the next action replaces it.
    pub model_notes: HashMap<data::Family, (bool, String)>,
    /// "Delete unused models": in flight, and the line it left, by family.
    pub unused_busy: HashSet<data::Family>,
    pub unused_notes: HashMap<data::Family, (bool, String)>,
    /// Semantic Search after a confirmed switch: the re-index line it keeps.
    pub reindex: Option<String>,
    /// Which Deepgram placeholder the key field shows (`Some(key saved)`).
    pub deepgram_placeholder: Option<bool>,
    pub delete_window: usize,
    /// The shortcut row listening for keys.
    pub kb_listening: Option<&'static str>,
    /// A key the listening row refused: `(action id, why)`.
    pub kb_capture_error: Option<(&'static str, String)>,
    /// Where the first conflicting shortcut row was last painted ("Jump to conflict").
    pub kb_conflict_row: std::rc::Rc<std::cell::Cell<Option<gpui_kit::Bounds<gpui_kit::Pixels>>>>,
    pub semantic_pick: Option<String>,
    /// Each provider's live `verify_ai_provider` verdict (absent = never checked).
    pub reach: HashMap<String, Reach>,
    /// The session's one automatic reachability pass has run.
    pub reach_checked: bool,
    /// Model rows whose "Model files" disclosure is open.
    pub files_open: HashSet<&'static str>,
    /// The last one-shot command's outcome: `(ok, what, the backend's words)`.
    pub note: Option<(bool, &'static str, String)>,
    /// One-shot commands in flight, by name (their buttons read busy).
    pub busy: HashSet<&'static str>,
    /// "Add app…": the installed apps to pick from (`None` = closed).
    /// `(bundle id, name, running)` per installed app.
    pub app_picker: Option<Vec<(String, String, bool)>>,
    /// The MCP modals: Add / Edit / Connect steps, tool curation, Node probe.
    pub(crate) mcp: sections::mcp::McpUi,
    /// Model downloads in flight, by `provider:model`: `(done, total, status)`.
    pub downloads: HashMap<String, (u64, Option<u64>, String)>,
    pub prompt: privacy_prompt::PromptUi,
    /// Esc asked to empty the search field (done at the next render).
    pub clear_search: bool,
    /// The permission row whose request is in flight, and the last refusal.
    pub perm_busy: Option<&'static str>,
    pub perm_error: Option<(&'static str, String)>,
    /// The "No system sound yet" hint was dismissed (recorded, then hidden).
    pub sysaudio_dismissed: bool,
    pub sysaudio_error: Option<String>,
    /// The Firefox / Zen Accessibility request's last failure.
    pub gecko_error: Option<String>,
    /// Storage: a folder picked this session (it applies after a restart).
    pub picked_location: Option<String>,
    pub restarting: bool,
    /// The latest retention cleanup: `(segments, frames, audio)` or why it failed.
    pub cleanup: Option<Result<(i64, i64, i64), String>>,
    /// A confirmed shorter retention runs the cleanup once its save lands.
    pub cleanup_after_save: bool,
    /// CLI Access: rows whose block / unblock is in flight, the last failure,
    /// and the CLI install's failure.
    pub grant_saving: HashSet<String>,
    pub grant_error: Option<String>,
    pub cli_error: Option<String>,
    /// Licence: Refresh's transient line (`true` = checked), its generation,
    /// Activate's outcome, and "Free up my devices"' line.
    pub lic_refresh: Option<bool>,
    pub lic_refresh_gen: u64,
    pub lic_activate: Option<Result<(), String>>,
    pub lic_reset: Option<(crate::ui::Tone, String)>,
    /// A successful activation empties the key field (at the next render).
    pub clear_license_key: bool,
    /// "Add app…" reopens with an empty search (at the next render).
    pub clear_app_search: bool,
    /// The "Add app…" list's highlighted row (↑/↓, Enter picks it).
    pub app_highlight: usize,
    /// Where "Add app…" was last painted (its dropdown hangs off it).
    pub app_anchor: Option<crate::ui::popover::Anchor>,
    /// Developer › log: the last open / delete failure; "✓ Deleted" for 2.2 s.
    pub log_error: Option<String>,
    pub log_deleted: bool,
    /// About › Updates: Restart to Update's failure.
    pub update_error: Option<String>,
    /// Delete Recent Capture's receipt under its row: `(tone, words)`.
    pub delete_receipt: Option<(crate::ui::Tone, String)>,
}

/// One provider row's reachability check.
#[derive(Clone, Debug, PartialEq)]
pub enum Reach {
    Checking,
    Reachable,
    /// `classify_listing_failure`'s short reason.
    Failed(String),
}

/// Text fields and steppers (their raw strings parse into the drafts).
pub struct Inputs {
    pub search: Entity<InputState>,
    pub res_w: Entity<InputState>,
    pub res_h: Entity<InputState>,
    pub mbps: Entity<InputState>,
    pub tool_calls: Entity<InputState>,
    pub idle_unload: Entity<InputState>,
    pub chunk: Entity<InputState>,
    pub whitelist: Entity<InputState>,
    pub deepgram_key: Entity<InputState>,
    pub license_key: Entity<InputState>,
    /// "Add app…": the installed-app search.
    pub app_search: Entity<InputState>,
}

pub struct Sliders {
    pub mic: Entity<SliderState>,
    pub sys: Entity<SliderState>,
    pub speaker_timeout: Entity<SliderState>,
}

pub struct Settings {
    pub data: SettingsData,
    /// What the controls show; `saved` is the last persisted settings.
    pub draft: Option<RecordingSettings>,
    saved: Option<RecordingSettings>,
    pub kb_draft: Option<Value>,
    kb_saved: Option<Value>,
    pub mic_draft: Option<MicrophoneControllerState>,
    mic_saved: Option<MicrophoneControllerState>,
    pub group: GroupId,
    pub section: SectionId,
    /// A rail click / deeplink owns `section` until the user scrolls.
    pub spy_suppressed: bool,
    pub scroll: ScrollHandle,
    /// The column's section blocks in order, after `item_base` other children.
    items: Vec<SectionId>,
    item_base: usize,
    /// A scroll asked for before the column was laid out.
    pending_scroll: Option<SectionId>,
    /// Loaded drafts not yet pushed into the text fields / sliders.
    needs_sync: bool,
    /// The model pickers' listed models (`ai_runtime_list_models`).
    pub pool: Vec<crate::ui::model_picker::PoolModel>,
    pub pool_loading: bool,
    pub engine: Engine,
    pub saving: HashSet<Unit>,
    pub failed: Option<(Unit, String)>,
    /// Recording-domain failures in a row, and the pending automatic retry.
    fail_count: u32,
    retry_task: Option<Task<()>>,
    /// A privacy command holds the recording settings: no unit may save.
    pub privacy_in_flight: bool,
    pub privacy_error: Option<String>,
    /// Custom resolution / bitrate text that doesn't parse (blocks Video).
    pub video_invalid: bool,
    pub reload: reload::Reloads,
    pub ui: Ui,
    pub inputs: Inputs,
    pub sliders: Sliders,
    /// The footer's recording state (the shell's status model).
    pub status: StatusBarModel,
    /// The listening shortcut chord takes focus, so its keys reach `capture_key`.
    pub kb_focus: gpui_kit::FocusHandle,
    /// The shell shows Settings (false once it navigates away).
    shown: bool,
    /// CLI Access's 30 s re-read while the Data group is on screen.
    access_poll: Option<Task<()>>,
    backend: Arc<dyn Backend>,
    timer: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn input(window: &mut Window, cx: &mut App, placeholder: &str) -> Entity<InputState> {
    let ph = placeholder.to_string();
    cx.new(|cx| InputState::new(window, cx).placeholder(ph))
}

fn slider(window: &mut Window, cx: &mut App, min: f32, max: f32) -> Entity<SliderState> {
    let _ = window;
    cx.new(|_| {
        SliderState::new()
            .min(min)
            .max(max)
            .step(1.)
            .default_value(min)
    })
}

impl Settings {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let backend: Arc<dyn Backend> = match crate::chat::core(cx) {
            Some(core) => Arc::new(data::CoreBackend(core)),
            None => Arc::new(data::NoBackend),
        };
        Self::with_backend(backend, window, cx)
    }

    pub fn with_backend(
        backend: Arc<dyn Backend>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = Inputs {
            search: input(window, cx, "Search settings"),
            res_w: input(window, cx, "e.g. 1920"),
            res_h: input(window, cx, "e.g. 1080"),
            mbps: input(window, cx, "e.g. 12"),
            tool_calls: input(window, cx, ""),
            idle_unload: input(window, cx, ""),
            chunk: input(window, cx, ""),
            whitelist: input(window, cx, "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"),
            deepgram_key: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Paste your Deepgram API key")
            }),
            license_key: input(window, cx, "Paste your license key"),
            app_search: input(window, cx, "Search installed apps"),
        };
        let sliders = Sliders {
            mic: slider(window, cx, 0., 100.),
            sys: slider(window, cx, 0., 100.),
            speaker_timeout: slider(window, cx, 1., 60.),
        };
        let mut subs = vec![cx.subscribe_in(
            &inputs.search,
            window,
            |this, state, e: &InputEvent, _, cx| {
                match e {
                    InputEvent::Change => {
                        this.spy_suppressed = false;
                        cx.notify();
                    }
                    // Enter jumps to the first match; the query stays to refine.
                    InputEvent::PressEnter { .. } => {
                        let query = state.read(cx).value().trim().to_string();
                        let first = groups::filter(&query)
                            .into_iter()
                            .find_map(|(_, sections)| sections.first().copied());
                        if let (false, Some(section)) = (query.is_empty(), first) {
                            this.navigate(section, cx);
                        }
                    }
                    _ => {}
                }
            },
        )];
        subs.push(cx.subscribe_in(
            &inputs.app_search,
            window,
            |this, state, e: &InputEvent, _, cx| match e {
                InputEvent::Change => {
                    this.ui.app_highlight = 0;
                    cx.notify()
                }
                // Enter picks the highlighted match.
                InputEvent::PressEnter { .. } => {
                    let query = state.read(cx).value().to_string();
                    if let Some((bundle, name)) = commands::highlighted_app(this, &query) {
                        commands::exclude_app(this, bundle, name, cx);
                    }
                }
                _ => {}
            },
        ));
        subs.push(cx.subscribe_in(
            &inputs.license_key,
            window,
            |this, _, e: &InputEvent, _, cx| match e {
                // Editing the key clears both the error and the confirmation.
                InputEvent::Change => {
                    if this.ui.lic_activate.take().is_some() {
                        cx.notify();
                    }
                }
                InputEvent::PressEnter { .. } => this.activate_license(cx),
                _ => {}
            },
        ));
        let numeric: [(
            &Entity<InputState>,
            fn(&mut RecordingSettings, &str) -> bool,
        ); 6] = [
            (&inputs.res_w, |d, raw| set_custom_res(d, Some(raw), None)),
            (&inputs.res_h, |d, raw| set_custom_res(d, None, Some(raw))),
            (&inputs.mbps, |d, raw| {
                let v = raw.trim().parse::<u32>().ok().filter(|v| *v > 0);
                d.video_bitrate.custom_mbps = v;
                v.is_some()
            }),
            (&inputs.tool_calls, |d, raw| {
                d.access.ask_ai_max_tool_calls = raw.trim().parse().unwrap_or(12);
                true
            }),
            (&inputs.idle_unload, |d, raw| {
                d.transcription.idle_unload_seconds = raw.trim().parse().unwrap_or(0);
                true
            }),
            (&inputs.chunk, |d, raw| {
                d.transcription.chunk_seconds = raw.trim().parse().unwrap_or(0);
                true
            }),
        ];
        for (state, apply) in numeric {
            subs.push(
                cx.subscribe_in(state, window, move |this, state, e: &InputEvent, _, cx| {
                    if matches!(e, InputEvent::Change) {
                        let raw = state.read(cx).value().to_string();
                        this.edit(
                            |d| {
                                apply(d, &raw);
                            },
                            cx,
                        );
                        this.video_invalid = sections::video_invalid(this, cx);
                        this.tick(cx);
                    }
                }),
            );
        }
        let slides: [(&Entity<SliderState>, fn(&mut RecordingSettings, f32)); 3] = [
            (&sliders.mic, |d, v| {
                d.microphone_activity_sensitivity = v as u8
            }),
            (&sliders.sys, |d, v| {
                d.system_audio_activity_sensitivity = v as u8
            }),
            (&sliders.speaker_timeout, |d, v| {
                d.speaker_analysis.timeout_seconds = v as u64 * 60
            }),
        ];
        for (state, apply) in slides {
            subs.push(cx.subscribe(state, move |this, _, e: &SliderEvent, cx| {
                if let SliderEvent::Change(SliderValue::Single(v))
                | SliderEvent::Release(SliderValue::Single(v)) = e
                {
                    let v = *v;
                    this.edit(|d| apply(d, v), cx);
                }
            }));
        }
        Self {
            data: SettingsData::default(),
            draft: None,
            saved: None,
            kb_draft: None,
            kb_saved: None,
            mic_draft: None,
            mic_saved: None,
            group: GroupId::General,
            section: SectionId::Appearance,
            spy_suppressed: false,
            scroll: ScrollHandle::new(),
            items: Vec::new(),
            item_base: 0,
            pending_scroll: None,
            needs_sync: false,
            pool: Vec::new(),
            pool_loading: false,
            engine: Engine::default(),
            saving: HashSet::new(),
            failed: None,
            fail_count: 0,
            retry_task: None,
            privacy_in_flight: false,
            privacy_error: None,
            video_invalid: false,
            reload: reload::Reloads::default(),
            ui: Ui {
                delete_window: 1, // 5 min, as Tauri
                app_anchor: Some(crate::ui::popover::Anchor::owned(cx)),
                ..Ui::default()
            },
            inputs,
            sliders,
            status: StatusBarModel::default(),
            kb_focus: cx.focus_handle(),
            shown: true,
            access_poll: None,
            backend,
            timer: None,
            _subscriptions: subs,
        }
    }

    /// Swap the backend (goldens and parity serve the fixture's answers).
    pub fn set_backend(&mut self, backend: Arc<dyn Backend>) {
        self.backend = backend;
    }

    /// Adopt freshly loaded answers; drafts reset to what's saved.
    pub fn set_data(&mut self, data: SettingsData, cx: &mut Context<Self>) {
        self.saved = data.settings.clone();
        self.draft = data.settings.clone();
        self.kb_saved = data.keyboard.clone();
        self.kb_draft = data.keyboard.clone();
        self.mic_saved = data.mic.clone();
        self.mic_draft = data.mic.clone();
        self.data = data;
        self.needs_sync = true;
        self.test_providers_once(cx);
        cx.notify();
    }

    /// Reload's stand-in for a re-read: drop unsaved drafts back to the last saved.
    pub fn discard_drafts(&mut self, cx: &mut Context<Self>) {
        self.draft = self.saved.clone();
        self.kb_draft = self.kb_saved.clone();
        self.mic_draft = self.mic_saved.clone();
        self.needs_sync = true;
        self.reload_all(cx);
        self.tick(cx);
        cx.notify();
    }

    /// Push the drafts into the text fields and sliders that mirror them (run
    /// from `render`, which has the window `InputState::set_value` needs).
    pub(crate) fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        sections::coerce_resolution(self, cx);
        // The custom size / bitrate fields hold the Video save (also after a reload).
        let invalid = sections::video_invalid(self, cx);
        if invalid != self.video_invalid {
            self.video_invalid = invalid;
            self.tick(cx);
        }
        if std::mem::take(&mut self.ui.clear_app_search) {
            self.inputs
                .app_search
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        if std::mem::take(&mut self.ui.clear_license_key) {
            self.inputs
                .license_key
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        if std::mem::take(&mut self.ui.clear_search) {
            self.inputs
                .search
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        if !std::mem::take(&mut self.needs_sync) {
            return;
        }
        let Some(d) = self.draft.clone() else { return };
        let set =
            |e: &Entity<InputState>, v: String, window: &mut Window, cx: &mut Context<Self>| {
                e.update(cx, |s, cx| s.set_value(v, window, cx));
            };
        if let ScreenResolution::Custom { width, height } = d.screen_resolution {
            set(&self.inputs.res_w, width.to_string(), window, cx);
            set(&self.inputs.res_h, height.to_string(), window, cx);
        }
        let mbps = d
            .video_bitrate
            .custom_mbps
            .map(|v| v.to_string())
            .unwrap_or_default();
        set(&self.inputs.mbps, mbps, window, cx);
        set(
            &self.inputs.tool_calls,
            d.access.ask_ai_max_tool_calls.to_string(),
            window,
            cx,
        );
        set(
            &self.inputs.idle_unload,
            d.transcription.idle_unload_seconds.to_string(),
            window,
            cx,
        );
        set(
            &self.inputs.chunk,
            d.transcription.chunk_seconds.to_string(),
            window,
            cx,
        );
        let wl = d.ocr.tesseract_char_whitelist.clone().unwrap_or_default();
        set(&self.inputs.whitelist, wl, window, cx);
        let slide =
            |e: &Entity<SliderState>, v: f32, window: &mut Window, cx: &mut Context<Self>| {
                e.update(cx, |s, cx| s.set_value(v, window, cx));
            };
        slide(
            &self.sliders.mic,
            d.microphone_activity_sensitivity as f32,
            window,
            cx,
        );
        slide(
            &self.sliders.sys,
            d.system_audio_activity_sensitivity as f32,
            window,
            cx,
        );
        let minutes = (d.speaker_analysis.timeout_seconds / 60).max(1) as f32;
        slide(&self.sliders.speaker_timeout, minutes, window, cx);
    }

    /// Refill from app-core off the main thread (the real app); the drafts
    /// reset to what's saved.
    pub fn reload_all(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    // ── drafts + autosave ────────────────────────────────────────────────────

    /// The draft the controls show (defaults before the first load).
    pub fn d(&self) -> RecordingSettings {
        self.draft
            .clone()
            .unwrap_or_else(app_core::recording_settings::default_recording_settings)
    }

    /// Change the recording-settings draft, then let autosave decide.
    pub fn edit(&mut self, f: impl FnOnce(&mut RecordingSettings), cx: &mut Context<Self>) {
        if let Some(d) = self.draft.as_mut() {
            f(d);
            self.tick(cx);
            cx.notify();
        }
    }

    pub fn edit_keyboard(&mut self, f: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(k) = self.kb_draft.as_mut() {
            f(k);
            self.tick(cx);
            cx.notify();
        }
    }

    pub fn edit_mic(
        &mut self,
        f: impl FnOnce(&mut MicrophoneControllerState),
        cx: &mut Context<Self>,
    ) {
        if let Some(m) = self.mic_draft.as_mut() {
            f(m);
            self.tick(cx);
            cx.notify();
        }
    }

    // ── navigation ───────────────────────────────────────────────────────────

    /// Select a section's group and scroll to it (rail click, deeplink).
    pub fn navigate(&mut self, section: SectionId, cx: &mut Context<Self>) {
        let group = groups::group_for(section);
        if group != self.group {
            self.scroll
                .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
        }
        self.group = group;
        self.section = section;
        self.spy_suppressed = true;
        view::scroll_to(self, section);
        cx.notify();
    }

    /// `?tab=`: unknown values keep the current group.
    pub fn open_tab(&mut self, tab: &str, cx: &mut Context<Self>) {
        if let Some(section) = groups::resolve_tab(tab) {
            self.navigate(section, cx);
        }
    }
}

/// `ScreenResolutionControl`'s width/height fields: both must parse.
fn set_custom_res(d: &mut RecordingSettings, w: Option<&str>, h: Option<&str>) -> bool {
    let (cw, ch) = match d.screen_resolution {
        ScreenResolution::Custom { width, height } => (width, height),
        _ => (0, 0),
    };
    let parse =
        |raw: Option<&str>, keep: u32| raw.map_or(Some(keep), |r| r.trim().parse::<u32>().ok());
    match (parse(w, cw), parse(h, ch)) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            d.screen_resolution = ScreenResolution::Custom { width, height };
            true
        }
        _ => false,
    }
}

/// `move |v, window, cx| this.update(..)` for a control's `on_change`.
pub fn on<A: 'static>(
    cx: &Context<Settings>,
    f: impl Fn(&mut Settings, A, &mut Context<Settings>) + 'static,
) -> impl Fn(A, &mut Window, &mut App) + 'static {
    let this = cx.entity().downgrade();
    move |a, _, cx| {
        let _ = this.update(cx, |s, cx| f(s, a, cx));
    }
}

/// The native confirm with a "Cancel" button ([`confirm_as`]).
pub fn confirm(
    cx: &mut Context<Settings>,
    title: &str,
    message: &str,
    ok_label: &str,
    then: impl FnOnce(&mut Settings, &mut Context<Settings>) + 'static,
) {
    commands::confirm_as(cx, title, message, ok_label, "Cancel", then);
}

/// The same for a button's `on_click`.
pub fn click(
    cx: &Context<Settings>,
    f: impl Fn(&mut Settings, &mut Context<Settings>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let this = cx.entity().downgrade();
    move |_, _, cx| {
        let _ = this.update(cx, |s, cx| f(s, cx));
    }
}
