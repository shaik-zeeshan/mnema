//! The processor drill-in (`tabs/ProcessorView.svelte`): breadcrumb, the
//! processor's spec, the provider-unreachable notice, the status filter and
//! the jobs table; a job opens the inspector in place (`inspector.rs`). The
//! meaning index has no jobs, so it shows only the semantic-index spec.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Div, IntoElement, SharedString, Window};
use serde_json::Value;

use super::detail::{self, Filter, JobState, Spec, JOBS_SHOWN};
use super::tabs::{list, parse_job_ts};
use super::{model, DebugWindow};
use crate::settings::model::Json;
use crate::small::parts::spec_row;
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::button::Button;
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::notice::notice_el;
use crate::ui::segmented::{opt_count, Segmented};
use crate::ui::Tracking as _;
use crate::ui::{focus_handle, icon, nid, ring_box, with_tip, CopyButton, Tone};

/// `.mx-spec` label column in the drill-in.
pub(super) const LABEL_W: gpui_kit::Pixels = px(110.);

pub(super) fn chip_kind(s: JobState) -> ChipKind {
    match s {
        JobState::Retrying => ChipKind::Warn,
        JobState::Failed => ChipKind::Danger,
        JobState::Running | JobState::Preparing => ChipKind::Info,
        _ => ChipKind::Neutral,
    }
}

/// `formatJobTs`: the local clock with seconds, the raw string if unparseable.
pub(super) fn job_ts(s: Option<&str>, cx: &gpui_kit::App) -> String {
    let Some(s) = s.filter(|s| !s.is_empty()) else {
        return "\u{2014}".into();
    };
    parse_job_ts(s).map_or(s.to_string(), |ms| {
        let off = crate::ui::clock::now(cx).offset();
        crate::timeline::model::clock_label(crate::insights::stats::local(ms, off), true)
    })
}

pub(super) fn n(v: Option<f64>) -> String {
    v.map_or("\u{2014}".into(), |v| model::count(v as i64))
}

/// `.num` inline.
pub(super) fn num(s: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .font_family(FONT_MONO)
        .tracking(-0.01)
        .text_color(p.text_strong)
        .child(s.into())
}

pub(super) fn note(s: &str) -> Option<SharedString> {
    (!s.is_empty()).then(|| SharedString::from(s.to_string()))
}

/// The raw error string inside a notice (`.dv-raw-err`): mono, wrapped anywhere.
/// The raw error box, with a copy button for a bug report.
pub(super) fn raw_err(copy_id: &'static str, e: &str, p: &Palette) -> Div {
    div()
        .mt(px(6.))
        .pl(px(8.))
        .pr(px(4.))
        .py(px(4.))
        .flex()
        .items_start()
        .gap(px(6.))
        .rounded(theme::radius::SM)
        .bg(p.wash)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .py(px(2.))
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .line_height(relative(1.45))
                .text_color(p.text)
                .child(e.to_string()),
        )
        .child(CopyButton::icon(copy_id, "Copy error", e.to_string()).h(px(22.)))
}

fn link(
    id: &'static str,
    label: &'static str,
    on: impl Fn(&mut DebugWindow, &mut Window, &mut Context<DebugWindow>) + 'static,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let strong = p.text_strong;
    div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Link)
        .cursor_pointer()
        .font_weight(theme::W_REGULAR)
        .text_color(p.text_subtle)
        .hover(move |s| s.text_color(strong))
        .on_click(cx.listener(move |w, _, window, cx| on(w, window, cx)))
        .child(label)
        .into_any_element()
}

