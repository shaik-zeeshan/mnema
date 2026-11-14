//! `.mx-chip` (status, never an action: a clickable chip is `Button::sm`) and
//! `.mx-badge` (the notification dot on an icon button).

use gpui_kit::prelude::*;
use gpui_kit::{div, px, Div, Hsla, SharedString};

use super::{mix, spread};
use crate::theme::{self, radius, size, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::Tracking as _;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChipKind {
    Neutral,
    Screen,
    Mic,
    SysAudio,
    Accent,
    Warn,
    Danger,
    Info,
    /// `.mx-chip--cat` with `--c` = the category colour.
    Category(Hsla),
}

pub fn chip(label: impl Into<SharedString>, kind: ChipKind, p: &Palette) -> Div {
    let (fg, bg, bd) = match kind {
        ChipKind::Neutral => (p.text_muted, p.surface_subtle, p.border),
        ChipKind::Screen => (p.source_screen, p.source_screen_bg, p.source_screen_border),
        ChipKind::Mic => (p.source_mic, p.source_mic_bg, p.source_mic_border),
        ChipKind::SysAudio => (
            p.source_sysaudio,
            p.source_sysaudio_bg,
            p.source_sysaudio_border,
        ),
        ChipKind::Accent => (p.accent, p.accent_bg, p.accent_border),
        ChipKind::Warn => (p.warn, p.warn_bg, p.warn_border),
        ChipKind::Danger => (p.danger_text, p.danger_bg, p.danger_border),
        ChipKind::Info => (p.info, p.info_bg, p.info_border),
        ChipKind::Category(c) => (p.text, mix(c, p.surface, 0.09), mix(c, p.border, 0.30)),
    };
    let cat = matches!(kind, ChipKind::Category(_));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .h(size::CHIP_H)
        .px(px(7.))
        .rounded(radius::XS)
        .border_1()
        .border_color(bd)
        .bg(bg)
        .text_color(fg)
        .font_weight(theme::W_MEDIUM)
        .line_height(gpui_kit::relative(1.))
        .whitespace_nowrap()
        .tracking(if cat { 0. } else { 0.02 })
        .when(!cat, |d| d.font_family(FONT_MONO).text_size(text::XS))
        .when_some(
            match kind {
                ChipKind::Category(c) => Some(c),
                _ => None,
            },
            |d, c| {
                d.font_family(FONT_UI)
                    .text_size(text::SM)
                    .child(div().flex_none().size(px(6.)).rounded(px(2.)).bg(c))
            },
        )
        .child(label.into())
}

/// A non-category chip as a `baseline::row` item: its one centred line of text
/// sits like a single line whose line box is the whole chip.
pub fn chip_line() -> super::baseline::Line {
    super::baseline::Line::new(
        FONT_MONO,
        text::XS,
        f32::from(size::CHIP_H) / f32::from(text::XS),
    )
}

/// The badge dot: put it inside a `relative()` icon button. Colour = the worst
/// unread severity (info = accent). No notifications = no badge.
pub fn badge(tone: super::Tone, p: &Palette) -> Div {
    let c = match tone {
        super::Tone::Warn => p.warn,
        super::Tone::Danger => p.danger,
        _ => p.accent,
    };
    div()
        .absolute()
        .top(px(5.))
        .right(px(5.))
        .size(px(6.))
        .rounded_full()
        .bg(c)
        .shadow(vec![spread(p.bg, px(2.))])
}
