//! Privacy › Excluded Apps › "Add app…" (`AppPrivacyExclusion.svelte`): a
//! dropdown under the button with the installed-app search, the first 12
//! matches, ↑/↓ moving the highlight and Enter (or a click) excluding one.
//! Esc, or a press outside it, closes it.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use crate::settings::commands::{candidates, exclude_app};
use crate::settings::{icons, Settings};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::app_icon::icon_or_initial;
use crate::ui::chip::{chip, ChipKind};
use crate::ui::input::TextInput;
use crate::ui::popover::{esc_closes, float, popover};
use crate::ui::Button;

/// `.excl-pop`: 340 px wide, opening rightward from the button's left edge.
const POP_W: f32 = 340.;
/// The trigger's height plus the 4 px gap.
const POP_TOP: f32 = 30.;

impl Settings {
    /// "Add app…": open with an empty, focused search (or close again).
    fn toggle_app_picker(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        if self.ui.app_picker.take().is_some() {
            cx.notify();
            return;
        }
        self.open_app_picker(cx);
        let search = self.inputs.app_search.clone();
        search.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.focus(window, cx)
        });
        cx.notify();
    }

    /// ↑/↓ over the visible matches (wrapping, as `onComboboxKeydown`).
    fn move_app_highlight(&mut self, down: bool, cx: &mut Context<Self>) {
        let query = self.inputs.app_search.read(cx).value().to_string();
        let n = candidates(self, &query).len();
        if n == 0 {
            return;
        }
        let i = self.ui.app_highlight.min(n - 1);
        self.ui.app_highlight = if down { (i + 1) % n } else { (i + n - 1) % n };
        cx.notify();
    }
}

impl Settings {
    /// Ask once, off the main thread, for the icons of the excluded apps and
    /// the open picker's matches (the controller's two `resolveAppIcons` effects).
    pub(crate) fn sync_app_icons(&mut self, cx: &mut Context<Self>) {
        let mut ids: Vec<String> = self
            .d()
            .privacy
            .excluded_apps
            .iter()
            .map(|a| a.bundle_id.trim().to_string())
            .collect();
        if self.ui.app_picker.is_some() {
            let query = self.inputs.app_search.read(cx).value().to_string();
            ids.extend(candidates(self, &query).iter().map(|c| c.0.clone()));
        }
        ids.retain(|b| !b.is_empty() && self.ui.icons_asked.insert(b.clone()));
        if ids.is_empty() {
            return;
        }
        let backend = self.backend.clone();
        let job = cx.background_spawn(async move { backend.app_icons(&ids) });
        cx.spawn(async move |this, cx| {
            let got = job.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.app_icons.extend(got);
                cx.notify();
            });
        })
        .detach();
    }
}

/// `.excl-app`: the app's icon, else `appIconFallback`'s letter on a 22 px tile.
pub(super) fn excl_app(s: &Settings, name: &str, bundle: &str, p: &Palette) -> AnyElement {
    let (raised, border, strong) = (p.surface_raised, p.border, p.text_strong);
    let label = if name.trim().is_empty() { bundle } else { name };
    icon_or_initial(
        s.ui.app_icons.get(bundle.trim()).cloned(),
        label,
        22.,
        move || {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .bg(raised)
                .text_size(text::SM)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(strong)
        },
    )
}

/// The trigger and, while open, its dropdown.
pub fn add_app(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let open = s.ui.app_picker.is_some();
    let at = s.ui.app_anchor.clone();
    let trigger = Button::new("excl-add", "Add app\u{2026}")
        .sm()
        .icon(icons::PLUS)
        .disabled(s.privacy_in_flight)
        .on_click(
            cx.listener(|s, _: &gpui_kit::ClickEvent, window, cx| s.toggle_app_picker(window, cx)),
        );
    let wrapper = match &at {
        Some(a) => a.track(div()),
        None => div(),
    }
    .relative()
    .child(trigger);
    if !open {
        return wrapper.into_any_element();
    }
    let query = s.inputs.app_search.read(cx).value().to_string();
    let listing = s.ui.app_picker.as_ref().is_some_and(|a| a.is_empty());
    let matches = candidates(s, &query);
    let lit = s.ui.app_highlight.min(matches.len().saturating_sub(1));
    let rows: Vec<AnyElement> = matches
        .iter()
        .enumerate()
        .map(|(ix, (bundle, name, running))| {
            let (bundle, name) = (bundle.clone(), name.clone());
            div()
                .id(SharedString::from(format!("excl-pick-{bundle}")))
                .test_support()
                .role(gpui_kit::Role::ListBoxOption)
                .aria_selected(ix == lit)
                .flex()
                .items_center()
                .gap(space::S3)
                .w_full()
                .px(px(10.))
                .py(px(7.))
                .rounded(radius::SM)
                .cursor_pointer()
                .when(ix == lit, |d| d.bg(p.wash_strong))
                .on_mouse_move(cx.listener(move |s, _, _, cx| {
                    if s.ui.app_highlight != ix {
                        s.ui.app_highlight = ix;
                        cx.notify();
                    }
                }))
                .child(excl_app(s, &name, &bundle, p))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(text::MD)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(p.text_strong)
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .font_family(FONT_MONO)
                                .text_size(text::SM)
                                .text_color(p.text_subtle)
                                .child(bundle.clone()),
                        ),
                )
                .when(*running, |d| d.child(chip("running", ChipKind::Neutral, p)))
                .on_click(crate::settings::click(cx, move |s, cx| {
                    exclude_app(s, bundle.clone(), name.clone(), cx)
                }))
                .into_any_element()
        })
        .collect();
    let list = if rows.is_empty() {
        div()
            .px(px(10.))
            .pt(space::S2)
            .pb(space::S3)
            .text_size(text::BASE)
            .text_color(p.text_subtle)
            .child(if listing {
                "Listing installed apps\u{2026}"
            } else {
                "No matching installed apps"
            })
            .into_any_element()
    } else {
        crate::ui::scroll::area(
            div()
                .id("excl-pick-list")
                .role(gpui_kit::Role::ListBox)
                .flex()
                .flex_col()
                .max_h(px(260.))
                .overflow_y_scroll()
                .px(px(4.))
                .pb(px(4.))
                .children(rows),
        )
        .into_any_element()
    };
    let close = {
        let at = at.clone();
        cx.listener(move |s, e: &gpui_kit::MouseDownEvent, _, cx| {
            // A press outside closes it; the trigger toggles it itself.
            if at.as_ref().is_none_or(|a| a.outside(e, cx)) {
                s.ui.app_picker = None;
                cx.notify();
            }
        })
    };
    let this = cx.entity().downgrade();
    let pop = popover("excl-pop", "Exclude an app", p)
        .top(px(POP_TOP))
        .left_0()
        .w(px(POP_W))
        .capture_key_down(cx.listener(|s, e: &gpui_kit::KeyDownEvent, _, cx| {
            match e.keystroke.key.as_str() {
                "down" => s.move_app_highlight(true, cx),
                "up" => s.move_app_highlight(false, cx),
                _ => return,
            }
            cx.stop_propagation();
        }))
        .on_mouse_down_out(close)
        .child(
            div()
                .pt(space::S3)
                .px(space::S4)
                .pb(space::S2)
                .child(TextInput::new(
                    "excl-search",
                    "Search installed apps",
                    &s.inputs.app_search,
                )),
        )
        .child(list)
        .test_support();
    esc_closes(wrapper, true, move |_, cx| {
        let _ = this.update(cx, |s, cx| {
            s.ui.app_picker = None;
            cx.notify();
        });
    })
    .child(float(pop))
    .into_any_element()
}
