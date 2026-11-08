//! Choice (tiles + list), Select / Combobox, Date input.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, App, Context, Window};
use time::Date;

use super::{cell, row, Gallery};
use crate::theme::{self, space};
use crate::ui::calendar::Unit;
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::combobox::Combobox;
use crate::ui::date_input::DateInput;
use crate::ui::select::Select;
use crate::ui::Force;

fn vad() -> Vec<ChoiceOption> {
    vec![
        ChoiceOption::new("Silero")
            .tag("default")
            .icon(IconName::AudioWaveform.path())
            .desc("Neural speech detector. Falls back to WebRTC."),
        ChoiceOption::new("WebRTC")
            .icon(IconName::Feather.path())
            .desc("Lightweight local detector, lowest CPU."),
        ChoiceOption::new("Off")
            .icon(IconName::CircleOff.path())
            .desc("Use microphone peak level only."),
    ]
}

fn asr() -> Vec<ChoiceOption> {
    vec![
        ChoiceOption::new("Parakeet")
            .tag("on-device · 800 MB")
            .icon(IconName::Cpu.path())
            .desc("Fast English transcription on the Neural Engine."),
        ChoiceOption::new("Whisper")
            .tag("on-device · 1.4 GB")
            .icon(IconName::Cpu.path())
            .desc("Multilingual; slower and heavier on memory."),
        ChoiceOption::new("Deepgram")
            .tag("cloud")
            .icon(IconName::Cloud.path())
            .desc("Uploads mic + system audio to your own Deepgram account."),
    ]
}

pub(super) fn choice(g: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let this = cx.weak_entity();
    let pick = |k: usize| {
        let this = this.clone();
        move |ix: usize, _: &mut Window, cx: &mut App| {
            this.update(cx, |g, cx| {
                g.choice[k] = ix;
                cx.notify();
            })
            .ok();
        }
    };
    let mut off = vad();
    off[1].disabled = true;
    div()
        .flex()
        .flex_col()
        .gap(space::S4)
        .child(cell(
            "tiles · selected · hover (2)",
            // Tiles share a definite width, as in a Settings pane.
            div().w(px(744.)).child(
                Choice::new("ch-tiles", "Microphone voice detection", vad(), g.choice[0])
                    .force(1, Force::Hover)
                    .on_change(pick(0)),
            ),
            &p,
        ))
        .child(cell(
            "tiles · focus · disabled (2)",
            div().w(px(744.)).child(
                Choice::new("ch-tiles-f", "Microphone voice detection", off, 0)
                    .force(2, Force::Focus),
            ),
            &p,
        ))
        .child(
            div()
                .flex()
                .gap(space::S3)
                .child(cell(
                    "list · selected (2) · hover (1)",
                    div().w(px(366.)).child(
                        Choice::new("ch-list", "Transcription provider", asr(), g.choice[1])
                            .list()
                            .force(0, Force::Hover)
                            .on_change(pick(1)),
                    ),
                    &p,
                ))
                .child(cell(
                    "list · keyboard focus (3)",
                    div().w(px(366.)).child(
                        Choice::new("ch-list-f", "Transcription provider", asr(), 1)
                            .list()
                            .force(2, Force::Focus),
                    ),
                    &p,
                )),
        )
        .into_any_element()
}

const MODELS: [&str; 6] = [
    "Claude Sonnet 4.5",
    "GPT-5",
    "Gemini 2.5 Pro",
    "Llama 3.3 70B",
    "Qwen 3 32B",
    "DeepSeek V3",
];

