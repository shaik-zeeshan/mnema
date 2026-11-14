//! Feedback (index.html "Feedback parts"): notice, banner, inline, empty; and
//! loading: skeletons, spinners, progress.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Window};

use super::{cell, panel, row, Gallery};
use crate::theme::{self, size, space, text, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::notice::{banner, inline, notice, strong};
use crate::ui::progress::{progress, progress_meta, spinner};
use crate::ui::{icon, skeleton as sk, Button, Tone};

pub(super) fn render(_: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let btn = |id: &'static str, label: &'static str| Button::new(id, label).sm();
    let notices = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .w(px(620.))
        .child(notice(
            Tone::Off,
            IconName::CircleOff.path(),
            "Summaries are off",
            Some("Turn on day summaries to see where the time went.".into()),
            vec![btn("n-0", "Turn on in Settings").ghost().icon(IconName::ArrowRight.path()).icon_after().into_any_element()],
            &p,
        ))
        .child(notice(
            Tone::Warn,
            IconName::Lock.path(),
            "Screen Recording is off for Mnema",
            Some("macOS blocks screen capture until you allow it.".into()),
            vec![btn("n-1", "Open System Settings").icon(IconName::ExternalLink.path()).icon_after().into_any_element()],
            &p,
        ))
        .child(notice(
            Tone::Warn,
            IconName::CloudOff.path(),
            "Can’t reach ChatGPT right now",
            Some("Answers resume on their own. You don’t need to sign in again.".into()),
            vec![btn("n-2", "Retry").ghost().icon(IconName::RotateCcw.path()).into_any_element()],
            &p,
        ))
        .child(notice(
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "ChatGPT sign-in expired",
            Some("Sign in again to keep summaries and Ask running.".into()),
            vec![btn("n-3", "Reconnect").into_any_element()],
            &p,
        ))
        .child(notice(
            Tone::Info,
            IconName::Info.path(),
            "No system sound yet",
            Some("If nothing has played, ignore this. Otherwise check Screen & System Audio Recording.".into()),
            vec![btn("n-4", "Dismiss").ghost().into_any_element()],
            &p,
        ));
    let msg = |parts: Vec<AnyElement>| {
        div()
            .flex()
            .flex_wrap()
            .gap(px(4.))
            .children(parts)
            .into_any_element()
    };
    let banners = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .w(px(720.))
        .child(banner(
            Tone::Info,
            msg(vec![
                strong("Trial", &p).into_any_element(),
                "· 9 days left. Everything works until then.".into_any_element(),
            ]),
            vec![btn("b-0", "Buy Mnema").primary().into_any_element()],
            &p,
        ))
        .child(banner(
            Tone::Warn,
            msg(vec![
                strong("Low disk", &p).into_any_element(),
                "· 4.1 GB free. Recording pauses at 2 GB.".into_any_element(),
            ]),
            vec![btn("b-1", "Free up space").into_any_element()],
            &p,
        ))
        .child(banner(
            Tone::Danger,
            msg(vec![
                strong("Read-only", &p).into_any_element(),
                "· the trial ended. Your history stays searchable.".into_any_element(),
            ]),
            vec![btn("b-2", "Activate").into_any_element()],
            &p,
        ))
        .child(banner(
            Tone::Neutral,
            msg(vec!["Updating the search index…".into_any_element()]),
            vec![],
            &p,
        ));
    let ic = |path| icon(path, size::BTN_ICON_SM).into_any_element();
    let inlines = div()
        .flex()
        .flex_col()
        .gap(space::S2)
        .child(inline(
            Tone::Ok,
            Some(ic(IconName::Check.path())),
            vec!["Saved".into_any_element()],
            &p,
        ))
        .child(inline(
            Tone::Info,
            Some(ic(IconName::Info.path())),
            vec!["Takes effect on the next recording.".into_any_element()],
            &p,
        ))
        .child(inline(
            Tone::Warn,
            Some(ic(IconName::Download.path())),
            vec![
                "Model not downloaded ·".into_any_element(),
                div()
                    .font_family(FONT_MONO)
                    .child("419 MB")
                    .into_any_element(),
                Button::new("i-0", "Download")
                    .ghost()
                    .inline()
                    .into_any_element(),
            ],
            &p,
        ))
        .child(inline(
            Tone::Danger,
            Some(ic(IconName::TriangleAlert.path())),
            vec![
                "OpenAI rejected this key.".into_any_element(),
                Button::new("i-1", "Retry")
                    .ghost()
                    .inline()
                    .into_any_element(),
            ],
            &p,
        ))
        .child(inline(
            Tone::Neutral,
            Some(spinner(size::SPIN_SM).into_any_element()),
            vec!["Checking the key…".into_any_element()],
            &p,
        ));
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(row(
            "notice",
            vec![cell("off · warn · warn · danger · info", notices, &p)],
            &p,
        ))
        .child(row(
            "banner",
            vec![cell("info · warn · danger · neutral", banners, &p)],
            &p,
        ))
        .child(row(
            "inline",
            vec![cell("ok · info · warn · danger · busy", inlines, &p)],
            &p,
        ))
        .into_any_element()
}

pub(super) fn empties(_: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let btn = |id: &'static str, label: &'static str| Button::new(id, label).sm();
    let empties = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .w(px(620.))
        .child(panel(&p).bg(p.surface_subtle).child(empty(
            EmptyLayout::Panel,
            Tone::Neutral,
            IconName::Inbox.path(),
            "Nothing captured yet today",
            "Start recording and this fills in as you work. Nothing leaves your Mac.",
            vec![btn("e-0", "Start recording").into_any_element()],
            &p,
        )))
        .child(panel(&p).child(empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            IconName::Search.path(),
            "No moments match “arc pricing”",
            "Try a wider range, or ask instead.",
            vec![btn("e-1", "Any time").ghost().into_any_element()],
            &p,
        )))
        .child(panel(&p).child(empty(
            EmptyLayout::Compact,
            Tone::Danger,
            IconName::TriangleAlert.path(),
            "Couldn’t load this week",
            "The database was busy. Nothing is lost.",
            vec![btn("e-2", "Retry").icon(IconName::RotateCcw.path()).into_any_element()],
            &p,
        )))
        .child(panel(&p).child(empty(
            EmptyLayout::Compact,
            Tone::Warn,
            IconName::CloudOff.path(),
            "Can’t reach ChatGPT",
            "Summaries resume on their own — your sign-in is fine.",
            vec![btn("e-3", "Try now").ghost().into_any_element()],
            &p,
        )))
        .child(panel(&p).child(empty(
            EmptyLayout::Center,
            Tone::Neutral,
            IconName::Bell.path(),
            "You’re all caught up",
            "Capture, model and update warnings land here.",
            vec![],
            &p,
        )));
    row(
        "empty",
        vec![cell(
            "panel · compact · danger · warn · center",
            empties,
            &p,
        )],
        &p,
    )
    .into_any_element()
}

