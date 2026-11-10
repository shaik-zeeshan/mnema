//! The scrolling body and the Overview and Pipeline tabs
//! (`tabs/{OverviewTab,PipelineTab}.svelte`, `HealthRow.svelte`).

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, SharedString, Window};
use serde_json::Value;

use super::model::{self, RowView, Tab};
use super::DebugWindow;
use crate::insights::stats;
use crate::settings::model::Json;
use crate::small::parts::{dot, spec_row};
use crate::theme::{self, radius, space, text, Palette, FONT_MONO};
use crate::ui::{icon, mix, nid, ring_box, Tone};

pub(super) fn clock(ms: i64, cx: &gpui_kit::App) -> String {
    stats::clock(ms, crate::ui::clock::now(cx).offset())
}

/// `.dv-h`: 500 12 muted, optional mono count.
pub(super) fn h(title: impl Into<SharedString>, num: Option<String>, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space::S2)
        .min_h(px(22.))
        .font_weight(theme::W_MEDIUM)
        .text_size(text::BASE)
        .line_height(relative(1.))
        .text_color(p.text_muted)
        .child(title.into())
        .children(num.map(|n| {
            div()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child(n)
        }))
}

pub(super) fn sec(p: &Palette) -> Div {
    let _ = p;
    div().flex().flex_col().gap(px(6.)).min_w_0()
}

