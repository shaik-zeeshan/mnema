//! The job inspector (`tabs/JobInspector.svelte`): replaces the jobs table
//! inside the drill-in; Back returns. Open in Timeline drives the MAIN window,
//! this one never navigates.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, ClipboardItem, Context, Div, IntoElement};
use serde_json::Value;

use super::data::JobCall;
use super::detail::{self, JobState, Spec};
use super::detail_view::{chip_kind, job_ts, num, raw_err, spec_list};
use super::model::Tab;
use super::tabs::parse_job_ts;
use super::DebugWindow;
use crate::settings::model::Json;
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::button::Button;
use crate::ui::chip::chip;
use crate::ui::notice::notice_el;
use crate::ui::{icon, Tone};

/// How long the Copy id tip says "Copied".
const COPIED_FOR: std::time::Duration = std::time::Duration::from_millis(1400);

/// What Open in Timeline lands on: a frame or an audio segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Frame(i64),
    Audio(i64),
}

pub fn timeline_target(job: &Value) -> Option<Target> {
    let id = job.num_at(&["subjectId"])? as i64;
    match job.str_at(&["subjectType"])? {
        "frame" => Some(Target::Frame(id)),
        "audio_segment" => Some(Target::Audio(id)),
        _ => None,
    }
}

/// The row with its payload parsed in place (`JSON.stringify(.., null, 2)`).
pub fn raw_json(job: &Value) -> String {
    let mut v = job.clone();
    if let Some(s) = job.str_at(&["payloadJson"]) {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            v["payloadJson"] = parsed;
        }
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

pub fn view(
    w: &DebugWindow,
    spec: Spec,
    job: &Value,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Vec<AnyElement> {
    let d = &w.detail;
    let now = crate::ui::clock::now_ms(cx);
    let id = job.num_at(&["id"]).unwrap_or(0.) as i64;
    let st = detail::job_state(job, now);
    let provider = detail::job_provider(job);
    let model_id = detail::job_model(job);
    let target = timeline_target(job);
    let can_reprocess = matches!(st, JobState::Failed | JobState::Completed)
        && spec.subject_type.is_some()
        && job.str_at(&["subjectType"]) == spec.subject_type;
    let mut out = Vec::new();

    out.push(
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .child(
                        div()
                            .id("dv-insp-title")
                            .font_family(FONT_MONO)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::LG)
                            .line_height(relative(1.2))
                            .text_color(p.text_strong)
                            .child(format!("#{id}")),
                    )
                    .child(chip(st.word(), chip_kind(st), p)),
            )
            .child(
                div()
                    .text_size(text::BASE)
                    .line_height(relative(1.4))
                    .text_color(p.text_muted)
                    .child(format!(
                        "{} of {}",
                        spec.name,
                        detail::subject_label(job).to_lowercase()
                    )),
            )
            .into_any_element(),
    );

    if let Some(e) = job.str_at(&["lastError"]) {
        let transient = detail::is_transient(Some(e));
        let title = if st == JobState::Failed || transient {
            detail::error_title(Some(e), provider.as_deref())
        } else {
            "The last try failed".into()
        };
        out.push(
            notice_el(
                if st == JobState::Failed {
                    Tone::Danger
                } else {
                    Tone::Warn
                },
                if transient {
                    "icons/cloud-off.svg"
                } else {
                    "icons/triangle-alert.svg"
                },
                title,
                Some(
                    div()
                        .id("dv-insp-error")
                        .test_support()
                        .child(raw_err(e, p))
                        .into_any_element(),
                ),
                vec![],
                p,
            )
            .into_any_element(),
        );
    }

    // Created · Last try (took) · Tries · Provider.
    let attempts = job.num_at(&["attemptCount"]).unwrap_or(0.) as i64;
    let failures = job.num_at(&["failureCount"]).unwrap_or(0.) as i64;
    let last_try = job
        .str_at(&["startedAt"])
        .or_else(|| (attempts > 0).then(|| job.str_at(&["updatedAt"])).flatten());
    let took = match (
        job.str_at(&["startedAt"]).and_then(parse_job_ts),
        job.str_at(&["finishedAt"]).and_then(parse_job_ts),
    ) {
        (Some(a), Some(b)) if b >= a => format!("took {:.1} s", (b - a) as f64 / 1000.),
        _ => String::new(),
    };
    let tries = div()
        .flex()
        .flex_wrap()
        .gap(px(4.))
        .child(num(attempts.to_string(), p))
        .child(if attempts == 1 { "try," } else { "tries," })
        .child(num(failures.to_string(), p))
        .child("failed");
    let provider_cell = div()
        .flex()
        .flex_wrap()
        .child(
            provider
                .as_deref()
                .map_or("\u{2014}".into(), detail::provider_name),
        )
        .children(model_id.map(|m| div().flex().child(" \u{b7} ").child(num(m, p))));
    let rows: [(&str, Div, &str); 4] = [
        (
            "Created",
            num(job_ts(job.str_at(&["createdAt"]), cx), p),
            "",
        ),
        ("Last try", num(job_ts(last_try, cx), p), &took),
        ("Tries", tries, ""),
        ("Provider", provider_cell, ""),
    ];
    out.push(spec_list(rows.into_iter(), p));
    out.push(actions(w, id, st, can_reprocess, target, job, cx, p));
    if let Some((ok, line)) = &d.message {
        out.push(
            div()
                .id("dv-insp-message")
                .test_support()
                .mt(px(-10.))
                .text_size(text::SM)
                .line_height(relative(1.4))
                .text_color(if *ok { p.text_muted } else { p.danger_text })
                .child(line.clone())
                .into_any_element(),
        );
    }
    out.push(raw(w, job, cx, p));
    out
}

