//! The window: rail | (bar over one scroll column). The column's direct
//! children are the page head, any save notices, then one block per section,
//! so `ScrollHandle::bounds_for_item` gives the scroll-spy each section's top.

use gpui_kit::prelude::*;
use gpui_kit::{div, point, px, AnyElement, Context, ScrollEvent, Window};

use super::groups::{self, SectionId, SPY_LINE};
use super::{bar, icons, rail, sections, Settings};
use crate::theme::{self, breakpoint, space, text, FONT_UI};
use crate::ui::{notice, Button, Tone};

/// `.settings-page`'s max width.
pub const PAGE_MAX_W: f32 = 760.;
/// `scroll-margin-top` on each section anchor.
const SCROLL_MARGIN: f32 = 24.;

/// Park `section`'s top just under the column top (a group's first: the top).
pub fn scroll_to(s: &mut Settings, section: SectionId) {
    let first = groups::group(groups::group_for(section)).sections[0].id;
    if section == first {
        s.scroll.set_offset(point(px(0.), px(0.)));
        return;
    }
    let Some(ix) = s.items.iter().position(|id| *id == section) else {
        s.pending_scroll = Some(section);
        return;
    };
    let item = s.item_base + ix;
    match s.scroll.bounds_for_item(item) {
        Some(b) => {
            let view = s.scroll.bounds();
            let offset = s.scroll.offset();
            let content_top = b.top() - view.top() - offset.y;
            let max = s.scroll.max_offset().y;
            let y = (content_top - px(SCROLL_MARGIN)).max(px(0.)).min(max);
            s.scroll.set_offset(point(offset.x, -y));
            s.pending_scroll = None;
        }
        // Not laid out yet: settle it after the next paint.
        None => s.pending_scroll = Some(section),
    }
}

/// The section whose top crossed the spy line (unless a click owns it).
fn spy(s: &mut Settings) {
    if s.spy_suppressed || s.items.is_empty() {
        return;
    }
    let view = s.scroll.bounds();
    let tops: Vec<(SectionId, f32)> = s
        .items
        .iter()
        .enumerate()
        .filter_map(|(ix, id)| {
            let b = s.scroll.bounds_for_item(s.item_base + ix)?;
            Some((*id, f32::from(b.top() - view.top())))
        })
        .collect();
    let at_bottom = f32::from(-s.scroll.offset().y) >= f32::from(s.scroll.max_offset().y) - 2.;
    if let Some(id) = groups::active_section_at(&tops, SPY_LINE, at_bottom) {
        s.section = id;
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        self.sync_controls(window, cx);
        if let Some(section) = self.pending_scroll {
            scroll_to(self, section);
        }
        spy(self);
        let p = theme::tokens(cx);
        let g = groups::group(self.group);
        let scrolled = f32::from(self.scroll.offset().y) < -4.;

        let head = div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .mb(space::S6)
            .max_w(px(PAGE_MAX_W))
            .child(
                div()
                    .text_size(text::H1)
                    .font_weight(theme::W_SEMIBOLD)
                    .line_height(gpui_kit::relative(1.15))
                    .text_color(p.text_strong)
                    .child(g.label),
            )
            .child(
                div()
                    .text_size(text::PROSE)
                    .line_height(gpui_kit::relative(1.6))
                    .text_color(p.text_muted)
                    .child(g.description),
            );
        let mut notices: Vec<AnyElement> = Vec::new();
        if let Some((_, msg)) = &self.failed {
            let body = format!(
                "{}. Retry, or dismiss to put the last saved value back.",
                msg.trim_end_matches('.')
            );
            notices.push(
                div()
                    .max_w(px(PAGE_MAX_W))
                    .mb(space::S5)
                    .child(notice::notice(
                        Tone::Danger,
                        icons::ALERT,
                        "Couldn\u{2019}t save recording settings",
                        Some(body.into()),
                        vec![
                            Button::new("settings-notice-retry", "Retry")
                                .sm()
                                .icon(icons::RETRY)
                                .on_click(super::click(cx, |s, cx| s.retry(cx)))
                                .into_any_element(),
                            Button::new("settings-notice-dismiss", "Dismiss")
                                .ghost()
                                .sm()
                                .on_click(super::click(cx, |s, cx| s.dismiss_failure(cx)))
                                .into_any_element(),
                        ],
                        &p,
                    ))
                    .into_any_element(),
            );
        }
        if let Some(note) = super::commands::note(self, &p, cx) {
            notices.push(
                div()
                    .max_w(px(PAGE_MAX_W))
                    .mb(space::S5)
                    .child(note)
                    .into_any_element(),
            );
        }
        if let Some(err) = &self.data.load_error {
            let body = format!(
                "{}. Capture, privacy and storage show their defaults until it loads.",
                err.trim_end_matches('.')
            );
            notices.push(
                div()
                    .max_w(px(PAGE_MAX_W))
                    .mb(space::S5)
                    .child(notice::notice(
                        Tone::Danger,
                        icons::ALERT,
                        "Couldn\u{2019}t load recording settings",
                        Some(body.into()),
                        Vec::new(),
                        &p,
                    ))
                    .into_any_element(),
            );
        }

        let blocks = sections::render(self, window, cx);
        self.item_base = 1 + notices.len();
        self.items = blocks.iter().map(|(id, _)| *id).collect();
        let last = blocks.len().saturating_sub(1);
        let column = div()
            .id("settings-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(space::S4)
            // `.settings-scroll`: padding-inline S5 on a window ≤800 px wide.
            .px(
                if window.viewport_size().width <= breakpoint::SETTINGS_TIGHT {
                    space::S5
                } else {
                    space::S7
                },
            )
            .on_scroll(cx.listener(|this, _: &ScrollEvent, _, cx| {
                this.spy_suppressed = false;
                cx.notify();
            }))
            .child(head)
            .children(notices)
            .children(blocks.into_iter().enumerate().map(|(ix, (_, el))| {
                div()
                    .max_w(px(PAGE_MAX_W))
                    .when(ix < last, |d| d.pb(space::S7))
                    .when(ix == last, |d| d.pb(space::S8))
                    .child(el)
            }));

        div()
            .id("settings")
            .capture_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, _, cx| {
                if this.ui.kb_listening.is_some() && sections::capture_key(this, &e.keystroke, cx) {
                    cx.stop_propagation();
                }
            }))
            .flex()
            .size_full()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family(FONT_UI)
            .text_size(text::MD)
            .child(rail::render(self, window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(bar::render(self, scrolled, cx))
                    .child(column),
            )
            .children(sections::overlays(self, window, cx))
            .children(super::commands::app_picker(self, &p, cx))
    }
}
