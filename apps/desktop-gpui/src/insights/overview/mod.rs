//! Overview (`Overview.svelte` + `OverviewLede.svelte`): the period capsule and
//! Ask on the bar, the read beside four stats, the exhibits, and the feed.
//! Capture figures render for everyone; the read, categories and focus need the
//! engine, and capture-only mode greys the columns to app time.

mod exhibits;
mod modals;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

pub use modals::modal;

use super::model::{OverviewData, Status};
use super::parts::{self, display, lede, prose, read, stat, stat_text, stat_value, top_value};
use super::stats::{self, category_color, category_label, fmt_duration, signed_duration};
use super::Insights;
use crate::theme::{space, Palette};
use crate::ui::calendar::{capsule_label, period_of, Unit};
use crate::ui::date_input::DateInput;
use crate::ui::{clock, skeleton, Button};

/// AI on and reachable, or derivation running.
pub fn engine_on(s: &Status) -> bool {
    s.ai.as_ref().is_some_and(|a| a.enabled && a.available)
        || s.ctx.as_ref().is_some_and(|c| c.engine_available)
}

/// Capture-only: AI off, never set up, or continuous derivation off.
pub fn cap_only(s: &Status) -> bool {
    s.ai.as_ref()
        .is_some_and(|a| !a.has_providers || !a.enabled)
        || s.ctx.as_ref().and_then(|c| c.reason.as_deref()) == Some("user_context_disabled")
}

/// Ask can open (not pitched, not off).
pub fn engine_ready(s: &Status) -> bool {
    s.loaded && s.ai.as_ref().is_some_and(|a| a.has_providers && a.enabled)
}

fn at_latest(this: &Insights, cx: &Context<Insights>) -> bool {
    clock::now_ms(cx) < this.overview_window(cx).1
}

/// "this week" · "today" · "Oct 5 – 11".
pub fn scope(this: &Insights, cx: &Context<Insights>) -> String {
    let (d, unit) = this.period;
    if at_latest(this, cx) {
        return if unit == Unit::Day {
            "today".into()
        } else {
            format!("this {}", unit.name())
        };
    }
    let (a, b) = period_of(d, unit);
    capsule_label(Some(unit), a, b, clock::now(cx).date())
}

/// "week 45" · "November" · "Wed, Nov 4".
fn when(this: &Insights) -> String {
    let (a, _) = period_of(this.period.0, this.period.1);
    match this.period.1 {
        Unit::Week => format!("week {}", a.iso_week()),
        Unit::Month => stats::month_name(a),
        Unit::Day => format!(
            "{}, {} {}",
            &a.weekday().to_string()[..3],
            &a.month().to_string()[..3],
            a.day()
        ),
    }
}

pub fn controls(
    this: &Insights,
    p: &Palette,
    _: &mut Window,
    cx: &mut Context<Insights>,
) -> Vec<AnyElement> {
    let cap = cap_only(&this.status);
    let today = clock::now(cx).date();
    let min = this
        .status
        .ctx
        .as_ref()
        .and_then(|c| c.earliest_capture_at_ms)
        .map(|ms| stats::local(ms, clock::now(cx).offset()).date())
        .unwrap_or(today - time::Duration::days(365));
    let off = clock::now(cx).offset();
    let hours: Vec<i64> = this
        .overview
        .as_ref()
        .map(|o| o.capture_hours.clone())
        .unwrap_or_default();
    let density = move |d: time::Date| {
        let s = stats::date_ms(d, off);
        let n = hours
            .iter()
            .filter(|h| **h >= s && **h < s + stats::DAY_MS)
            .count();
        match n {
            0 => 0,
            1..=2 => 1,
            3..=5 => 2,
            _ => 3,
        }
    };
    let entity = cx.entity().downgrade();
    let ask = Button::new("insights-ask", format!("Ask about {}", scope(this, cx)))
        .ghost()
        .icon(IconName::Sparkle.path())
        .accent_icon()
        .kbd("⌘J")
        .soft_disabled(cap)
        .tip(if cap {
            "Ask needs AI — turn it on in Settings".to_string()
        } else {
            format!("Opens Chat with {} in scope", scope(this, cx))
        })
        .on_click(cx.listener(|this, _, _, cx| this.ask(cx)));
    let date = DateInput::new("insights-period", this.period.0, today)
        .period(this.period.1)
        .range(min, today)
        .density(density)
        .on_change(move |start, _, unit, _, cx| {
            entity
                .update(cx, |this, cx| {
                    this.set_period(start, unit.unwrap_or(Unit::Week), cx)
                })
                .ok();
        });
    let _ = p;
    vec![ask.into_any_element(), date.into_any_element()]
}

