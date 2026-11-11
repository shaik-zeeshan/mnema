//! Data › Storage › Delete Recent Capture (`DeleteRecentCapture.svelte`): one
//! danger-tinted box with the window and the delete button (asks first).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use crate::settings::row::line;
use crate::settings::{click, confirm, icons, on, Settings, DELETE_WINDOWS};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::segmented::Segmented;
use crate::ui::{Button, Tone};

/// `deleteWindowPhrase`.
pub fn phrase(seconds: u64) -> String {
    let m = seconds / 60;
    format!("last {m} minute{}", if m == 1 { "" } else { "s" })
}

const DELETING: &str = "Delete Recent Capture";

/// `deleteReceipt`: the one line under the row once it ran.
pub fn receipt(seconds: u64, file_errors: i64) -> (Tone, String) {
    let done = format!("Deleted the {}", phrase(seconds));
    if file_errors <= 0 {
        return (Tone::Ok, format!("{done}."));
    }
    let files = format!(
        "{file_errors} file{}",
        if file_errors == 1 { "" } else { "s" }
    );
    (
        Tone::Warn,
        format!("{done}, but {files} couldn\u{2019}t be removed from disk yet \u{2014} Mnema will retry."),
    )
}

impl Settings {
    /// Asks with the tray's words, then deletes; the receipt says what happened.
    fn delete_recent(&mut self, seconds: u64, cx: &mut Context<Self>) {
        let message = format!(
            "Delete the {} from Mnema's library? This removes whole overlapping capture segments and cannot be undone.",
            phrase(seconds)
        );
        confirm(
            cx,
            "Delete Recent Capture",
            &message,
            "Delete",
            move |s, cx| {
                let Some(core) = crate::chat::core(cx) else {
                    return;
                };
                if !s.ui.busy.insert(DELETING) {
                    return;
                }
                s.ui.delete_receipt = None;
                let job = cx.background_spawn(async move {
                    crate::settings::live::delete_recent(&core, seconds)
                });
                cx.spawn(async move |this, cx| {
                    let result = job.await;
                    let _ = this.update(cx, |s, cx| {
                        s.ui.busy.remove(DELETING);
                        s.ui.delete_receipt = Some(match result {
                            Ok(errors) => receipt(seconds, errors),
                            Err(e) => {
                                log::warn!("settings: delete recent capture failed: {e}");
                                (
                                    Tone::Danger,
                                    format!("Couldn\u{2019}t delete the {}: {e}", phrase(seconds)),
                                )
                            }
                        });
                        s.refresh(cx);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            },
        );
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let seconds = DELETE_WINDOWS[s.ui.delete_window];
    let busy = s.ui.busy.contains(DELETING);
    let copy = div()
        .flex()
        .flex_col()
        .gap(px(3.))
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
            Segmented::new(
                "delete-window",
                "Delete window",
                DELETE_WINDOWS.iter().map(|s| format!("{} min", s / 60)),
                s.ui.delete_window,
            )
            .md()
            .mono()
            .disabled(busy)
            .on_change(on(cx, |s, i: usize, cx| {
                s.ui.delete_window = i;
                cx.notify();
            })),
        )
        .child(
            Button::new(
                "delete-recent",
                if busy {
                    "Deleting\u{2026}".to_string()
                } else {
                    format!("Delete last {} min\u{2026}", seconds / 60)
                },
            )
            .danger()
            .when(!busy, |b| b.icon(icons::TRASH))
            .busy(busy)
            .disabled(busy)
            .on_click(click(cx, move |s, cx| s.delete_recent(seconds, cx))),
        );
    div()
        .px(px(20.))
        .rounded(radius::LG)
        .border_1()
        .border_color(p.danger_border)
        .bg(p.danger_bg_soft)
        .child(
            // `.st-danger__row`: copy `flex: 1 1 240px`, the controls right-aligned
            // and dropping to their own line before the copy squeezes.
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(space::S6)
                .gap_y(space::S3)
                .py(px(14.))
                .child(copy.flex_grow(1.).flex_shrink(1.).flex_basis(px(240.)).min_w_0())
                .child(ctl.flex_none().flex_wrap().justify_end().max_w_full().ml_auto()),
        )
        .children(s.ui.delete_receipt.clone().map(|(tone, words)| {
            let ic = if tone == Tone::Ok { icons::CHECK } else { icons::ALERT };
            div().pb(space::S2).child(line(tone, ic, words, p))
        }))
        .child(
            div()
                .pb(space::S3)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child("Also in the menu bar \u{b7} Retention cleanup is separate and never touches User Context."),
        )
        .into_any_element()
}
