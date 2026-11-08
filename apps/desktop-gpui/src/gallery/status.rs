//! Chips, the notification badge, kbd keys and the small-window strip.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Window};

use super::{cell, panel, row, Gallery};
use crate::theme::{self, size, space};
use crate::ui::chip::{badge, chip, ChipKind};
use crate::ui::window_strip::WindowStrip;
use crate::ui::{icon_button, kbd, keys, Button, Force, Tone};

pub(super) fn render(_: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let kinds = [
        ("screen", ChipKind::Screen),
        ("mic", ChipKind::Mic),
        ("system audio", ChipKind::SysAudio),
        ("live", ChipKind::Accent),
        ("paused", ChipKind::Warn),
        ("failed", ChipKind::Danger),
        ("syncing", ChipKind::Info),
        ("neutral", ChipKind::Neutral),
    ];
    let cats = [
        ("Creating", p.cat_creating),
        ("Communication", p.cat_communication),
        ("Meetings", p.cat_meetings),
        ("Research", p.cat_research),
        ("Learning", p.cat_learning),
        ("Entertainment", p.cat_entertainment),
    ];
    let chips = div()
        .flex()
        .flex_wrap()
        .gap(space::S2)
        .children(kinds.map(|(l, k)| chip(l, k, &p)));
    let cat_chips = div()
        .flex()
        .flex_wrap()
        .gap(space::S2)
        .children(cats.map(|(l, c)| chip(l, ChipKind::Category(c), &p)));
    let bell = |id: &'static str, tone: Tone| {
        icon_button(id, "Notifications", px(28.), false, &p)
            .child(crate::ui::icon(IconName::Bell.path(), size::ICON))
            .child(badge(tone, &p))
    };
    let badges = div()
        .flex()
        .gap(space::S3)
        .child(bell("badge-info", Tone::Info))
        .child(bell("badge-warn", Tone::Warn))
        .child(bell("badge-danger", Tone::Danger));
    let kbds = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(kbd("/", &p))
        .child(keys(&["⌘", "K"], &p))
        .child(keys(&["⌥", "⌘", "Space"], &p));
    let strip = |f: Force, actions: bool| {
        panel(&p).w(px(340.)).overflow_hidden().child(
            WindowStrip::new("Developer")
                .force(f)
                .when(actions, |s| {
                    s.action(
                        Button::icon_only("strip-refresh", IconName::RefreshCw.path(), "Refresh")
                            .ghost()
                            .sm(),
                    )
                })
                .on_close(|_, _| {}),
        )
    };
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(row("chip", vec![cell("sources · tones", chips, &p)], &p))
        .child(row(
            "category",
            vec![cell("--c = category colour", cat_chips, &p)],
            &p,
        ))
        .child(row(
            "badge · keys",
            vec![
                cell("info · warn · danger", badges, &p),
                cell("kbd · chords", kbds, &p),
            ],
            &p,
        ))
        .child(row(
            "strip",
            vec![
                cell("idle", strip(Force::None, false), &p),
                cell("hover · actions", strip(Force::Hover, true), &p),
            ],
            &p,
        ))
        .child(row(
            "",
            vec![cell("close focused", strip(Force::Focus, false), &p)],
            &p,
        ))
        .into_any_element()
}