/// Per-period numbers the lede and exhibits share.
pub struct Figures {
    pub range: (i64, i64),
    pub acts: Vec<capture_types::Activity>,
    pub tracked: i64,
    pub apps: Vec<capture_types::AppUsage>,
    pub captured_days: usize,
    pub nothing: bool,
    pub cap_only: bool,
    pub engine_on: bool,
}

pub fn figures(this: &Insights, d: &OverviewData, cx: &Context<Insights>) -> Figures {
    let off = clock::now(cx).offset();
    let range = this.overview_window(cx);
    let usage = d.usage.as_ref();
    let mut days: Vec<i64> = usage
        .map(|u| {
            u.activity_heatmap
                .iter()
                .filter(|b| b.intensity_count > 0)
                .map(|b| stats::start_of_day(b.bucket_start_ms, off))
                .collect()
        })
        .unwrap_or_default();
    days.sort();
    days.dedup();
    Figures {
        acts: stats::in_range(&d.activities, range.0, range.1),
        tracked: usage
            .map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum())
            .unwrap_or(0),
        apps: usage
            .map(|u| stats::listed_apps(&u.time_per_app))
            .unwrap_or_default(),
        captured_days: days.len(),
        nothing: d.presence == Some((false, false)),
        cap_only: cap_only(&this.status),
        engine_on: engine_on(&this.status),
        range,
    }
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let Some(d) = this.overview.clone().filter(|_| this.status.loaded) else {
        return loading(this.width, p).into_any_element();
    };
    let f = figures(this, &d, cx);
    div()
        .flex()
        .flex_col()
        .child(read(
            lede_col(this, &d, &f, p, cx),
            stat_cells(this, &d, &f, p),
            this.width,
            p,
        ))
        .child(exhibits::exhibits(this, &d, &f, p, window, cx))
        .children((!f.cap_only).then(|| exhibits::feed(this, &d, &f, p, cx)))
        .into_any_element()
}

fn loading(width: gpui_kit::Pixels, p: &Palette) -> gpui_kit::Div {
    let lede = div()
        .flex()
        .flex_col()
        .gap(gpui_kit::px(12.))
        .child(skeleton::skel(p).w(gpui_kit::px(140.)).h(gpui_kit::px(9.)))
        .child(
            skeleton::skel(p)
                .h(gpui_kit::px(34.))
                .w(gpui_kit::relative(0.78)),
        )
        .child(
            skeleton::skel(p)
                .h(gpui_kit::px(34.))
                .w(gpui_kit::relative(0.52)),
        )
        .child(skeleton::lines(2, p));
    let stats = (0..4)
        .map(|_| {
            div()
                .flex()
                .flex_col()
                .gap(gpui_kit::px(8.))
                .w(gpui_kit::px(132.))
                .child(
                    skeleton::skel(p)
                        .h(gpui_kit::px(8.))
                        .w(gpui_kit::relative(0.6)),
                )
                .child(
                    skeleton::skel(p)
                        .h(gpui_kit::px(24.))
                        .w(gpui_kit::relative(0.9)),
                )
                .into_any_element()
        })
        .collect();
    div().child(read(lede, stats, width, p))
}

