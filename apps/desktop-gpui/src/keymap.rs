//! App actions, their bindings (the user's `keyboard-bindings.json` settings),
//! and the shortcuts sheet's rows. Ported from `lib/keyboard.ts`,
//! `lib/keyboard-bindings.svelte.ts` and `lib/global-shortcuts.ts`.
//!
//! - Alt chords match the physical key: GPUI matches a binding on `Keystroke::key`
//!   (the key's unmodified character), never on `key_char` (what ⌥ typed, e.g. ®).
//! - App shortcuts without ⌘ are bound in [`APP_CONTEXT`], so they never fire while
//!   an input (or, from S6, a focused button) has focus.
//! - Esc / Tab / ⇧Tab / the frame arrows are reserved and can't be rebound.

use std::rc::Rc;

use gpui_kit::{
    actions, Action, App, Global, KeyBinding, KeyBindingContextPredicate, KeyBindingMetaIndex,
    Keystroke, OperatingSystem,
};
use serde_json::Value;

actions!(
    mnema,
    [
        FocusRecall,
        FocusAsk,
        GoTimeline,
        GoInsights,
        OpenChats,
        ToggleSettings,
        OpenDebug,
        ToggleShortcutsHelp,
        CloseOverlay,
        ToggleSourceScreen,
        ToggleSourceMicrophone,
        ToggleSourceSystemAudio,
    ]
);

// ↵ / ⌘↵ arrive as the input's `PressEnter` event; Tab and Esc are ours.
actions!(command_field, [FlipMode, Dismiss]);

/// The main window root's key context.
pub const SHELL_CONTEXT: &str = "Shell";
/// Where unmodified app shortcuts apply: inside the shell, not while typing.
pub const APP_CONTEXT: &str = "Shell && !Input && !MxButton";
pub const COMMAND_FIELD_CONTEXT: &str = "CommandField";
/// As deep as the kit input's own Tab / Esc bindings, and registered later, so ours win.
const IN_COMMAND_FIELD: &str = "CommandField > Input";

/// `appShortcuts` + `globalShortcuts.bindings` from the keyboard settings, as the
/// app stores them (`CommandOrControl+Alt+R`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub toggle_recording: String,
    pub pause_resume_recording: String,
    pub toggle_main_window: String,
    pub quick_recall: String,
    pub open_settings: String,
    pub open_debug: String,
    pub sources: [String; 3],
    pub toggle_shortcuts_help: String,
    /// `audioDrawerShortcuts`: play/pause, −5 s, +5 s, −30 s, +30 s.
    pub audio_drawer: [String; 5],
}

impl Default for Bindings {
    /// `DEFAULT_KEYBOARD_BINDINGS`.
    fn default() -> Self {
        Self {
            toggle_recording: "CommandOrControl+Alt+R".into(),
            pause_resume_recording: "CommandOrControl+Alt+P".into(),
            toggle_main_window: "CommandOrControl+Alt+M".into(),
            quick_recall: "CommandOrControl+Alt+Space".into(),
            open_settings: "CommandOrControl+,".into(),
            open_debug: "CommandOrControl+D".into(),
            sources: ["1".into(), "2".into(), "3".into()],
            toggle_shortcuts_help: "/".into(),
            audio_drawer: [
                "Space".into(),
                "ArrowLeft".into(),
                "ArrowRight".into(),
                "Shift+ArrowLeft".into(),
                "Shift+ArrowRight".into(),
            ],
        }
    }
}

impl Bindings {
    /// From `get_keyboard_bindings_settings` JSON; a missing or reserved entry keeps its default.
    // Reads the wire JSON: the settings event carries it, and so does the parity fixture.
    pub fn from_settings(settings: &Value) -> Self {
        Self::default().with_settings(settings)
    }

