//! General › Keyboard Shortcuts (`Shortcuts.svelte`): the global switch, four
//! categories of rebindable chords (click, press keys; Esc cancels, ⌫ clears)
//! and Restore defaults. Bindings save through their own autosave unit.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Keystroke, SharedString};
use serde_json::Value;

use std::collections::HashMap;

use crate::settings::row::{line, row, section};
use crate::settings::{click, icons, on, reload, Settings};
use crate::theme::{self, space, text, Palette};
use crate::ui::switch::Switch;
use crate::ui::{kbd, notice, progress, Button, Tone};

/// One editable action: id, label, description, category, (group key, binding key).
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub path: (&'static str, &'static str),
}

const fn a(
    id: &'static str,
    label: &'static str,
    desc: &'static str,
    group: &'static str,
    key: &'static str,
) -> Action {
    Action {
        id,
        label,
        desc,
        path: (group, key),
    }
}

const BG: &str = "Works even when Mnema is in the background.";
const STOPPED: &str = "Available while recording is stopped.";

/// `SHORTCUT_CATEGORIES` with their `EDITABLE_SHORTCUT_ACTIONS`.
pub static CATEGORIES: [(&str, &[Action]); 4] = [
    (
        "Recording & window",
        &[
            a(
                "toggleRecording",
                "Start or stop recording",
                BG,
                "globalShortcuts.bindings",
                "toggleRecording",
            ),
            a(
                "pauseResumeRecording",
                "Pause or resume recording",
                "Pauses recording, or resumes it, without ending the session.",
                "globalShortcuts.bindings",
                "pauseResumeRecording",
            ),
            a(
                "toggleMainWindow",
                "Show or hide Mnema",
                BG,
                "globalShortcuts.bindings",
                "toggleMainWindow",
            ),
            a(
                "toggleQuickRecall",
                "Summon Quick Recall",
                BG,
                "globalShortcuts.bindings",
                "quickRecall",
            ),
        ],
    ),
    (
        "App",
        &[
            a(
                "openSettings",
                "Open settings",
                "Foreground app shortcut.",
                "appShortcuts",
                "openSettings",
            ),
            a(
                "openDebug",
                "Open debug",
                "Visible when developer options are enabled.",
                "appShortcuts",
                "openDebug",
            ),
            a(
                "toggleSourceScreen",
                "Toggle screen for the next recording",
                STOPPED,
                "appShortcuts",
                "toggleSourceScreen",
            ),
            a(
                "toggleSourceMicrophone",
                "Toggle microphone for the next recording",
                STOPPED,
                "appShortcuts",
                "toggleSourceMicrophone",
            ),
            a(
                "toggleSourceSystemAudio",
                "Toggle system audio for the next recording",
                STOPPED,
                "appShortcuts",
                "toggleSourceSystemAudio",
            ),
            a(
                "toggleShortcutsHelp",
                "Show keyboard shortcuts",
                "Opens the shortcut help overlay.",
                "appShortcuts",
                "toggleShortcutsHelp",
            ),
        ],
    ),
    (
        "Dashboard",
        &[
            a(
                "dashboard.openJumpPicker",
                "Open jump picker",
                "Dashboard timeline navigation.",
                "dashboardShortcuts",
                "openJumpPicker",
            ),
            a(
                "dashboard.jumpLatest",
                "Jump to latest",
                "Move to the newest available frame.",
                "dashboardShortcuts",
                "jumpLatest",
            ),
            a(
                "dashboard.toggleOcr",
                "Toggle OCR panel",
                "Show or hide OCR for the active frame.",
                "dashboardShortcuts",
                "toggleOcr",
            ),
            a(
                "dashboard.refreshTimeline",
                "Refresh timeline",
                "Reload timeline and audio data.",
                "dashboardShortcuts",
                "refreshTimeline",
            ),
            a(
                "dashboard.copyFrame",
                "Copy active frame image",
                "Copies the current frame preview.",
                "dashboardShortcuts",
                "copyFrame",
            ),
            a(
                "dashboard.downloadFrame",
                "Download active frame image",
                "Downloads the current frame preview.",
                "dashboardShortcuts",
                "downloadFrame",
            ),
        ],
    ),
    (
        "Audio Drawer",
        &[
            a(
                "audioDrawer.playPause",
                "Play or pause",
                "Audio drawer playback.",
                "audioDrawerShortcuts",
                "playPause",
            ),
            a(
                "audioDrawer.seekBack",
                "Seek back 5 seconds",
                "Audio drawer playback.",
                "audioDrawerShortcuts",
                "seekBack",
            ),
            a(
                "audioDrawer.seekForward",
                "Seek forward 5 seconds",
                "Audio drawer playback.",
                "audioDrawerShortcuts",
                "seekForward",
            ),
            a(
                "audioDrawer.seekBackFast",
                "Seek back 30 seconds",
                "Audio drawer playback.",
                "audioDrawerShortcuts",
                "seekBackFast",
            ),
            a(
                "audioDrawer.seekForwardFast",
                "Seek forward 30 seconds",
                "Audio drawer playback.",
                "audioDrawerShortcuts",
                "seekForwardFast",
            ),
        ],
    ),
];

