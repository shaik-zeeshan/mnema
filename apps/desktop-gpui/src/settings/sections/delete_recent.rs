//! Data › Storage › Delete Recent Capture (`DeleteRecentCapture.svelte`): one
//! danger-tinted box with the window and the delete button (asks first).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use crate::settings::{click, confirm, icons, on, Settings, DELETE_WINDOWS};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::segmented::Segmented;
use crate::ui::Button;

/// `deleteWindowPhrase`.
pub fn phrase(seconds: u64) -> String {
    let m = seconds / 60;
    format!("last {m} minute{}", if m == 1 { "" } else { "s" })
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let seconds = DELETE_WINDOWS[s.ui.delete_window];
    let copy = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w(px(240.))
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.danger_text)
                .child("Delete Recent Capture"),
        )
        .child(
            div()
                .text_size(text::BASE)
                .line_height(gpui_kit::relative(1.5))
                .text_color(p.text_muted)
                .child("Permanently removes every segment that overlaps the window \u{2014} screen, audio, OCR text, and transcripts. Can\u{2019}t be undone."),
        );
    let ctl = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(
            Segmented::new("delete-window", "Delete window", DELETE_WINDOWS.iter().map(|s| format!("{} min", s / 60)), s.ui.delete_window)
                .md()
                .mono()
                .on_change(on(cx, |s, i: usize, cx| {
                    s.ui.delete_window = i;
                    cx.notify();
                })),
        )
        .child(
            Button::new("delete-recent", format!("Delete last {} min\u{2026}", seconds / 60))
                .danger()
                .icon(icons::TRASH)
                .on_click(click(cx, move |_, cx| {
                    let message = format!(
                        "Delete the {} from Mnema's library? This removes whole overlapping capture segments and cannot be undone.",
                        phrase(seconds)
                    );
                    // shortcut: delete_recent_capture finalizes the live segment through
                    // native_capture (src-tauri) until S3; the confirm runs, the delete is logged.
                    confirm(cx, "Delete Recent Capture", &message, "Delete", |_, _| {
                        log::info!("settings: delete recent capture requested (lands with S3)")
                    });
                })),
        );
    div()
        .px(px(20.))
        .rounded(radius::LG)
        .border_1()
        .border_color(p.danger_border)
        .bg(p.danger_bg_soft)
        .child(div().flex().flex_wrap().items_center().gap(space::S6).py(px(14.)).child(copy).child(ctl))
        .child(
            div()
                .pb(space::S3)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child("Also in the menu bar \u{b7} Retention cleanup is separate and never touches User Context."),
        )
        .into_any_element()
}