    /// `settings` over `self`: a missing, reserved or unparseable entry keeps its
    /// current key; an empty one clears it.
    pub fn with_settings(mut self, settings: &Value) -> Self {
        let b = &mut self;
        let pick = |slot: &mut String, group: &str, key: &str| {
            let value = settings
                .get(group)
                .and_then(|g| {
                    if group == "globalShortcuts" {
                        g.get("bindings")
                    } else {
                        Some(g)
                    }
                })
                .and_then(|g| g.get(key))
                .and_then(Value::as_str);
            if let Some(v) = value.filter(|v| !is_reserved(v) && parses(v)) {
                *slot = v.to_string();
            }
        };
        pick(
            &mut b.toggle_recording,
            "globalShortcuts",
            "toggleRecording",
        );
        pick(
            &mut b.pause_resume_recording,
            "globalShortcuts",
            "pauseResumeRecording",
        );
        pick(
            &mut b.toggle_main_window,
            "globalShortcuts",
            "toggleMainWindow",
        );
        pick(&mut b.quick_recall, "globalShortcuts", "quickRecall");
        pick(&mut b.open_settings, "appShortcuts", "openSettings");
        pick(&mut b.open_debug, "appShortcuts", "openDebug");
        let [s, m, a] = &mut b.sources;
        pick(s, "appShortcuts", "toggleSourceScreen");
        pick(m, "appShortcuts", "toggleSourceMicrophone");
        pick(a, "appShortcuts", "toggleSourceSystemAudio");
        pick(
            &mut b.toggle_shortcuts_help,
            "appShortcuts",
            "toggleShortcutsHelp",
        );
        // The drawer's own scope: the frame arrows aren't reserved there, Esc / Tab are.
        let drawer = settings.get("audioDrawerShortcuts");
        let keys = [
            "playPause",
            "seekBack",
            "seekForward",
            "seekBackFast",
            "seekForwardFast",
        ];
        for (slot, key) in b.audio_drawer.iter_mut().zip(keys) {
            let value = drawer.and_then(|g| g.get(key)).and_then(Value::as_str);
            let reserved =
                |v: &str| ["escape", "tab", "shift+tab"].contains(&v.to_ascii_lowercase().as_str());
            if let Some(v) = value.filter(|v| !reserved(v) && parses(v)) {
                *slot = v.to_string();
            }
        }
        self
    }
}

/// Empty (a cleared binding) or a keystroke GPUI can bind.
fn parses(binding: &str) -> bool {
    binding.trim().is_empty()
        || Keystroke::parse_with_os(&to_gpui(binding), OperatingSystem::Mac).is_ok()
}

/// `RESERVED_SHORTCUT_BINDINGS`.
pub fn is_reserved(binding: &str) -> bool {
    const RESERVED: [&str; 7] = [
        "escape",
        "tab",
        "shift+tab",
        "arrowleft",
        "arrowright",
        "shift+arrowleft",
        "shift+arrowright",
    ];
    RESERVED.contains(&binding.to_ascii_lowercase().as_str())
}

fn key_name(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "arrowleft" => "left".into(),
        "arrowright" => "right".into(),
        "arrowup" => "up".into(),
        "arrowdown" => "down".into(),
        k => k.into(),
    }
}

/// `CommandOrControl+Alt+R` -> GPUI's `cmd-alt-r`.
pub fn to_gpui(binding: &str) -> String {
    let parts: Vec<&str> = binding.split('+').collect();
    // `CommandOrControl++` binds the plus key itself.
    let (mods, key) = match binding.strip_suffix("++") {
        Some(rest) => (rest.split('+').collect::<Vec<_>>(), "+"),
        None => (parts[..parts.len() - 1].to_vec(), parts[parts.len() - 1]),
    };
    let mut out: Vec<String> = mods
        .iter()
        .map(|m| match *m {
            "CommandOrControl" | "Command" | "Cmd" | "Super" => "cmd".into(),
            "Control" | "Ctrl" => "ctrl".into(),
            "Alt" | "Option" => "alt".into(),
            other => other.to_ascii_lowercase(),
        })
        .collect();
    out.push(key_name(key));
    out.join("-")
}

