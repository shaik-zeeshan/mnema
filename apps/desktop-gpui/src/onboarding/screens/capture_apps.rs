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
    /// The open field; `None` = the "+" shows.
    pub(in crate::onboarding) query: Option<Entity<InputState>>,
    /// Installed apps `(bundle id, name)`, loaded on the first open.
    pub(in crate::onboarding) candidates: Option<Vec<(String, String)>>,
    over_field: bool,
    sub: Option<Subscription>,
}

impl Onboarding {
    pub fn open_app_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Type any app name"));
        query.update(cx, |s, cx| s.focus(window, cx));
        self.app_pick.sub = Some(
            cx.subscribe_in(&query, window, |o, _, e: &InputEvent, _, cx| match e {
                // The first match, as the datalist's Enter would complete it.
                InputEvent::PressEnter { .. } => match o.app_matches(cx).first().cloned() {
                    Some((bundle, name)) => o.add_app(&bundle, &name, cx),
                    None => o.close_app_pick(cx),
                },
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
