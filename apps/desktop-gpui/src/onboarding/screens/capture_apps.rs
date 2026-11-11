//! Capture & Storage's "Never recorded" Add (`ExcludedApps.svelte`): the
//! icon-only "+" after the app chips swaps in a type-ahead field over the
//! installed apps (app-core's privacy app candidates). A pick is a draft-only
//! `upsert_privacy_excluded_app` (re-enabling a struck row with the same
//! identity); nothing else is resynced.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, AppContext, Context, Entity, Subscription, Window};

use crate::onboarding::Onboarding;
use crate::theme;
use crate::ui::input::TextInput;
use crate::ui::menu::{option_row, options_empty};
use crate::ui::select::list_box;
use crate::ui::{nid, Button};

/// `.add-input { width: 190px }`.
const FIELD_W: f32 = 190.;

#[derive(Default)]
pub struct AppPick {
    /// Resolved icons by bundle id (`resolve_app_icons`); a miss stays a letter.
    pub(in crate::onboarding) icons: std::collections::HashMap<String, std::path::PathBuf>,
    icons_asked: std::collections::HashSet<String>,
    /// The open field; `None` = the "+" shows.
    pub(in crate::onboarding) query: Option<Entity<InputState>>,
    /// Installed apps `(bundle id, name)`, loaded on the first open.
    pub(in crate::onboarding) candidates: Option<Vec<(String, String)>>,
    over_field: bool,
    sub: Option<Subscription>,
}

/// `resolveTypedApp`: the installed app with exactly this name, else the first
/// whose name starts with it (case-insensitive).
pub fn resolve_typed<'a>(
    typed: &str,
    candidates: impl Iterator<Item = &'a (String, String)> + Clone,
) -> Option<(String, String)> {
    let needle = typed.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let name = |c: &&(String, String)| c.1.trim().to_lowercase();
    candidates
        .clone()
        .find(|c| name(c) == needle)
        .or_else(|| candidates.clone().find(|c| name(c).starts_with(&needle)))
        .cloned()
}

/// A rule with no bundle id waits for the app to be installed (`isPendingExclusion`).
pub fn is_pending(a: &capture_types::ExcludedAppEntry) -> bool {
    a.bundle_id.trim().is_empty() && !a.display_name.trim().is_empty()
}

/// The line under the apps (`excludedNote`): what the rules do, then a hint.
pub fn excluded_note(entries: &[capture_types::ExcludedAppEntry]) -> (String, &'static str) {
    if entries.is_empty() {
        return (
            "Nothing is hidden. Everything on your screen is recorded.".into(),
            "Add an app it should never see.",
        );
    }
    if !entries.iter().any(|e| e.enabled) {
        return (String::new(), "Click a crossed-out name to hide it again.");
    }
    let waiting = entries
        .iter()
        .filter(|e| e.enabled && is_pending(e))
        .count();
    let mut parts = Vec::new();
    if entries
        .iter()
        .any(|e| e.enabled && !e.bundle_id.trim().is_empty())
    {
        parts.push(
            "Nothing from them is recorded \u{2014} not the screen, and not system audio while that filter is on."
                .to_string(),
        );
    }
    match waiting {
        0 => {}
        1 => parts.push("One of them isn\u{2019}t installed yet \u{2014} the rule waits.".into()),
        n => parts.push(format!(
            "{n} of them aren\u{2019}t installed yet \u{2014} the rule waits."
        )),
    }
    (parts.join(" "), "Click a name to record it after all.")
}

impl Onboarding {
    /// Ask once, off the main thread, for the icons of the listed apps that
    /// have a bundle id (a rule waiting for its app has none to draw).
    pub(super) fn request_app_icons(&mut self, cx: &mut Context<Self>) {
        let ask: Vec<String> = self
            .draft
            .privacy
            .excluded_apps
            .iter()
            .map(|a| a.bundle_id.trim().to_string())
            .filter(|b| !b.is_empty() && self.app_pick.icons_asked.insert(b.clone()))
            .collect();
        if ask.is_empty() {
            return;
        }
        self.run(
            cx,
            move |b| b.app_icons(&ask),
            |s, got, cx| {
                s.app_pick.icons.extend(got);
                cx.notify();
            },
        );
    }