#[allow(clippy::too_many_arguments)]
fn actions(
    w: &DebugWindow,
    id: i64,
    st: JobState,
    can_reprocess: bool,
    target: Option<Target>,
    job: &Value,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let _ = p;
    let acting = w.detail.acting;
    let open = Button::new("dv-open-timeline", "Open in Timeline")
        .primary()
        .sm()
        .icon("icons/chart-no-axes-gantt.svg")
        .disabled(target.is_none())
        .on_click(move |_, _, cx| match target {
            Some(Target::Frame(f)) => cx.defer(move |cx| crate::window::open_frame(f, cx)),
            Some(Target::Audio(a)) => {
                cx.defer(move |cx| crate::timeline::open_audio(a, 0, None, None, cx))
            }
            None => {}
        });
    let reprocess = can_reprocess.then(|| {
        let me = cx.entity().downgrade();
        let call = JobCall::Reprocess {
            processor: job.str_at(&["processor"]).unwrap_or("").to_string(),
            subject_id: job.num_at(&["subjectId"]).unwrap_or(0.) as i64,
        };
        Button::new("dv-reprocess", "Reprocess")
            .sm()
            .icon("icons/refresh-cw.svg")
            .disabled(acting)
            .on_click(move |_, _, cx| {
                let call = call.clone();
                let _ = me.update(cx, |w, cx| w.job_act(call, cx));
            })
    });
    let retry = (st == JobState::Retrying).then(|| {
        let me = cx.entity().downgrade();
        Button::new("dv-retry-now", "Retry now")
            .sm()
            .icon("icons/rotate-ccw.svg")
            .disabled(acting)
            .on_click(move |_, _, cx| {
                let _ = me.update(cx, |w, cx| w.job_act(JobCall::RetryNow(id), cx));
            })
    });
    let me = cx.entity().downgrade();
    let log = Button::icon_only("dv-show-in-log", "icons/file-text.svg", "Show in log")
        .ghost()
        .sm()
        .tip("Show in log")
        .on_click(move |_, window, cx| {
            let _ = me.update(cx, |w, cx| {
                w.needle
                    .update(cx, |s, cx| s.set_value(detail::log_needle(id), window, cx));
                w.go(Tab::Logs, cx);
            });
        });
    let me = cx.entity().downgrade();
    let copy = Button::icon_only(
        "dv-copy-id",
        gpui_kit::assets::IconName::Copy.path(),
        "Copy job id",
    )
    .ghost()
    .sm()
    .tip(if w.detail.copied { "Copied" } else { "Copy id" })
    .on_click(move |_, _, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
        let _ = me.update(cx, |w, cx| {
            w.detail.copied = true;
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(COPIED_FOR).await;
                let _ = this.update(cx, |w, cx| {
                    w.detail.copied = false;
                    cx.notify();
                });
            })
            .detach();
        });
    });
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .child(open)
        .children(reprocess)
        .children(retry)
        .child(div().flex_1())
        .child(log)
        .child(copy)
        .into_any_element()
}

/// `<details>` Raw payload: `processing_jobs · N B`, the row as pretty JSON.
fn raw(w: &DebugWindow, job: &Value, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let json = raw_json(job);
    let open = w.detail.raw_open;
    let strong = p.text_strong;
    div()
        .flex()
        .flex_col()
        .rounded(radius::MD)
        .border_1()
        .border_color(p.border)
        .child(
            div()
                .id("dv-raw")
                .test_support()
                .role(gpui_kit::Role::Button)
                .aria_label("Raw payload")
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(8.))
                .cursor_pointer()
                .font_weight(theme::W_MEDIUM)
                .text_size(text::BASE)
                .line_height(relative(1.))
                .text_color(p.text_muted)
                .hover(move |s| s.text_color(strong))
                .on_click(cx.listener(|w, _, _, cx| {
                    w.detail.raw_open = !w.detail.raw_open;
                    cx.notify();
                }))
                .child(div().text_color(p.text_subtle).child(icon(
                    if open {
                        "icons/chevron-down.svg"
                    } else {
                        "icons/chevron-right.svg"
                    },
                    px(12.),
                )))
                .child("Raw payload")
                .child(div().flex_1())
                .child(
                    div()
                        .font_family(FONT_MONO)
                        .font_weight(theme::W_REGULAR)
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(format!("processing_jobs \u{b7} {} B", json.len())),
                ),
        )
        .when(open, |el| {
            el.child(
                div()
                    .id("dv-raw-json")
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(10.))
                    .border_t_1()
                    .border_color(p.hairline)
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .line_height(relative(1.5))
                    .text_color(p.text_muted)
                    .child(json),
            )
        })
        .into_any_element()
}
