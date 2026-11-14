//! The AI tab (`tabs/AiTab.svelte`): engine status, the last Ask AI turn, recent
//! User Context runs and recent CLI requests, each one dense block.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context};

use super::readouts;
use super::tabs::{clock, h, list, sec};
use super::DebugWindow;
use crate::settings::model::Json;
use crate::small::parts::dot;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::Tracking as _;
use crate::ui::{Button, Tone};

pub(super) fn compact_empty(title: &str, failed: bool, p: &Palette) -> AnyElement {
    empty(
        EmptyLayout::Compact,
        if failed { Tone::Danger } else { Tone::Neutral },
        if failed {
            "icons/triangle-alert.svg"
        } else {
            "icons/inbox.svg"
        },
        title.to_string(),
        "",
        vec![],
        p,
    )
    .px(px(0.))
    .py(px(10.))
    .into_any_element()
}

pub fn tab(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> Vec<AnyElement> {
    let d = &w.data;
    let engine = match readouts::engine_view(d.ai_status.as_ref()) {
        Some((tone, title, provider, model_id)) => {
            let sub = match (provider, model_id) {
                (None, None) => None,
                (a, b) => Some(
                    [a, b.map(|m| m.to_string())]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" \u{b7} "),
                ),
            };
            // The engine's trouble, in words, for a bug report.
            let copy_text = match &sub {
                Some(s) => format!("{title} \u{b7} {s}"),
                None => title.clone(),
            };
            div()
                .child(dot(tone, px(7.), p))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(theme::W_MEDIUM)
                                .text_size(text::MD)
                                .line_height(relative(1.3))
                                .text_color(p.text_strong)
                                .child(title),
                        )
                        .children(sub.map(|s| {
                            div()
                                .text_size(text::BASE)
                                .text_color(p.text_subtle)
                                .child(s)
                        })),
                )
                .when(matches!(tone, Tone::Warn | Tone::Danger), |d| {
                    d.child(crate::ui::CopyButton::icon(
                        "dv-ai-engine-copy",
                        "Copy message",
                        copy_text,
                    ))
                })
                .child({
                    let me = cx.entity().downgrade();
                    let testing = w.acting == Some(super::data::Action::TestAi);
                    Button::new(
                        "dv-test-ai",
                        if testing {
                            "Testing\u{2026}"
                        } else {
                            "Test connection"
                        },
                    )
                    .sm()
                    .icon("icons/plug.svg")
                    .spinner(testing)
                    .busy(testing)
                    .disabled(w.acting.is_some())
                    .on_click(move |_, _, cx| {
                        let _ = me.update(cx, |w, cx| w.act(super::data::Action::TestAi, cx));
                    })
                })
        }
        None => div().child(dot(Tone::Off, px(7.), p)).child(
            div()
                .text_size(text::BASE)
                .text_color(p.text_subtle)
                .child("Checking the AI engine\u{2026}"),
        ),
    }
    .flex()
    .items_center()
    .gap(px(10.))
    .px(px(12.))
    .py(px(10.))
    .rounded(radius::MD)
    .border_1()
    .border_color(p.border)
    .bg(p.surface_subtle);

    let usage = d.usage.as_ref().map(|u| {
        let stat = |label: &'static str, (v, unit): (String, &'static str)| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(12.))
                .py(px(10.))
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::SM)
                        .line_height(relative(1.))
                        .text_color(p.text_subtle)
                        .child(label),
                )
                // `.mx-stat__value small`: the unit on the value's baseline.
                .child(
                    baseline::row_top(vec![
                        (
                            Line::new(FONT_MONO, text::LG, 1.),
                            // `.dv-stats .mx-stat__value`.
                            div().text_color(p.text_strong).tracking(-0.03).child(v),
                        ),
                        (
                            Line::new(FONT_UI, text::SM, 1.),
                            div().text_color(p.text_subtle).child(unit),
                        ),
                    ])
                    .gap(px(2.)),
                )
        };
        let n = |k: &str| u.num_at(&[k]).unwrap_or(0.);
        div()
            .flex()
            .rounded(radius::MD)
            .border_1()
            .border_color(p.border)
            .child(stat("Read", readouts::token_stat(n("inputTokens"))))
            .child(
                stat("Wrote", readouts::token_stat(n("outputTokens")))
                    .border_l_1()
                    .border_color(p.hairline),
            )
            .child(
                stat("Tool calls", (format!("{}", n("toolCalls")), ""))
                    .border_l_1()
                    .border_color(p.hairline),
            )
            .child(
                stat("Took", readouts::duration_stat(n("durationMs")))
                    .border_l_1()
                    .border_color(p.hairline),
            )
    });
    let started = d
        .usage
        .as_ref()
        .and_then(|u| u.num_at(&["startedAtMs"]))
        .map(|ms| clock(ms as i64, cx));

    let off = crate::ui::clock::now(cx).offset();
    let now_ms = crate::ui::clock::now_ms(cx);
    let cell = |w: Option<f32>, el: gpui_kit::Div| match w {
        Some(w) => el.flex_none().w(px(w)),
        None => el.flex_1().min_w_0(),
    };
    let mono = |s: String, c| {
        div()
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .text_color(c)
            .whitespace_nowrap()
            .child(s)
    };
    let log_row = |cells: Vec<AnyElement>| {
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .min_h(px(32.))
            .py(px(5.))
            .px(px(2.))
            .border_b_1()
            .border_color(p.hairline)
            .text_size(text::BASE)
            .line_height(relative(1.3))
            .text_color(p.text_muted)
            .children(cells)
            .into_any_element()
    };
    let allowed = d.grants.iter().filter(|g| !g.bool_at(&["blocked"])).count();
    // Newest 4, newest first (the audit file is append-ordered).
    let cli: Vec<AnyElement> = d
        .cli_history
        .iter()
        .rev()
        .take(4)
        .map(|e| {
            let ok = e.str_at(&["outcome"]).is_none_or(|o| o == "success");
            log_row(vec![
                cell(
                    Some(92.),
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text)
                        .whitespace_nowrap(),
                )
                .child(e.str_at(&["toolIdentity"]).unwrap_or("").to_string())
                .into_any_element(),
                cell(
                    Some(64.),
                    mono(
                        e.str_at(&["commandType"]).unwrap_or("").replace('_', "-"),
                        p.text,
                    ),
                )
                .into_any_element(),
                cell(None, div().flex())
                    .child(chip(
                        if ok { "allowed" } else { "refused" },
                        if ok {
                            ChipKind::Neutral
                        } else {
                            ChipKind::Warn
                        },
                        p,
                    ))
                    .into_any_element(),
                mono(
                    e.num_at(&["timestampUnixMs"])
                        .map_or(String::new(), |ms| clock(ms as i64, cx)),
                    p.text_subtle,
                )
                .into_any_element(),
            ])
        })
        .collect();
    let runs: Vec<AnyElement> = d
        .runs
        .iter()
        .map(|r| {
            let n = |k: &str| r.num_at(&[k]).unwrap_or(0.) as i64;
            let kind = r.str_at(&["kind"]).unwrap_or("");
            let status = r.str_at(&["status"]).unwrap_or("");
            let (label, chip_kind) = match status {
                "completed" => ("done", ChipKind::Neutral),
                "failed" => ("failed", ChipKind::Danger),
                "running" => ("running", ChipKind::Info),
                s => (s, ChipKind::Neutral),
            };
            let kind_label = match kind {
                "activity" => "Activities",
                "conclusion" => "Conclusions",
                "confidence" => "Confidence",
                "backfill" => "Backfill",
                k => k,
            };
            let window = match (r.num_at(&["windowStartMs"]), r.num_at(&["windowEndMs"])) {
                (Some(a), Some(b)) => format!(
                    " \u{b7} {}",
                    crate::insights::stats::clock_range(a as i64, b as i64, off, "\u{2013}")
                ),
                _ => String::new(),
            };
            let plural =
                |k: i64, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
            let dropped = n("ungrounded")
                + n("guardrailSuppressed")
                + n("belowFormationBar")
                + n("resurfaceBlocked");
            let result = match status {
                "failed" => "Couldn\u{2019}t finish".to_string(),
                "running" => "Working\u{2026}".into(),
                "skipped" => "Nothing captured".into(),
                _ => {
                    let made = if matches!(kind, "conclusion" | "confidence") {
                        plural(n("conclusionsDerived"), "conclusion", "conclusions")
                    } else {
                        plural(n("activitiesDerived"), "activity", "activities")
                    };
                    if dropped > 0 {
                        format!("{made}, {dropped} dropped")
                    } else {
                        made
                    }
                }
            };
            let (tok, unit) = readouts::token_stat((n("inputTokens") + n("outputTokens")) as f64);
            let meta = format!(
                "{tok}{unit} tok \u{b7} {}",
                r.num_at(&["createdAtMs"])
                    .map_or(String::new(), |ms| clock(ms as i64, cx))
            );
            log_row(vec![
                cell(Some(150.), div().whitespace_nowrap().overflow_hidden())
                    .child(crate::chat::inline_runs(&[
                        (kind_label, Some((theme::W_MEDIUM, p.text))),
                        (&window, None),
                    ]))
                    .into_any_element(),
                chip(label.to_string(), chip_kind, p).into_any_element(),
                cell(None, div().truncate())
                    .child(result)
                    .into_any_element(),
                mono(meta, p.text_subtle).into_any_element(),
            ])
        })
        .collect();
    let uc_line = d.uc_status.as_ref().map(|s| {
        if s.str_at(&["reason"]) == Some("user_context_disabled") {
            return "turned off".to_string();
        }
        let dg = s.at(&["lastDayDigest"]);
        if dg.is_null() {
            return "no digest yet".to_string();
        }
        // Fresh = it covers today (ends after local midnight).
        let midnight = crate::insights::stats::start_of_day(now_ms, off);
        let fresh = dg
            .num_at(&["rangeEndMs"])
            .is_some_and(|e| e as i64 > midnight);
        format!(
            "{} \u{b7} built {}",
            if fresh {
                "digest fresh"
            } else {
                "digest from an earlier day"
            },
            dg.num_at(&["generatedAtMs"])
                .map_or(String::new(), |ms| clock(ms as i64, cx))
        )
    });
    vec![
        sec(p).child(engine).into_any_element(),
        sec(p)
            .child(
                h("Last Ask AI turn", None, p)
                    .child(div().flex_1())
                    .children(started.map(|s| {
                        div()
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .text_color(p.text_subtle)
                            .child(s)
                    })),
            )
            .child(usage.map_or_else(
                || compact_empty("No Ask AI turn since the app started", false, p),
                |u| u.into_any_element(),
            ))
            .into_any_element(),
        sec(p)
            .child(h("User Context", uc_line, p).child(div().flex_1()).child({
                let me = cx.entity().downgrade();
                let deriving = w.acting == Some(super::data::Action::Derive);
                Button::new(
                    "dv-derive",
                    if deriving {
                        "Deriving\u{2026}"
                    } else {
                        "Derive latest now"
                    },
                )
                .ghost()
                .sm()
                .icon("icons/refresh-cw.svg")
                .spinner(deriving)
                .busy(deriving)
                .disabled(w.acting.is_some())
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |w, cx| w.act(super::data::Action::Derive, cx));
                })
            }))
            .child(if w.data.runs_error.is_some() {
                compact_empty("Couldn\u{2019}t load recent runs", true, p)
            } else if runs.is_empty() {
                compact_empty("No User Context runs yet", false, p)
            } else {
                list(p).children(runs).into_any_element()
            })
            .into_any_element(),
        sec(p)
            .child(h(
                "CLI access",
                Some(format!(
                    "{allowed} {} allowed",
                    if allowed == 1 { "tool" } else { "tools" }
                )),
                p,
            ))
            .child(if w.data.cli_error.is_some() {
                compact_empty("Couldn\u{2019}t read CLI requests", true, p)
            } else if cli.is_empty() {
                compact_empty("No CLI requests yet", false, p)
            } else {
                list(p).children(cli).into_any_element()
            })
            .into_any_element(),
    ]
}