    pub fn open_app_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Type any app name"));
        query.update(cx, |s, cx| s.focus(window, cx));
        self.app_pick.sub =
            Some(
                cx.subscribe_in(&query, window, |o, field, e: &InputEvent, _, cx| match e {
                    // `commitAdd`: an installed app by name, else a rule that waits for it.
                    InputEvent::PressEnter { .. } => {
                        let typed = field.read(cx).value().trim().to_string();
                        if typed.is_empty() {
                            return o.close_app_pick(cx);
                        }
                        let found = resolve_typed(&typed, o.app_pick.candidates.iter().flatten());
                        match found {
                            Some((bundle, name)) => o.add_app(&bundle, &name, cx),
                            None => o.add_app("", &typed, cx),
                        }
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                }),
            );
        self.app_pick.query = Some(query);
        if self.app_pick.candidates.is_none() {
            self.run(
                cx,
                |b| b.app_candidates(),
                |s, c, _| s.app_pick.candidates = Some(c),
            );
        }
        cx.notify();
    }

    pub fn close_app_pick(&mut self, cx: &mut Context<Self>) {
        self.app_pick.query = None;
        self.app_pick.sub = None;
        self.app_pick.over_field = false;
        cx.notify();
    }

    /// Adds `bundle` (or re-enables its struck row), then closes the field.
    pub fn add_app(&mut self, bundle: &str, name: &str, cx: &mut Context<Self>) {
        let _ = app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
            &mut self.draft,
            bundle,
            name,
            true,
        );
        self.close_app_pick(cx);
    }

    /// Candidates whose name contains the typed text (case-insensitive).
    fn app_matches(&self, cx: &Context<Self>) -> Vec<(String, String)> {
        let q = self
            .app_pick
            .query
            .as_ref()
            .map(|s| s.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        self.app_pick
            .candidates
            .iter()
            .flatten()
            .filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub(super) fn app_add(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(query) = self.app_pick.query.clone() else {
            return Button::icon_only(
                "ob-app-add",
                "icons/plus.svg",
                "Add an app it should never see",
            )
            .ghost()
            .sm()
            .tip("Add an app it should never see")
            .on_click(cx.listener(|this, _, window, cx| this.open_app_pick(window, cx)))
            .into_any_element();
        };
        let p = theme::tokens(cx);
        let loading = self.app_pick.candidates.is_none();
        let rows: Vec<AnyElement> = self
            .app_matches(cx)
            .into_iter()
            .enumerate()
            .map(|(i, (bundle, name))| {
                option_row(nid("ob-app-cand", i as u64), name.clone(), false, false, &p)
                    .on_click(cx.listener(move |this, _, _, cx| this.add_app(&bundle, &name, cx)))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        let list = list_box(nid("ob-app-list", 0), false, &p)
            .when(empty, |d| {
                d.child(options_empty(
                    if loading {
                        "Looking for apps\u{2026}"
                    } else {
                        "No matches"
                    },
                    &p,
                ))
            })
            .children(rows)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.app_pick.over_field {
                    this.close_app_pick(cx);
                }
            }));
        div()
            .id("ob-app-pick")
            .relative()
            .flex()
            .items_center()
            .on_hover(cx.listener(|this, h: &bool, _, _| this.app_pick.over_field = *h))
            .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.close_app_pick(cx);
                }
            }))
            .child(
                TextInput::new(
                    "ob-app-query",
                    "Add an app Mnema should never record",
                    &query,
                )
                .w(px(FIELD_W)),
            )
            .child(crate::ui::float(crate::ui::scroll::area(list)))
            .into_any_element()
    }
}