/// One `.dv-row`: dot · name · plain reason · metric · chevron. `focus` makes
/// it a Tab stop (Enter / Space press it), with its focus-visible ring.
fn row(
    id: gpui_kit::ElementId,
    r: &RowView,
    plain: bool,
    focus: Option<(&gpui_kit::FocusHandle, bool)>,
    on_click: impl Fn(&mut DebugWindow, &mut Context<DebugWindow>) + 'static,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let loud = matches!(r.tone, Tone::Warn | Tone::Danger | Tone::Info);
    let metric_color = if matches!(r.tone, Tone::Warn | Tone::Danger) && !plain {
        r.tone.colors(p).0
    } else {
        p.text_muted
    };
    let ok = r.tone == Tone::Ok;
    let wash = p.wash;
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(SharedString::from(r.name.clone()))
        .flex()
        .items_center()
        .gap(px(10.))
        .min_h(px(34.))
        .py(px(6.))
        .pl(px(6.))
        .pr(px(4.))
        .border_b_1()
        .border_color(p.hairline)
        .cursor_pointer()
        .hover(move |s| s.bg(wash))
        .relative()
        .when_some(focus, |el, (h, visible)| {
            el.track_focus(h).when(visible, |el| {
                el.child(
                    ring_box(p.accent, px(2.), px(0.))
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0(),
                )
            })
        })
        .on_click(cx.listener(move |w, _, window, cx| {
            on_click(w, cx);
            window.focus(&w.root_focus, cx);
        }))
        .child(dot(r.tone, px(7.), p))
        .child(
            div()
                .flex_none()
                .w(px(108.))
                .font_weight(if ok {
                    theme::W_REGULAR
                } else {
                    theme::W_MEDIUM
                })
                .text_size(text::MD)
                .line_height(relative(1.3))
                .text_color(if ok { p.text } else { p.text_strong })
                .child(r.name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(text::BASE)
                .line_height(relative(1.4))
                .text_color(if loud { p.text } else { p.text_subtle })
                .child(r.why.clone()),
        )
        .child(
            div()
                .flex_none()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .text_color(metric_color)
                .whitespace_nowrap()
                .child(r.metric.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_color(p.text_faint)
                .child(icon("icons/chevron-right.svg", px(14.))),
        )
        .into_any_element()
}

pub(super) fn list(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(p.hairline)
}

fn overview(w: &DebugWindow, cx: &mut Context<DebugWindow>, p: &Palette) -> Vec<AnyElement> {
    let clk = |ms: i64| clock(ms, cx);
    let groups = model::group(&w.data.health, &clk);
    let mut n = 0u64;
    groups
        .into_iter()
        .map(|(title, rows)| {
            let count = rows.len().to_string();
            let mut items = Vec::new();
            for r in &rows {
                n += 1;
                let tab = r.tab;
                items.push(row(
                    nid("dv-row", n),
                    r,
                    false,
                    None,
                    move |w, cx| w.go(tab, cx),
                    cx,
                    p,
                ));
            }
            sec(p)
                .child(h(title, Some(count), p))
                .child(list(p).children(items))
                .into_any_element()
        })
        .collect()
}

fn flow(w: &DebugWindow, p: &Palette) -> AnyElement {
    let nodes = model::flow_nodes(&w.data.health, &w.data.lanes, w.data.semantic.as_ref());
    div()
        .flex()
        .gap(px(8.))
        .children(nodes.into_iter().map(|n| {
            let (t, bg, bd) = n
                .tone
                .map_or((p.text_muted, p.surface_subtle, p.border), |t| t.colors(p));
            let loud = matches!(n.tone, Some(Tone::Warn | Tone::Danger));
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(9.))
                .pt(px(9.))
                .pb(px(10.))
                .rounded(radius::MD)
                .border_1()
                .border_color(if loud {
                    mix(bd, p.border, 0.75)
                } else {
                    p.border
                })
                .bg(if loud {
                    mix(bg, p.surface, 0.55)
                } else {
                    p.surface_subtle
                })
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::SM)
                        .line_height(relative(1.2))
                        .text_color(p.text_muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(n.name),
                )
                .child(
                    div()
                        .mt(px(5.))
                        .font_family(FONT_MONO)
                        .text_size(text::XL)
                        .line_height(relative(1.))
                        .text_color(p.text_strong)
                        .child(n.big),
                )
                .child(
                    div()
                        .mb(px(3.))
                        .text_size(text::XS)
                        .line_height(relative(1.2))
                        .text_color(p.text_subtle)
                        .child(n.unit),
                )
                .children(n.lines.into_iter().map(|l| {
                    div()
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .line_height(relative(1.35))
                        .text_color(l.tone.map_or(p.text_muted, |t| t.colors(p).0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(l.text)
                }))
                .children(n.bar.map(|v| {
                    div()
                        .mt(px(6.))
                        .h(px(3.))
                        .rounded(px(2.))
                        .bg(p.wash_strong)
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(relative(v))
                                .rounded(px(2.))
                                .bg(n.tone.map_or(p.chart_grey_4, |_| t)),
                        )
                }))
        }))
        .into_any_element()
}

fn pipeline(
    w: &DebugWindow,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Vec<AnyElement> {
    if w.drilled() {
        return super::detail_view::view(w, window, cx, p);
    }
    let d = &w.data;
    let rows = model::processor_rows(&d.health, &d.lanes, d.settings.as_ref());
    let mut items = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let h = &w.proc_focus[i];
        let visible = h.is_focused(window) && window.last_input_was_keyboard();
        items.push(row(
            nid("dv-proc", i as u64),
            r,
            true,
            Some((h, visible)),
            move |w, cx| w.open_processor(i, cx),
            cx,
            p,
        ));
    }
    let at = |s: Option<&str>| {
        s.and_then(parse_job_ts)
            .map_or("\u{2014}".to_string(), |ms| clock(ms, cx))
    };
    let (ret, ret_at) = retention_line(d.retention.as_ref());
    let (bat, bat_at) = batch_line(&d.batches);
    let (jobs, jobs_at) = jobs_line(&d.app_jobs);
    let house = [
        ("Retention cleanup", ret, at(ret_at.as_deref())),
        ("Frame batches", bat, at(bat_at.as_deref())),
        ("App jobs", jobs, at(jobs_at.as_deref())),
    ];
    let err = |e: &Option<String>| {
        e.clone()
            .map(|e| div().text_size(text::SM).text_color(p.danger_text).child(e))
    };
    vec![
        sec(p)
            .child(h("Flow", Some("failed = last 24 h".into()), p))
            .child(flow(w, p))
            .children(err(&d.pipeline_error))
            .into_any_element(),
        sec(p)
            .child(h("Processors", None, p))
            .child(list(p).children(items))
            .into_any_element(),
        sec(p)
            .child(h("Housekeeping", None, p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .children(
                        house
                            .into_iter()
                            .enumerate()
                            .map(|(i, (label, value, note))| {
                                spec_row(
                                    i == 0,
                                    label,
                                    div().child(value),
                                    Some(note.into()),
                                    px(120.),
                                    p,
                                )
                            }),
                    ),
            )
            .into_any_element(),
    ]
}

/// SQLite's naive-UTC "YYYY-MM-DD HH:MM:SS" (or RFC 3339) → unix ms.
pub(super) fn parse_job_ts(s: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    let t = s.trim();
    let iso = if t.len() >= 19 && t.as_bytes()[10] == b' ' {
        let naive = format!("{}T{}", &t[..10], &t[11..]);
        if naive.ends_with('Z') || naive.contains('+') {
            naive
        } else {
            format!("{naive}Z")
        }
    } else {
        t.to_string()
    };
    time::OffsetDateTime::parse(&iso, &Rfc3339)
        .ok()
        .map(|d| (d.unix_timestamp_nanos() / 1_000_000) as i64)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
}

fn retention_line(s: Option<&Value>) -> (String, Option<String>) {
    let Some(s) = s else {
        return ("\u{2014}".into(), None);
    };
    let days = s
        .str_at(&["policy"])
        .and_then(|p| p.strip_prefix("days_"))
        .map(str::to_string);
    let Some(days) = days else {
        return ("keeps everything".into(), None);
    };
    let at = s.str_at(&["createdAt"]).map(str::to_string);
    if s.str_at(&["status"]) == Some("failed") {
        return (
            format!(
                "failed \u{b7} {}",
                first_line(s.str_at(&["errorMessage"]).unwrap_or("unknown error"))
            ),
            at,
        );
    }
    if at.is_none() {
        return (format!("nothing run yet \u{b7} keeps {days} days"), None);
    }
    let n = s.num_at(&["deletedCaptureSegments"]).unwrap_or(0.) as i64;
    (
        format!(
            "removed {} {} older than {days} days",
            model::count(n),
            if n == 1 { "file" } else { "files" }
        ),
        at,
    )
}

fn latest(rows: &[&Value]) -> Option<String> {
    rows.iter()
        .filter_map(|r| r.str_at(&["updatedAt"]))
        .max()
        .map(str::to_string)
}

fn batch_line(b: &[Value]) -> (String, Option<String>) {
    let failed: Vec<&Value> = b
        .iter()
        .filter(|x| x.str_at(&["status"]) == Some("failed"))
        .collect();
    if !failed.is_empty() {
        let e = failed
            .iter()
            .find_map(|x| x.str_at(&["lastError"]))
            .map(first_line);
        return (
            format!(
                "{} failed{}",
                failed.len(),
                e.map_or(String::new(), |e| format!(" \u{b7} {e}"))
            ),
            latest(&failed),
        );
    }
    let open = b.len();
    (
        format!(
            "{} in progress, none failed",
            if open > 0 {
                model::count(open as i64)
            } else {
                "none".into()
            }
        ),
        latest(&b.iter().collect::<Vec<_>>()),
    )
}

fn jobs_line(j: &[Value]) -> (String, Option<String>) {
    let running: Vec<&Value> = j
        .iter()
        .filter(|x| x.str_at(&["status"]) == Some("running"))
        .collect();
    if !running.is_empty() {
        let mut kinds: Vec<String> = running
            .iter()
            .map(|r| match r.str_at(&["kind"]).unwrap_or("") {
                "frame_batch_combine" => "Combining frames".to_string(),
                k => k.replace('_', " "),
            })
            .collect();
        kinds.dedup();
        return (
            format!("{} running \u{b7} {}", running.len(), kinds.join(", ")),
            latest(&running),
        );
    }
    let queued = j
        .iter()
        .filter(|x| x.str_at(&["status"]) == Some("queued"))
        .count();
    (
        if queued > 0 {
            format!("none running \u{b7} {queued} queued")
        } else {
            "none running".into()
        },
        latest(&j.iter().collect::<Vec<_>>()),
    )
}

/// The tab body: one scroll column (the Logs tab manages its own tail).
pub fn body(
    w: &DebugWindow,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    if w.tab == Tab::Logs {
        return super::logs::tab(w, window, cx, p);
    }
    let children = match w.tab {
        Tab::Overview => overview(w, cx, p),
        Tab::Pipeline => pipeline(w, window, cx, p),
        Tab::Capture => super::capture::tab(w, cx, p),
        Tab::Ai => super::ai::tab(w, cx, p),
        Tab::Logs => vec![],
    };
    div()
        .id("dv-body")
        .track_scroll(&w.body_scroll)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(18.))
        .pt(px(14.))
        .px(space::S4)
        .pb(space::S4)
        // A button's outcome, in the backend's words, until the next one.
        .children(w.note.as_ref().map(|(ok, line)| {
            div()
                .id("dv-note")
                .text_size(crate::theme::text::BASE)
                .text_color(if *ok { p.text_muted } else { p.danger_text })
                .child(line.clone())
        }))
        .children(children)
        .child(crate::ui::scroll::bar(&w.body_scroll))
        .into_any_element()
}