fn lede_col(
    this: &Insights,
    d: &OverviewData,
    f: &Figures,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> gpui_kit::Div {
    let latest = at_latest(this, cx);
    let unit = this.period.1.name();
    let scope = scope(this, cx);
    let kicker = format!(
        "{} · {}",
        if latest {
            "The read so far"
        } else {
            "The read"
        },
        when(this)
    );
    let plain = format!("This {unit} · {}", when(this));
    let digest = d
        .digest
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|r| r.digest.clone());
    let reason = d
        .digest
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|r| r.reason);
    let digest_err = d.digest.as_ref().and_then(|r| r.as_ref().err());
    let off = clock::now(cx).offset();
    let (running, paused) = this.recording;
    if f.nothing && f.acts.is_empty() && latest && (!running || paused) {
        // 2a: the current range is empty and Mnema isn't recording (or is paused).
        let (body, label) = if paused {
            ("Recording is paused, so there's nothing to read or chart yet. Resume and the first summary lands after about 10 minutes of work.", "Resume recording")
        } else {
            ("Mnema isn't recording yet, so there's nothing to read or chart. Start recording and the first summary lands after about 10 minutes of work; the read follows once there are two activities.", "Start recording")
        };
        return lede(&plain, p)
            .child(display(format!("Nothing captured {scope}."), None, p))
            .child(prose(body, p))
            .child(div().mt(space::S5).flex().child(
                Button::new("ov-start-recording", label).primary().on_click(
                    cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::StartRecording)),
                ),
            ));
    }
    if f.nothing && f.acts.is_empty() {
        let body = if latest {
            format!("Mnema wasn't recording {scope} yet, so there's nothing to read. Step to another {unit}, or start recording.")
        } else {
            format!("Mnema wasn't recording in this period, so there's nothing to read. Step to another {unit}, or start recording.")
        };
        return lede(&plain, p)
            .child(display(format!("Nothing captured {scope}."), None, p))
            .child(prose(body, p));
    }
    if f.cap_only {
        let head = if d.usage_error.is_some() && d.usage.is_none() {
            display(format!("Couldn't load app time {scope}."), None, p)
        } else {
            let n = f.apps.len();
            let top = f.apps.first().map(|a| format!("Most of it in {}.", a.app));
            display(
                format!(
                    "{} tracked across {n} {}.",
                    fmt_duration(f.tracked),
                    if n == 1 { "app" } else { "apps" }
                ),
                top,
                p,
            )
        };
        return lede(&plain, p).child(head);
    }
    if let Some(dg) = digest {
        let h = dg.headline.unwrap_or_default();
        let (head, turn) = match h.trim().split_once(" — ") {
            Some((a, b)) => (a.to_string(), Some(format!("—\u{a0}{b}"))),
            None => (h.trim().to_string(), None),
        };
        let written = if stats::start_of_day(dg.generated_at_ms, off)
            == stats::start_of_day(clock::now_ms(cx), off)
        {
            stats::clock(dg.generated_at_ms, off)
        } else {
            format!(
                "{}, {}",
                stats::month_day(dg.generated_at_ms, off),
                stats::clock(dg.generated_at_ms, off)
            )
        };
        return lede(&kicker, p)
            .when(!head.is_empty(), |l| l.child(display(head, turn, p)))
            .child(prose(dg.narrative, p))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .mt(space::S5)
                    .child(parts::mono_label(format!("written {written}"), p))
                    .children(
                        watermark(this, f, cx).map(|w| parts::mono_label(format!("· {w}"), p)),
                    )
                    .child(reread(this, cx)),
            );
    }
    if !f.engine_on {
        return lede(&kicker, p)
            .child(display("No new read while AI is paused.", None, p))
            .child(prose(
                "Everything below was written before. The read comes back on its own.",
                p,
            ));
    }
    if let Some(e) = digest_err {
        log::debug!("insights: digest failed ({} chars)", e.len());
        return lede(&kicker, p)
            .child(display(
                format!("Couldn't write the read for {scope}."),
                None,
                p,
            ))
            .child(div().mt(space::S5).child(reread(this, cx)));
    }
    if reason == Some(capture_types::DigestAbsentReason::TooFew) {
        let n = f.acts.len();
        let tail = if n < 2 {
            format!(
                "; this {unit} has {} so far",
                if n == 1 { "one" } else { "none" }
            )
        } else {
            " it can use".into()
        };
        return lede(&kicker, p)
            .child(display(format!("Too little captured {scope} to write a read."), None, p))
            .child(prose(format!("A read needs at least two activities{tail}. The numbers on the right still count."), p));
    }
    lede(&kicker, p)
        .child(display(format!("No read for {scope} yet."), None, p))
        .child(prose(
            "It's written on its own once there's enough. Re-read asks for one now.",
            p,
        ))
        .child(div().mt(space::S5).child(reread(this, cx)))
}

/// "summarized through 2:20 PM", only while the watermark cuts into the range.
fn watermark(this: &Insights, f: &Figures, cx: &Context<Insights>) -> Option<String> {
    let covered = this.status.ctx.as_ref()?.covered_until_ms?;
    let now = clock::now_ms(cx);
    let inside = covered >= f.range.0 && covered < f.range.1.min(now);
    (f.engine_on && inside && !f.acts.is_empty()).then(|| {
        format!(
            "summarized through {}",
            stats::clock(covered, clock::now(cx).offset())
        )
    })
}