/// `CommandOrControl+Alt+R` -> `⌘⌥R` (`formatShortcut`).
pub fn display(binding: &str) -> String {
    let gpui = to_gpui(binding);
    let mut text = String::new();
    let mut parts = gpui.split('-').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            text.push_str(&match part {
                "escape" => "Esc".into(),
                "space" => "Space".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "up" => "↑".into(),
                "down" => "↓".into(),
                k => k.to_uppercase(),
            });
        } else {
            text.push_str(match part {
                "cmd" => "⌘",
                "alt" => "⌥",
                "shift" => "⇧",
                "ctrl" => "⌃",
                _ => "",
            });
        }
    }
    text
}

/// A binding from the user's settings: parsed at runtime, so a bad string is
/// logged and skipped rather than a panic. A cleared one binds nothing.
fn user_binding<A: Action>(keys: &str, action: A, context: &str, cx: &App) -> Option<KeyBinding> {
    if keys.trim().is_empty() {
        return None;
    }
    let predicate = KeyBindingContextPredicate::parse(context).ok().map(Rc::new);
    match KeyBinding::load(&to_gpui(keys), Box::new(action), predicate, false, None, cx) {
        Ok(binding) => Some(binding),
        Err(e) => {
            log::warn!("keymap: skipped an unparseable shortcut binding: {e}");
            None
        }
    }
}

/// Tags every binding [`bind`] installs, so the next call can take them all out
/// again. Meta only ranks against `NoAction` bindings, and there any meta acts
/// like an untagged binding, so the tag changes no precedence.
const OURS: KeyBindingMetaIndex = KeyBindingMetaIndex(0x4d4e_4d41);

/// What [`bind`] last installed, so [`rebind`] can keep a slot the new settings refuse.
struct Bound(Bindings);

impl Global for Bound {}

/// A keyboard settings change: rebind from `settings` over what is bound now.
pub fn rebind(settings: &Value, cx: &mut App) {
    let current = cx
        .try_global::<Bound>()
        .map(|b| b.0.clone())
        .unwrap_or_default();
    bind(&current.with_settings(settings), cx);
}

/// Install the app's bindings, replacing the set a previous call installed, so a
/// rebound or cleared key stops firing at once. Everything else in the keymap
/// (gpui-kit's component bindings, ⌘Q) is kept, in its order.
pub fn bind(bindings: &Bindings, cx: &mut App) {
    let shell = Some(SHELL_CONTEXT);
    let field = Some(IN_COMMAND_FIELD);
    let user = [
        user_binding(&bindings.open_settings, ToggleSettings, SHELL_CONTEXT, cx),
        user_binding(&bindings.open_debug, OpenDebug, SHELL_CONTEXT, cx),
        user_binding(
            &bindings.toggle_shortcuts_help,
            ToggleShortcutsHelp,
            APP_CONTEXT,
            cx,
        ),
        user_binding(&bindings.sources[0], ToggleSourceScreen, APP_CONTEXT, cx),
        user_binding(
            &bindings.sources[1],
            ToggleSourceMicrophone,
            APP_CONTEXT,
            cx,
        ),
        user_binding(
            &bindings.sources[2],
            ToggleSourceSystemAudio,
            APP_CONTEXT,
            cx,
        ),
    ];
    let drawer = {
        use crate::timeline::audio::{
            self, PlayPause, SeekBack, SeekBackFast, SeekForward, SeekForwardFast,
        };
        let [play, back, fwd, back_fast, fwd_fast] = &bindings.audio_drawer;
        [
            user_binding(play, PlayPause, audio::CONTEXT, cx),
            user_binding(back, SeekBack, audio::CONTEXT, cx),
            user_binding(fwd, SeekForward, audio::CONTEXT, cx),
            user_binding(back_fast, SeekBackFast, audio::CONTEXT, cx),
            user_binding(fwd_fast, SeekForwardFast, audio::CONTEXT, cx),
            Some(KeyBinding::new(
                "escape",
                audio::Escape,
                Some("AudioDrawer"),
            )),
        ]
    };
    let fixed = [
        KeyBinding::new("cmd-k", FocusRecall, shell),
        KeyBinding::new("cmd-j", FocusAsk, shell),
        KeyBinding::new("cmd-1", GoTimeline, shell),
        KeyBinding::new("cmd-2", GoInsights, shell),
        KeyBinding::new("escape", CloseOverlay, shell),
        KeyBinding::new("shift-/", ToggleShortcutsHelp, Some(APP_CONTEXT)),
        KeyBinding::new("tab", FlipMode, field),
        KeyBinding::new("escape", Dismiss, field),
    ];
    let kept: Vec<KeyBinding> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|b| b.meta() != Some(OURS))
        .cloned()
        .collect();
    let ours = crate::timeline::key_bindings()
        .into_iter()
        .chain(drawer.into_iter().flatten())
        .chain(fixed)
        .chain(user.into_iter().flatten())
        .map(|b| b.with_meta(OURS));
    cx.clear_key_bindings();
    cx.bind_keys(kept.into_iter().chain(ours));
    cx.set_global(Bound(bindings.clone()));
    // The menu bar reads its shortcuts from the keymap only when it's set.
    crate::native::app_menu::refresh(cx);
}