pub(super) fn select(g: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let this = cx.weak_entity();
    let (sel, combo) = (this.clone(), this);
    let w = px(150.);
    let opts = ["Apple Vision", "Tesseract", "PaddleOCR"];
    let closed = vec![
        cell(
            "value",
            div().w(w).child(
                Select::new("sel-0", "OCR engine", opts, g.select).on_change(move |i, _, cx| {
                    sel.update(cx, |g, cx| {
                        g.select = Some(i);
                        cx.notify();
                    })
                    .ok();
                }),
            ),
            &p,
        ),
        cell(
            "placeholder",
            div().w(w).child(
                Select::new("sel-1", "OCR engine", opts, None).placeholder("Choose an engine"),
            ),
            &p,
        ),
        cell(
            "hover",
            div()
                .w(w)
                .child(Select::new("sel-2", "OCR engine", opts, Some(0)).force(Force::Hover)),
            &p,
        ),
        cell(
            "focus",
            div()
                .w(w)
                .child(Select::new("sel-3", "OCR engine", opts, Some(0)).force(Force::Focus)),
            &p,
        ),
    ];
    let closed2 = vec![
        cell(
            "warn",
            div().w(w).child(
                Select::new("sel-4", "Model", MODELS, None)
                    .placeholder("Model unavailable")
                    .warn(true),
            ),
            &p,
        ),
        cell(
            "disabled",
            div()
                .w(w)
                .child(Select::new("sel-5", "OCR engine", opts, Some(0)).disabled(true)),
            &p,
        ),
        cell(
            "busy",
            div().w(w).child(
                Select::new("sel-6", "Action", opts, None)
                    .placeholder("Applying…")
                    .busy(true),
            ),
            &p,
        ),
        cell(
            "compact",
            div().w(px(150.)).child(
                Select::new("sel-7", "Action", opts, None)
                    .placeholder("More…")
                    .compact(),
            ),
            &p,
        ),
    ];
    let combo_row = vec![
        cell(
            "combobox",
            div().w(px(200.)).child(
                Combobox::new("combo-0", "Model", MODELS, g.combo).on_change(move |i, _, cx| {
                    combo
                        .update(cx, |g, cx| {
                            g.combo = Some(i);
                            cx.notify();
                        })
                        .ok();
                }),
            ),
            &p,
        ),
        cell(
            "combobox · focus",
            div().w(px(200.)).child(
                Combobox::new("combo-f", "Model", MODELS, None)
                    .placeholder("Choose a model")
                    .force(Force::Focus),
            ),
            &p,
        ),
    ];
    let open = vec![
        cell(
            "select open",
            div()
                .w(px(190.))
                .h(px(170.))
                .child(Select::new("sel-open", "OCR engine", opts, Some(1)).force_open()),
            &p,
        ),
        cell(
            "combobox open · \"g\"",
            div()
                .w(px(210.))
                .h(px(170.))
                .child(Combobox::new("combo-open", "Model", MODELS, Some(1)).force_open("g")),
            &p,
        ),
        cell(
            "combobox · no matches",
            div()
                .w(px(190.))
                .h(px(170.))
                .child(Combobox::new("combo-none", "Model", MODELS, None).force_open("zzz")),
            &p,
        ),
    ];
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(row("select", closed, &p))
        .child(row("states", closed2, &p))
        .child(row("combobox", combo_row, &p))
        .child(row("open", open, &p).items_start())
        .into_any_element()
}

/// A stable fake capture density for the ruler and calendar dots.
fn density(d: Date) -> u8 {
    (d.to_julian_day().rem_euclid(7) as u8 * 3 + d.day()) % 4
}

pub(super) fn date(g: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let this = cx.weak_entity();
    let set = move |kind: u8| {
        let this = this.clone();
        move |s: Date, _: Date, _: Option<Unit>, _: &mut Window, cx: &mut App| {
            this.update(cx, |g, cx| {
                match kind {
                    0 => g.day = s,
                    1 => g.week = s,
                    _ => g.month = s,
                }
                cx.notify();
            })
            .ok();
        }
    };
    let today = g.today;
    let min = today - time::Duration::days(80);
    let day = |id: &str| {
        DateInput::new(id.to_string(), g.day, today)
            .range(min, today)
            .density(density)
    };
    let capsules = vec![
        cell("day", day("date-day").on_change(set(0)), &p),
        cell("hover", day("date-hover").force(Force::Hover), &p),
        cell("focus", day("date-focus").force(Force::Focus), &p),
    ];
    let periods = vec![
        cell(
            "period · week",
            DateInput::new("date-week", g.week, today)
                .range(min, today)
                .period(Unit::Week)
                .density(density)
                .on_change(set(1)),
            &p,
        ),
        cell(
            "period · month",
            DateInput::new("date-month", g.month, today)
                .range(min, today)
                .period(Unit::Month)
                .density(density)
                .on_change(set(2)),
            &p,
        ),
    ];
    let open = vec![
        cell(
            "day · open",
            div()
                .w(px(300.))
                .h(px(340.))
                .child(day("date-open").force_open()),
            &p,
        ),
        cell(
            "period · week · open",
            div().w(px(340.)).h(px(340.)).child(
                DateInput::new("date-week-open", g.week, today)
                    .range(min, today)
                    .period(Unit::Week)
                    .density(density)
                    .force_open(),
            ),
            &p,
        ),
    ];
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(row("capsule", capsules, &p))
        .child(row("period", periods, &p))
        .child(row("calendar", open, &p).items_start())
        .into_any_element()
}