fn reread(this: &Insights, cx: &Context<Insights>) -> Button {
    let me = cx.entity().downgrade();
    Button::new("insights-reread", "Re-read")
        .ghost()
        .sm()
        .icon(IconName::RotateCcw.path())
        .disabled(this.rereading)
        .on_click(move |_, _, cx| {
            let _ = me.update(cx, |i, cx| i.reread(cx));
        })
}

fn stat_cells(this: &Insights, d: &OverviewData, f: &Figures, p: &Palette) -> Vec<AnyElement> {
    let prev_word = if this.period.1 == Unit::Day {
        "yesterday".to_string()
    } else {
        format!("last {}", this.period.1.name())
    };
    let usage_failed = d.usage_error.is_some() && d.usage.is_none();
    let too_few = d
        .digest
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(|r| r.reason)
        == Some(capture_types::DigestAbsentReason::TooFew);
    let has_digest = d
        .digest
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .is_some_and(|r| r.digest.is_some());
    let prev = d
        .prev_usage
        .as_ref()
        .map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum::<i64>());
    let delta = prev.filter(|v| *v > 0).map(|v| f.tracked - v);
    let mut out = Vec::new();
    let tracked_delta = if usage_failed {
        Some("couldn't load".into())
    } else if f.nothing {
        Some("no capture".into())
    } else if f.cap_only || !too_few || has_digest {
        delta.map(|dm| format!("{} vs {prev_word}", signed_duration(dm)))
    } else {
        Some("app active time".into())
    };
    let up = delta.is_some_and(|x| x > 0) && !f.cap_only;
    let value = if usage_failed {
        "—".into()
    } else {
        fmt_duration(f.tracked)
    };
    out.push(stat(
        "Tracked · app time",
        stat_value(&value, p),
        tracked_delta,
        up,
        p,
    ));
    if !f.cap_only && too_few && !has_digest && !f.nothing {
        out.push(stat_text(
            "Activities",
            &f.acts.len().to_string(),
            Some("a read needs 2".into()),
            p,
        ));
    } else {
        let (v, delta) = if usage_failed {
            ("—".into(), Some("couldn't load".into()))
        } else if f.nothing {
            ("—".into(), None)
        } else {
            let avg = if f.captured_days > 0 {
                f.tracked / f.captured_days as i64
            } else {
                0
            };
            let n = f.captured_days;
            (
                fmt_duration(avg),
                Some(format!(
                    "over {n} captured day{}",
                    if n == 1 { "" } else { "s" }
                )),
            )
        };
        out.push(stat_text("Daily avg", &v, delta, p));
        if !f.cap_only && !f.nothing {
            let c = stats::focus_counts(&f.acts);
            let deep = if d.engine_error.is_some() {
                stat_text(
                    "Deep focus · by activity",
                    "—",
                    Some("couldn't load".into()),
                    p,
                )
            } else if c.n == 0 {
                stat_text(
                    "Deep focus · by activity",
                    "—",
                    Some("no focus read yet".into()),
                    p,
                )
            } else {
                let dd = stats::deep_focus_delta(&f.acts, &d.prev_activities);
                let pts = dd
                    .map(|x| format!(" · {}{} pts", if x >= 0 { "+" } else { "−" }, x.abs()))
                    .unwrap_or_default();
                let n = c.n;
                stat(
                    "Deep focus · by activity",
                    stat_value(&format!("{}%", stats::deep_pct(&f.acts).unwrap_or(0)), p),
                    Some(format!(
                        "{} of {n} activit{}{pts}",
                        c.deep,
                        if n == 1 { "y" } else { "ies" }
                    )),
                    dd.is_some_and(|x| x > 0),
                    p,
                )
            };
            out.push(deep);
            let top = stats::top_category(&f.acts, f.range.0, f.range.1);
            out.push(match top {
                Some(t) => stat(
                    "Top category",
                    top_value(category_label(t.category), category_color(t.category, p), p),
                    Some(format!("{}% of activity time", t.pct)),
                    false,
                    p,
                ),
                None => stat_text("Top category", "—", None, p),
            });
        }
    }
    out.into_iter().map(|e| e.into_any_element()).collect()
}