/// One row of the shortcuts sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpRow {
    pub label: String,
    pub keys: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpGroup {
    pub title: &'static str,
    pub rows: Vec<HelpRow>,
}

/// What the sheet's Global group shows depends on capture state (ShortcutsPop.svelte).
#[derive(Clone, Copy, Debug, Default)]
pub struct HelpState {
    pub capturing: bool,
    pub user_paused: bool,
    pub can_toggle_recording: bool,
    pub can_toggle_sources: bool,
    pub developer_options: bool,
}

/// The sheet, built live: Global first, then the shell's own group. Surfaces
/// append theirs (S8 onward) through `extra`.
pub fn help_groups(b: &Bindings, s: HelpState, extra: Vec<HelpGroup>) -> Vec<HelpGroup> {
    let row = |label: &str, keys: String| HelpRow {
        label: label.into(),
        keys,
    };
    let mut global = Vec::new();
    if s.can_toggle_recording {
        let label = if s.capturing {
            "Stop recording"
        } else {
            "Start recording"
        };
        global.push(row(label, display(&b.toggle_recording)));
    }
    if s.capturing {
        let label = if s.user_paused {
            "Resume recording"
        } else {
            "Pause recording"
        };
        global.push(row(label, display(&b.pause_resume_recording)));
    }
    global.push(row("Show or hide Mnema", display(&b.toggle_main_window)));
    global.push(row("Summon Quick Recall", display(&b.quick_recall)));
    global.push(row("Open settings", display(&b.open_settings)));
    if s.developer_options {
        global.push(row("Open debug", display(&b.open_debug)));
    }
    if s.can_toggle_sources {
        for (i, what) in ["screen", "microphone", "system audio"].iter().enumerate() {
            let label = format!("Toggle {what} for the next recording");
            global.push(row(&label, display(&b.sources[i])));
        }
    }
    global.push(row(
        "Show keyboard shortcuts",
        display(&b.toggle_shortcuts_help),
    ));
    global.push(row("Close keyboard shortcuts", "Esc".into()));
    let here = vec![
        row("Recall anything", "⌘K".into()),
        row("Ask", "⌘J".into()),
        row("Timeline", "⌘1".into()),
        row("Insights", "⌘2".into()),
    ];
    let mut groups = vec![
        HelpGroup {
            title: "Global",
            rows: global,
        },
        HelpGroup {
            title: "Here",
            rows: here,
        },
    ];
    groups.extend(extra.into_iter().filter(|g| !g.rows.is_empty()));
    groups
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod tests;