fn slot<'a>(kb: &'a mut Value, (group, key): (&str, &str)) -> &'a mut Value {
    let mut v = kb;
    for part in group.split('.') {
        v = &mut v[part];
    }
    &mut v[key]
}

pub fn binding(kb: &Value, (group, key): (&str, &str)) -> String {
    let mut v = kb;
    for part in group.split('.') {
        v = &v[part];
    }
    v[key].as_str().unwrap_or_default().to_string()
}

/// `formatShortcut`: one cap per modifier, then the key.
pub fn tokens(binding: &str) -> Vec<String> {
    if binding.trim().is_empty() {
        return Vec::new();
    }
    binding
        .split('+')
        .map(|part| match part {
            "CommandOrControl" | "Command" | "Cmd" | "Super" => "\u{2318}".into(),
            "Alt" | "Option" => "\u{2325}".into(),
            "Shift" => "\u{21e7}".into(),
            "Control" | "Ctrl" => "\u{2303}".into(),
            "ArrowLeft" => "\u{2190}".into(),
            "ArrowRight" => "\u{2192}".into(),
            "ArrowUp" => "\u{2191}".into(),
            "ArrowDown" => "\u{2193}".into(),
            "Escape" => "Esc".into(),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => k.to_string(),
        })
        .collect()
}

