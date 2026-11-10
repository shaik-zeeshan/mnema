//! "What Mnema is doing" (`StatusPop.svelte`, 340 px, opens upward from the REC
//! pill, the lane group or the disk-soon flag): capture now, the backlog
//! ("Catching up"), free space, and the engine when unhealthy. The words come
//! from `pop_rows.rs` (`status-pop.ts`).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, Context, Hsla, Pixels, SharedString};

use super::pop_rows::{self, CaptureGroup, CatchingUp, PopRow, RowTone};
use super::Shell;
use crate::keymap;
use crate::theme::{self, size, space, text, Palette};
use crate::ui::{icon, kbd, kicker, pop_in, popover, progress, Button};

/// The leading dot's width; the meaning-index bar insets by it plus the gap so
/// its label lines up with the rows' label column.
const DOT: Pixels = px(7.);

fn row(r: &PopRow, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let tone: Hsla = match r.tone {
        Some(RowTone::Ok) => p.accent,
        Some(RowTone::Warn) => p.warn,
        Some(RowTone::Danger) => p.danger,
        None => p.text_faint,
    };
    let dot = div()
        .flex_none()
        .mt(px(5.))
        .size(DOT)
        .rounded_full()
        .map(|d| match r.ring {
            true => d.border(px(1.5)).border_color(p.text_muted),
            false => d.bg(tone),
        });
    let action = r.action.map(|(label, tab)| {
        let shell = cx.entity().downgrade();
        div().mt(px(2.)).ml(px(-6.)).child(
            Button::new(SharedString::from(format!("status-pop-{tab}")), label)
                .ghost()
                .sm()
                .h(size::H_MINI)
                .on_click(move |_, _, cx| {
                    let _ = shell.update(cx, |s, cx| {
                        s.overlay = None;
                        cx.notify();
                    });
                    cx.defer(move |cx| crate::window::open_settings(Some(tab), cx));
                }),
        )
    });
    div()
        .flex()
        .gap(space::S2)
        .text_size(text::BASE)
        .line_height(relative(1.4))
        .child(dot)
        .child(
            div()
                .flex_none()
                .w(px(92.))
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_strong)
                .child(r.name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .text_color(if r.tone == Some(RowTone::Warn) {
                    p.text
                } else {
                    p.text_muted
                })
                .child(r.value.clone())
                .children(
                    r.note
                        .clone()
                        .map(|n| div().text_color(p.text_muted).child(n)),
                )
                .children(action),
        )
}

fn rows(list: &[PopRow], p: &Palette, cx: &mut Context<Shell>) -> gpui_kit::Div {
    div().flex().flex_col().gap(px(7.)).children(
        list.iter()
            .map(|r| row(r, p, cx).into_any_element())
            .collect::<Vec<_>>(),
    )
}

fn group(label: &'static str, p: &Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(space::S4)
        .py(space::S2)
        .border_t_1()
        .border_color(p.hairline)
        .child(
            div()
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_subtle)
                .child(label),
        )
}

pub fn render(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let m = &shell.model.status;
    let capture = match pop_rows::capture_group(m) {
        CaptureGroup::Rows(list) if list.is_empty() => None,
        CaptureGroup::Rows(list) => Some(group("Capture", p).child(rows(&list, p, cx))),
        CaptureGroup::Starting => {
            Some(group("Capture", p).child(div().text_color(p.text_subtle).child("Starting…")))
        }
        CaptureGroup::Stopped => Some(
            group("Capture", p).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(p.text_subtle)
                    .child("Not recording. Press Record or")
                    .child(kbd(keymap::display(&shell.bindings.toggle_recording), p))
                    .child("."),
            ),
        ),
    };
    let catching = pop_rows::catching_up(m, shell.model.backlog.as_ref()).map(|c| match c {
        CatchingUp::CaughtUp => group("Catching up", p).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(p.text_subtle)
                .child(icon(IconName::Check.path(), px(13.)))
                .child("All caught up"),
        ),
        CatchingUp::Busy {
            rows: list,
            meaning_percent,
        } => group("Catching up", p)
            .when(!list.is_empty(), |g| g.child(rows(&list, p, cx)))
            .children(meaning_percent.map(|pct| {
                let pct = pct.round();
                // Rows sit at line-height 1.4 and this label at 1, so the extra
                // S1 above and below matches the gap between two rows.
                div()
                    .pt(space::S1)
                    .pb(space::S1)
                    .pl(DOT + space::S2)
                    .child(
                        progress::progress_meta("Building meaning index", format!("{pct}%"), p)
                            .mb(space::S2),
                    )
                    .child(progress::progress(Some(pct as f32 / 100.), p))
            })),
    });
    let disk = pop_rows::disk_row(m).map(|r| group("Disk", p).child(rows(&[r], p, cx)));
    let ai = pop_rows::ai_row(m).map(|r| group("AI", p).child(rows(&[r], p, cx)));
    pop_in(
        popover("status-popover", "What Mnema is doing", p)
            .left(space::S2)
            .w(theme::size::POP_STATUS_W)
            .pb(space::S2)
            .child(
                div()
                    .pt(space::S3)
                    .px(space::S4)
                    .pb(space::S2)
                    .child(kicker("What Mnema is doing", p)),
            )
            .children(capture)
            .children(catching)
            .children(disk)
            .children(ai),
        theme::size::STATUS_BAR + theme::size::POP_GAP,
        true,
    )
}