pub(super) fn loading(_: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let summary = panel(&p)
        .w(px(300.))
        .p(space::S4)
        .child(sk::title(&p).mb(px(14.)))
        .child(sk::lines(3, &p));
    let rows = div()
        .w(px(300.))
        .child(sk::row(false, 0.62, false, &p))
        .child(sk::row(true, 0.48, true, &p));
    let thumbs = div()
        .flex()
        .gap(space::S2)
        .w(px(300.))
        .children((0..3).map(|_| div().flex_1().child(sk::thumb(&p))));
    let chart = div().w(px(300.)).child(sk::chart(14, &p));
    let spins = div()
        .flex()
        .items_center()
        .gap(space::S3)
        .child(
            Button::new("l-0", "Installing…")
                .primary()
                .sm()
                .busy(true)
                .spinner(true),
        )
        .child(
            Button::new("l-1", "Starting…")
                .sm()
                .busy(true)
                .spinner(true),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(p.text_subtle)
                .child(spinner(size::SPIN))
                .child("14"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(p.text_subtle)
                .child(spinner(size::SPIN_LG))
                .child("20"),
        );
    let bars = div()
        .flex()
        .flex_col()
        .gap(space::S4)
        .w(px(420.))
        .child(
            div()
                .child(progress_meta(
                    "Downloading Parakeet",
                    "312 / 800 MB · 2 min left",
                    &p,
                ))
                .child(progress(Some(0.39), &p)),
        )
        .child(
            div()
                .child(progress_meta(
                    "Rebuilding the search index",
                    "this can take a few minutes",
                    &p,
                ))
                .child(progress(None, &p)),
        );
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .text_size(text::BASE)
        .child(
            row(
                "skeleton",
                vec![cell("title + lines", summary, &p), cell("rows", rows, &p)],
                &p,
            )
            .items_start(),
        )
        .child(
            row(
                "",
                vec![cell("thumbs", thumbs, &p), cell("chart", chart, &p)],
                &p,
            )
            .items_start(),
        )
        .child(row(
            "spinner",
            vec![cell("in buttons · 14 · 20", spins, &p)],
            &p,
        ))
        .child(row(
            "progress",
            vec![cell("determinate · indeterminate", bars, &p)],
            &p,
        ))
        .into_any_element()
}