/// A pressed chord as a stored binding (`CommandOrControl+Alt+R`); None for a bare modifier.
pub fn from_keystroke(k: &Keystroke) -> Option<String> {
    let key = match k.key.as_str() {
        "shift" | "control" | "alt" | "platform" | "function" | "" => return None,
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "space" => "Space".into(),
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    };
    let m = &k.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if m.platform {
        parts.push("CommandOrControl".into());
    }
    if m.control {
        parts.push("Control".into());
    }
    if m.alt {
        parts.push("Alt".into());
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// The listening row's keydown: Esc cancels, ⌫ clears, anything else binds.
/// A Control chord or a key shortcuts can't use stays listening and says why.
pub fn capture(s: &mut Settings, k: &Keystroke, cx: &mut Context<Settings>) -> bool {
    let Some(id) = s.ui.kb_listening else {
        return false;
    };
    let Some(action) = CATEGORIES
        .iter()
        .flat_map(|(_, a)| a.iter())
        .find(|a| a.id == id)
    else {
        return false;
    };
    let path = action.path;
    let refuse = |s: &mut Settings, why: &str, cx: &mut Context<Settings>| {
        s.ui.kb_capture_error = Some((id, why.to_string()));
        cx.notify();
        true
    };
    let picked = match k.key.as_str() {
        "escape" => None,
        "backspace" | "delete" => Some(String::new()),
        "shift" | "control" | "alt" | "platform" | "function" | "" => return true,
        key => {
            if k.modifiers.control {
                return refuse(
                    s,
                    "Control shortcuts are not supported on macOS. Use Command or Option.",
                    cx,
                );
            }
            let named = from_keystroke(k).unwrap_or_default();
            let last = named.rsplit('+').next().unwrap_or(key);
            if super::shortcut_issues::normalize_key(last).is_none() {
                return refuse(s, "That key is not supported for shortcuts.", cx);
            }
            Some(named)
        }
    };
    // Listening ends first: while a row listens the shortcuts never save.
    s.ui.kb_listening = None;
    s.ui.kb_capture_error = None;
    match picked {
        Some(b) => s.edit_keyboard(|kb| *slot(kb, path) = Value::String(b), cx),
        None => s.tick(cx),
    }
    cx.notify();
    true
}

/// A press anywhere but a chord button ends listening (Svelte's pointerdown).
pub fn cancel_capture(s: &mut Settings, cx: &mut Context<Settings>) {
    if s.ui.kb_listening.take().is_some() {
        s.ui.kb_capture_error = None;
        s.tick(cx);
        cx.notify();
    }
}

/// The row's issue: a refused key while listening, else its draft's conflict.
fn issue_for(s: &Settings, id: &str, issues: &HashMap<&'static str, String>) -> Option<String> {
    match &s.ui.kb_capture_error {
        Some((e, why)) if *e == id => Some(why.clone()),
        _ => issues.get(id).cloned(),
    }
}

/// "Jump to conflict": centre the first conflicting row in the column.
fn jump_to_conflict(s: &mut Settings, cx: &mut Context<Settings>) {
    let Some(b) = s.ui.kb_conflict_row.get() else {
        return;
    };
    let view = s.scroll.bounds();
    let offset = s.scroll.offset();
    let content_top = b.top() - view.top() - offset.y;
    let y = (content_top - (view.size.height - b.size.height) / 2.)
        .max(px(0.))
        .min(s.scroll.max_offset().y);
    s.scroll.set_offset(gpui_kit::point(offset.x, -y));
    s.spy_suppressed = false;
    cx.notify();
}

fn chord(
    s: &Settings,
    action: &'static Action,
    issue: bool,
    saving: bool,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let listening = s.ui.kb_listening == Some(action.id);
    let kb = s.kb_draft.clone().unwrap_or_default();
    let toks = tokens(&binding(&kb, action.path));
    let id = action.id;
    let inner = if listening {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(progress::spinner(theme::size::SPIN_SM))
            .child("Press keys\u{2026}")
            .into_any_element()
    } else if toks.is_empty() {
        div().child("Set shortcut").into_any_element()
    } else {
        div()
            .flex()
            .gap(px(3.))
            .children(toks.into_iter().map(|t| {
                kbd(SharedString::from(t), p)
                    .h(px(20.))
                    .min_w(px(20.))
                    .text_size(text::SM)
            }))
            .into_any_element()
    };
    div()
        .id(SharedString::from(format!("kb-{id}")))
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(SharedString::from(if listening {
            format!("Listening for {} shortcut", action.label)
        } else {
            format!("Set shortcut for {}", action.label)
        }))
        .flex()
        .flex_none()
        .items_center()
        .h(theme::size::H_MD)
        .px(space::S3)
        .rounded(theme::radius::SM)
        .border_1()
        .border_color(if issue {
            p.danger_border
        } else {
            p.border_strong
        })
        .bg(p.surface)
        .text_size(text::MD)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text)
        .child(inner)
        // Its own press must not reach the page's "press elsewhere cancels".
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .when(listening, |d| d.track_focus(&s.kb_focus))
        .when(saving, |d| d.opacity(theme::DISABLED_OPACITY))
        .when(!saving, |d| {
            d.cursor_pointer()
                .hover(|st| st.bg(p.surface_hover))
                .on_click(cx.listener(move |s, _: &gpui_kit::ClickEvent, window, cx| {
                    s.ui.kb_capture_error = None;
                    s.ui.kb_listening = if s.ui.kb_listening == Some(id) {
                        None
                    } else {
                        // `event.currentTarget.focus()`: the keys come here.
                        s.kb_focus.focus(window, cx);
                        Some(id)
                    };
                    s.tick(cx);
                    cx.notify();
                }))
        })
        .into_any_element()
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let saving = s
        .saving
        .contains(&crate::settings::autosave::Unit::Keyboard);
    let actions = vec![reload::button(
        s,
        "reload-shortcuts",
        "Reload shortcuts from saved settings",
        false,
        |s, cx| {
            s.kb_draft = s.data.keyboard.clone();
            cx.notify();
        },
        cx,
    )];
    let kb = s.kb_draft.clone().unwrap_or_default();
    let issues = super::shortcut_issues::issues(&kb);
    let first = super::shortcut_issues::first(&issues);
    let mut rows: Vec<AnyElement> = Vec::new();
    if !issues.is_empty() {
        rows.push(
            div()
                .my(space::S3)
                .child(notice::notice(
                    Tone::Danger,
                    icons::ALERT,
                    "Shortcuts not saved",
                    Some(
                        "Resolve shortcut conflicts or invalid shortcuts before changes are saved."
                            .into(),
                    ),
                    vec![Button::new("kb-jump", "Jump to conflict")
                        .sm()
                        .on_click(click(cx, jump_to_conflict))
                        .into_any_element()],
                    p,
                ))
                .into_any_element(),
        );
    }
    rows.push(row("Global shortcuts")
        .desc("Work from any app while Mnema is in the background. Background shortcuts need Command/Control or Option.")
        .control(
            Switch::new("kb-global", "Global shortcuts", kb["globalShortcuts"]["enabled"].as_bool().unwrap_or(false))
                .on_toggle(on(cx, |s, v: bool, cx| s.edit_keyboard(|kb| kb["globalShortcuts"]["enabled"] = Value::Bool(v), cx))),
        )
        .build(p)
        .into_any_element());
    for (title, actions) in CATEGORIES.iter() {
        rows.push(
            div()
                .pt(space::S5)
                .pb(space::S2)
                .border_b_1()
                .border_color(p.hairline)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_muted)
                .child(*title)
                .into_any_element(),
        );
        for action in actions.iter() {
            let issue = issue_for(s, action.id, &issues);
            let r = row(action.label).desc(action.desc);
            let el = match issue {
                // The issue line sits under the copy, so the chord moves aside.
                Some(why) => r
                    .full()
                    .aside(chord(s, action, true, saving, p, cx))
                    .control(line(Tone::Danger, icons::ALERT, why, p))
                    .build(p),
                None => r.control(chord(s, action, false, saving, p, cx)).build(p),
            };
            if first == Some(action.id) {
                let cell = s.ui.kb_conflict_row.clone();
                rows.push(
                    div()
                        .relative()
                        .child(el)
                        .child(
                            gpui_kit::canvas(move |b, _, _| cell.set(Some(b)), |_, _, _, _| {})
                                .absolute()
                                .size_full(),
                        )
                        .into_any_element(),
                );
            } else {
                rows.push(el.into_any_element());
            }
        }
    }
    rows.push(
        row("Restore all default shortcuts")
            .desc("Asks first.")
            .control(
                Button::new("kb-restore", "Restore defaults")
                    .disabled(saving)
                    .on_click(click(cx, |_, cx| {
                        crate::settings::confirm(
                            cx,
                            "Restore default shortcuts",
                            "Restore all keyboard shortcuts to their defaults?",
                            "Restore defaults",
                            |s, cx| {
                                let d =
                                    app_core::keyboard_bindings::KeyboardBindingsSettings::defaults(
                                    );
                                if let Ok(d) = serde_json::to_value(d) {
                                    s.edit_keyboard(|kb| *kb = d, cx);
                                }
                            },
                        );
                    })),
            )
            .build(p)
            .into_any_element(),
    );
    section(
        "Keyboard Shortcuts",
        Some("Click a shortcut to rebind it, then press the keys. Esc cancels, \u{232b} clears."),
        actions,
        false,
        rows,
        p,
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_to_caps() {
        assert_eq!(
            tokens("CommandOrControl+Alt+R"),
            vec!["\u{2318}", "\u{2325}", "R"]
        );
        assert_eq!(tokens("Shift+ArrowLeft"), vec!["\u{21e7}", "\u{2190}"]);
        assert!(tokens("").is_empty());
        let k = Keystroke::parse("cmd-alt-r").unwrap();
        assert_eq!(
            from_keystroke(&k).as_deref(),
            Some("CommandOrControl+Alt+R")
        );
        assert_eq!(
            from_keystroke(&Keystroke::parse("shift-left").unwrap()).as_deref(),
            Some("Shift+ArrowLeft")
        );
    }
}