/// `.dv-crumb`: Back · Pipeline › <processor> (a link while a job is open).
fn crumb(w: &DebugWindow, spec: Spec, cx: &mut Context<DebugWindow>, p: &Palette) -> AnyElement {
    let me = cx.entity().downgrade();
    let back = Button::icon_only("dv-back", "icons/arrow-left.svg", "Back")
        .ghost()
        .sm()
        .h(px(24.))
        .on_click(move |_, window, cx| {
            let _ = me.update(cx, |w, cx| w.back(window, cx));
        });
    let name = if w.detail.inspected.is_some() {
        link(
            "dv-crumb-proc",
            spec.name,
            |w, window, cx| w.back(window, cx),
            cx,
            p,
        )
    } else {
        div().child(spec.name).into_any_element()
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .mt(px(-4.))
        .mb(px(-6.))
        .ml(px(-6.))
        .font_weight(theme::W_MEDIUM)
        .text_size(text::MD)
        .line_height(relative(1.))
        .text_color(p.text_strong)
        .child(div().mr(px(2.)).child(back))
        .child(link(
            "dv-crumb-pipeline",
            "Pipeline",
            |w, window, cx| {
                w.detail.inspected = None;
                w.back(window, cx);
            },
            cx,
            p,
        ))
        .child(
            div()
                .text_color(p.text_faint)
                .child(icon("icons/chevron-right.svg", px(12.))),
        )
        .child(name)
        .into_any_element()
}

pub fn view(
    w: &DebugWindow,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Vec<AnyElement> {
    let Some(spec) = w.detail.spec else {
        return vec![];
    };
    let mut out = vec![crumb(w, spec, cx, p)];
    if let Some(job) = w.detail.inspected.clone() {
        out.extend(super::inspector::view(w, spec, &job, cx, p));
    } else if spec.processor.is_none() {
        out.extend(meaning_index(w, p));
    } else {
        out.extend(lane_view(w, spec, window, cx, p));
    }
    out
}

fn meaning_index(w: &DebugWindow, p: &Palette) -> Vec<AnyElement> {
    let ix = w.data.semantic.as_ref();
    let at = |k: &str| ix.and_then(|v| v.num_at(&[k]));
    let dim = ix.map_or("\u{2014}".to_string(), |v| {
        v.num_at(&["liveDimension"])
            .map_or("no index".into(), |d| (d as i64).to_string())
    });
    let loaded = ix
        .and_then(|v| v["modelLoaded"].as_bool())
        .map_or("\u{2014}", |b| if b { "Yes" } else { "No" });
    let rows: [(&str, Div, &str); 6] = [
        ("Passages searchable", num(n(at("vectorCount")), p), ""),
        (
            "Behind",
            num(n(at("backlogCount")), p),
            "waiting to be indexed",
        ),
        ("Dimension", num(dim, p), ""),
        ("Model loaded", div().child(loaded), "unloads when idle"),
        (
            "Load failures",
            num(n(at("consecutiveLoadFailures")), p),
            "in a row",
        ),
        (
            "Quarantined",
            num(n(at("quarantinedCount")), p),
            "since app start",
        ),
    ];
    let mut out = vec![spec_list(rows.into_iter(), p)];
    if let Some(e) = ix.and_then(|v| v.str_at(&["lastLoadError"])) {
        out.push(
            notice_el(
                Tone::Danger,
                "icons/triangle-alert.svg",
                "The model couldn\u{2019}t load",
                Some(raw_err("dv-load-error-copy", e, p).into_any_element()),
                vec![],
                p,
            )
            .into_any_element(),
        );
    }
    out
}

pub(super) fn spec_list<'a>(
    rows: impl Iterator<Item = (&'a str, Div, &'a str)>,
    p: &Palette,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .children(
            rows.enumerate()
                .map(|(i, (label, value, n))| spec_row(i == 0, label, value, note(n), LABEL_W, p)),
        )
        .into_any_element()
}

/// The language a model runs in, `None` for auto.
fn language(code: Option<&str>) -> Option<String> {
    let code = code.filter(|c| !c.is_empty() && *c != "auto")?;
    // shortcut: a handful of names, the code otherwise (Svelte asks Intl.DisplayNames).
    Some(
        match code {
            "en" => "English",
            "de" => "German",
            "fr" => "French",
            "es" => "Spanish",
            "it" => "Italian",
            "ja" => "Japanese",
            "zh" => "Chinese",
            "hi" => "Hindi",
            "pt" => "Portuguese",
            "nl" => "Dutch",
            other => return Some(other.to_string()),
        }
        .to_string(),
    )
}

fn lane_view(
    w: &DebugWindow,
    spec: Spec,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Vec<AnyElement> {
    let d = &w.detail;
    let now = crate::ui::clock::now_ms(cx);
    let settings = w.data.settings.as_ref();
    let lane = spec.processor.and_then(|pr| model::lane(&w.data.lanes, pr));
    let config = settings.map(|s| s.at(&[spec.cfg])).filter(|c| !c.is_null());
    let cloud = config.and_then(|c| c.str_at(&["provider"])) == Some("deepgram");
    let lane_n = |k: &str| lane.and_then(|l| l.num_at(&[k]));

    // Spec: average time, failures, provider, model.
    let avg = lane_n("averageCompletedSecondsLast24h").map(model::avg_time);
    let avg = match (avg, spec.per) {
        (Some(a), per) => div()
            .flex()
            .gap(px(4.))
            .child(num(a, p))
            .children(per.map(|s| div().child(s))),
        (None, _) => div().child("\u{2014}"),
    };
    let failed = div()
        .flex()
        .gap(px(4.))
        .child(num(n(lane_n("failedLast24h")), p))
        .child("in the last 24 h");
    let provider = model::engine_words(&spec, settings, None).replacen(", ", " \u{b7} ", 1);
    let provider = if provider.is_empty() {
        "\u{2014}".into()
    } else {
        provider
    };
    let model_id = config
        .and_then(|c| c.str_at(&["modelId"]))
        .unwrap_or("default")
        .to_string();
    let lang = language(config.and_then(|c| c.str_at(&["language"])));
    let model_cell = div()
        .flex()
        .flex_wrap()
        // A long model id wraps inside the value column instead of running past it.
        .child(num(model_id, p).min_w_0())
        .children(lang.map(|l| div().child(format!(" \u{b7} {l}"))));
    let rows: [(&str, Div, &str); 4] = [
        ("Average time", avg, "last 24 h"),
        ("Failed", failed, ""),
        (
            "Provider",
            div().child(provider),
            if cloud { "your key" } else { "" },
        ),
        ("Model", model_cell, ""),
    ];
    let mut out = vec![spec_list(rows.into_iter(), p)];
    out.extend(unreachable(w, lane, config, cloud, now, cx, p));

    // The filter and the table.
    let counts: Vec<_> = Filter::ALL
        .iter()
        .map(|f| {
            opt_count(
                f.label(),
                detail::filter_count(lane, *f).map_or("\u{2014}".into(), model::count),
            )
        })
        .collect();
    let selected = Filter::ALL.iter().position(|f| *f == d.filter).unwrap_or(0);
    let me = cx.entity().downgrade();
    let seg = Segmented::new("dv-job-filter", "Job status", counts, selected).on_change(
        move |ix, _, cx| {
            let _ = me.update(cx, |w, cx| w.set_filter(Filter::ALL[ix], cx));
        },
    );
    let visible = d.visible(now);
    let rows: Vec<&Value> = visible.into_iter().take(JOBS_SHOWN).collect();
    let retrying = lane_n("retrying").unwrap_or(0.) as i64;
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .min_w_0()
        .child(div().flex().items_center().gap(space::S2).child(seg));
    if rows.is_empty() {
        let title = d.error.clone().unwrap_or_else(|| "No jobs here".into());
        section = section.child(div().id("dv-jobs-empty").test_support().child(empty(
            EmptyLayout::Compact,
            Tone::Neutral,
            "icons/inbox.svg",
            title,
            "",
            vec![],
            p,
        )));
    } else {
        let split = detail::filter_count(lane, d.filter);
        let tail = detail::tail(d.filter, split, d.total, &rows);
        let table = list(p).child(head(p)).children(
            rows.iter()
                .map(|job| job_row(job, now, retrying, window, cx, p)),
        );
        section = section
            .child(table)
            .children(tail.map(|t| {
                div()
                    .pt(px(4.))
                    .text_size(text::SM)
                    .text_color(p.text_subtle)
                    .child(t)
            }))
            .children(
                d.error
                    .clone()
                    .map(|e| super::tabs::err_line("dv-jobs-error-copy", e, p)),
            );
    }
    out.push(section.into_any_element());
    out
}

/// Provider-unreachable: a cloud lane, retrying jobs, a transient last error.
fn unreachable(
    w: &DebugWindow,
    lane: Option<&Value>,
    config: Option<&Value>,
    cloud: bool,
    now: i64,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> Option<AnyElement> {
    let lane = lane?;
    let waiting = lane.num_at(&["retrying"]).unwrap_or(0.) as i64;
    if waiting == 0 || !cloud {
        return None;
    }
    let retrying: Vec<&Value> = w
        .detail
        .jobs
        .iter()
        .filter(|j| detail::job_state(j, now) == JobState::Retrying)
        .collect();
    let error = retrying
        .first()
        .and_then(|j| j.str_at(&["lastError"]))
        .or(lane.str_at(&["lastError"]));
    if !detail::is_transient(error) {
        return None;
    }
    let newest = retrying
        .iter()
        .filter_map(|j| j.str_at(&["updatedAt"]).and_then(parse_job_ts))
        .max();
    let provider = config
        .and_then(|c| c.str_at(&["provider"]).map(str::to_string))
        .or_else(|| retrying.first().and_then(|j| detail::job_provider(j)))
        .unwrap_or_else(|| "provider".into());
    let mut text_line = String::new();
    if let Some(ms) = newest {
        text_line.push_str(&format!("Timed out at {}. ", super::tabs::clock(ms, cx)));
    }
    text_line.push_str(&format!(
        "{waiting} job{} wait and retry on their own; waiting doesn\u{2019}t count as a failed try.",
        if waiting == 1 { "" } else { "s" }
    ));
    let test = w.detail.test.as_ref().map(|t| {
        let (tone, line) = match t {
            Ok(()) => (Tone::Ok.colors(p).0, "Reachable now.".to_string()),
            Err(e) => (p.danger_text, e.clone()),
        };
        div()
            .id("dv-test-result")
            .test_support()
            .mt(px(4.))
            .text_color(tone)
            .child(line)
    });
    let body = div()
        .text_size(text::BASE)
        .line_height(relative(1.45))
        .text_color(p.text_muted)
        .child(text_line)
        .children(test);
    let me = cx.entity().downgrade();
    let testing = w.detail.testing;
    let button = Button::new(
        "dv-test-provider",
        if testing { "Testing\u{2026}" } else { "Test" },
    )
    .sm()
    .disabled(testing)
    .on_click(move |_, _, cx| {
        let _ = me.update(cx, |w, cx| {
            w.job_act(super::data::JobCall::TestDeepgram, cx)
        });
    });
    Some(
        notice_el(
            Tone::Warn,
            "icons/cloud-off.svg",
            format!(
                "{} can\u{2019}t be reached",
                detail::provider_name(&provider)
            ),
            Some(body.into_any_element()),
            vec![
                CopyButton::icon(
                    "dv-unreachable-copy",
                    "Copy error",
                    error.unwrap_or_default().to_string(),
                )
                .into_any_element(),
                button.into_any_element(),
            ],
            p,
        )
        .into_any_element(),
    )
}

/// The table's fixed columns (Job · Recording · Status · Tries · Updated).
const COL_ID: gpui_kit::Pixels = px(72.);
const COL_STATUS: gpui_kit::Pixels = px(76.);
const COL_TRIES: gpui_kit::Pixels = px(32.);
const COL_UPDATED: gpui_kit::Pixels = px(78.);

fn cols(id: Div, subj: Div, status: Div, tries: Div, updated: Div) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            id.flex_none()
                .w(COL_ID)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap(),
        )
        .child(
            subj.flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap(),
        )
        .child(status.flex_none().w(COL_STATUS))
        .child(tries.flex_none().w(COL_TRIES).flex().justify_end())
        .child(updated.flex_none().w(COL_UPDATED).flex().justify_end())
}

fn head(p: &Palette) -> AnyElement {
    cols(
        div().child("Job"),
        div().child("Recording"),
        div().child("Status"),
        div().child("Tries"),
        div().child("Updated"),
    )
    .px(px(4.))
    .py(px(6.))
    .font_weight(theme::W_MEDIUM)
    .text_size(text::XS)
    .line_height(relative(1.))
    .text_color(p.text_subtle)
    .border_b_1()
    .border_color(p.hairline)
    .into_any_element()
}

fn job_row(
    job: &Value,
    now: i64,
    retrying: i64,
    window: &mut Window,
    cx: &mut Context<DebugWindow>,
    p: &Palette,
) -> AnyElement {
    let id = job.num_at(&["id"]).unwrap_or(0.) as i64;
    let st = detail::job_state(job, now);
    let msg = detail::job_message(job, st, now, retrying);
    let mono = |s: String, c| {
        div()
            .font_family(FONT_MONO)
            .text_size(text::SM)
            .line_height(relative(1.))
            .text_color(c)
            .child(s)
    };
    let h = focus_handle(nid("dv-job-focus", id as u64), window, cx);
    let visible = h.is_focused(window) && window.last_input_was_keyboard();
    let wash = p.wash;
    let open = job.clone();
    div()
        .id(nid("dv-job", id as u64))
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(SharedString::from(format!("Job {id}")))
        .track_focus(&h)
        .relative()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(4.))
        .py(px(7.))
        .border_b_1()
        .border_color(p.hairline)
        .cursor_pointer()
        .hover(move |s| s.bg(wash))
        .when(visible, |el| {
            el.child(
                ring_box(p.accent, px(2.), px(0.))
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0(),
            )
        })
        .on_click(cx.listener(move |w, _, window, cx| {
            w.open_job(open.clone(), cx);
            window.focus(&w.root_focus, cx);
        }))
        .child(cols(
            mono(format!("#{id}"), p.text_subtle),
            div()
                .text_size(text::BASE)
                .line_height(relative(1.3))
                .text_color(p.text)
                .child(detail::subject_label(job)),
            div().flex().child(chip(st.word(), chip_kind(st), p)),
            mono(
                (job.num_at(&["attemptCount"]).unwrap_or(0.) as i64).to_string(),
                p.text_muted,
            ),
            mono(job_ts(job.str_at(&["updatedAt"]), cx), p.text_subtle),
        ))
        .children(msg.map(|m| {
            // One line under the row; the tip has the whole message.
            let line = div()
                .id(nid("dv-job-msg", id as u64))
                .ml(COL_ID + px(10.))
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(text::SM)
                .line_height(relative(1.4))
                .text_color(if st == JobState::Failed {
                    p.danger_text
                } else {
                    p.text_subtle
                })
                .child(m.clone());
            with_tip(line, m)
        }))
        .into_any_element()
}
