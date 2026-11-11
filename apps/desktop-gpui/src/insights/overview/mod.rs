//! Overview (`Overview.svelte` + `OverviewLede.svelte`): the period capsule and
//! Ask on the bar, the read beside four stats, the exhibits, and the feed.
//! Capture figures render for everyone; the read, categories and focus need the
//! engine, and capture-only mode greys the columns to app time.

mod correct;
mod exhibits;
mod modals;

pub use correct::{Fix, Ui};
#[cfg(test)]
pub use modals::threads;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

pub use modals::modal;

use super::model::{OverviewData, Status};
use super::parts::{self, display, lede, prose, read, stat, stat_text, stat_value, top_value};
use super::read::{self, Of};
use super::stats::{self, category_color, category_label, fmt_duration, signed_duration};
use super::Insights;
use capture_types::DigestAbsentReason;
use crate::theme::{space, Palette};
use crate::ui::calendar::{capsule_label, period_of, Unit};
use crate::ui::date_input::DateInput;
use crate::ui::{clock, skeleton, Button, Tone};

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
        .align_end()
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
    let Some(d) = this
        .overview
        .clone()
        .filter(|o| this.status.loaded && !o.stale)
    else {
        return loading(this.width, p).into_any_element();
    };
    let f = figures(this, &d, cx);
    div()
        .flex()
        .flex_col()
        .children(failing(this, &f, p, clock::now(cx).offset()))
        .child(read(
            lede_col(this, &d, &f, p, cx),
            stat_cells(this, &d, &f, p, cx),
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
        if latest { "The read so far" } else { "The read" },
        when(this)
    );
    let plain = format!("This {unit} · {}", when(this));
    let r = this.read_of(Of::Overview);
    let off = clock::now(cx).offset();
    let now = clock::now_ms(cx);
    let (running, paused) = this.recording;
    let paused_engine = this.status.loaded && !f.cap_only && !f.engine_on;
    let held = r.error.as_deref() == Some(read::SENSITIVE_HOLD);
    let fail = r.error.as_deref().filter(|_| !held).map(read::fail_text);
    let paused_by = match r.reason {
        Some(DigestAbsentReason::DerivationOff) => Some("continuous derivation is off"),
        Some(DigestAbsentReason::EngineNotReady) => Some("AI is paused"),
        _ if paused_engine => Some("AI is paused"),
        _ => None,
    };
    let retry = |id: &'static str| {
        let me = cx.entity().downgrade();
        Button::new(id, "Retry").ghost().sm().on_click(move |_, _, cx| {
            let _ = me.update(cx, |i, cx| i.reread(Of::Overview, cx));
        })
    };
    let lede_el = if f.nothing && f.acts.is_empty() {
        let l = lede(&plain, p).child(display(format!("Nothing captured {scope}."), None, p));
        if latest && (!running || paused) {
            // 2a: the current range is empty and Mnema isn't recording (or is paused).
            let (body, label) = if paused {
                ("Recording is paused, so there's nothing to read or chart yet. Resume and the first summary lands after about 10 minutes of work.", "Resume recording")
            } else {
                ("Mnema isn't recording yet, so there's nothing to read or chart. Start recording and the first summary lands after about 10 minutes of work; the read follows once there are two activities.", "Start recording")
            };
            l.child(prose(body, p)).child(div().mt(space::S5).flex().child(
                Button::new("ov-start-recording", label).primary().on_click(
                    cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::StartRecording)),
                ),
            ))
        } else {
            let body = if latest {
                format!("Mnema wasn't recording {scope} yet, so there's nothing to read. Step to another {unit}, or start recording.")
            } else {
                format!("Mnema wasn't recording in this period, so there's nothing to read. Step to another {unit}, or start recording.")
            };
            l.child(prose(body, p))
        }
    } else if f.cap_only {
        let l = lede(&plain, p);
        if d.usage_error.is_some() && d.usage.is_none() {
            l.child(display(format!("Couldn't load app time {scope}."), None, p))
        } else if let Some((_, note)) = no_app_time(this, d, f, cx) {
            l.child(display(format!("No app time {scope}."), None, p))
                .child(prose(note, p))
        } else {
            let n = f.apps.len();
            let top = f.apps.first().map(|a| format!("Most of it in {}.", a.app));
            l.child(display(
                format!(
                    "{} tracked across {n} {}.",
                    fmt_duration(f.tracked),
                    if n == 1 { "app" } else { "apps" }
                ),
                top,
                p,
            ))
        }
    } else if let (Some(by), None) = (paused_by, &r.digest) {
        lede(&kicker, p)
            .child(display(format!("No new read while {by}."), None, p))
            .child(prose(
                if r.reason == Some(DigestAbsentReason::DerivationOff) {
                    "Everything below was written before."
                } else {
                    "Everything below was written before. The read comes back on its own."
                },
                p,
            ))
    } else if let Some(dg) = r.digest.clone() {
        let h = dg.headline.unwrap_or_default();
        let (head, turn) = match h.trim().split_once(" — ") {
            Some((a, b)) => (a.to_string(), Some(format!("—\u{a0}{b}"))),
            None => (h.trim().to_string(), None),
        };
        let written = if stats::start_of_day(dg.generated_at_ms, off) == stats::start_of_day(now, off) {
            stats::clock(dg.generated_at_ms, off)
        } else {
            format!(
                "{}, {}",
                stats::month_day(dg.generated_at_ms, off),
                stats::clock(dg.generated_at_ms, off)
            )
        };
        let failed_refresh = r.error.is_some() && !r.regenerating;
        let foot = div().flex().items_center().gap(space::S3).mt(space::S5);
        let foot = if failed_refresh {
            // A failed Re-read keeps the read we had (OV-06/07) and says why.
            let why = if held {
                "the new read touched a private category.".to_string()
            } else {
                fail.clone().unwrap_or_default()
            };
            foot.child(danger(format!("Couldn't re-read: {why}"), retry("insights-reread-retry"), p))
        } else {
            foot.child(parts::mono_label(format!("written {written}"), p))
                .children(watermark(this, f, cx).map(|w| parts::mono_label(format!("· {w}"), p)))
                .when(!paused_engine, |d| d.child(reread(this, cx)))
        };
        lede(&kicker, p)
            .when(!head.is_empty(), |l| l.child(display(head.clone(), turn, p)))
            .when(!failed_refresh || head.is_empty(), |l| l.child(prose(dg.narrative, p)))
            .child(foot)
    } else if r.loading || r.regenerating || this.overview.as_ref().is_some_and(|o| o.stale) {
        lede(&kicker, p).child(parts::writing_skel(p)).child(
            div()
                .mt(space::S5)
                .child(parts::busy_line(format!("Writing the read for {scope}…"), p)),
        )
    } else if held {
        lede(&kicker, p)
            .child(display(format!("The read for {scope} was held back."), None, p))
            .child(prose(format!("It touched a private category, so Mnema dropped it rather than show it. Your activities are untouched. Re-read, or step to another {unit}."), p))
            .child(div().mt(space::S5).child(reread(this, cx)))
    } else if let Some(t) = fail {
        lede(&kicker, p)
            .child(display(format!("Couldn't write the read for {scope}."), None, p))
            .child(div().mt(space::S5).child(danger(t, retry("insights-read-retry"), p)))
    } else if waiting_first(this, d, f, cx) {
        lede(&kicker, p)
            .child(display("Waiting for the first summary.", None, p))
            .child(prose(format!("{}The first summary usually lands within about 10 minutes, and the read is written once there are two activities.", recording_since(this, cx).map(|t| format!("Recording since {t}. ")).unwrap_or_default()), p))
    } else if let Some(note) = older_note(this, f, cx) {
        lede(&kicker, p)
            .child(display(format!("No read for this {unit}."), None, p))
            .child(prose(note, p))
    } else if r.reason == Some(DigestAbsentReason::TooFew) {
        let n = f.acts.len();
        let tail = if n < 2 {
            format!("; this {unit} has {} so far", if n == 1 { "one" } else { "none" })
        } else {
            " it can use".into()
        };
        lede(&kicker, p)
            .child(display(format!("Too little captured {scope} to write a read."), None, p))
            .child(prose(format!("A read needs at least two activities{tail}. The numbers on the right still count."), p))
    } else {
        lede(&kicker, p)
            .child(display(format!("No read for {scope} yet."), None, p))
            .child(prose("It's written on its own once there's enough. Re-read asks for one now.", p))
            .child(div().mt(space::S5).child(reread(this, cx)))
    };
    // 4a: history backfill still running.
    let catching_up = !f.cap_only && f.engine_on && this.status.ctx.as_ref().is_some_and(|c| c.backfilling);
    lede_el.when(catching_up, |l| {
        l.child(div().mt(space::S5).child(parts::busy_line(
            "Catching up on older history — past days fill in as it goes.",
            p,
        )))
    })
}

/// An `mx-inline` danger line with its Retry.
fn danger(text_: String, retry: Button, p: &Palette) -> gpui_kit::Div {
    crate::ui::notice::inline(
        Tone::Danger,
        Some(crate::ui::icon(IconName::TriangleAlert.path(), gpui_kit::px(13.)).into_any_element()),
        vec![div().child(text_).into_any_element(), retry.into_any_element()],
        p,
    )
}

/// Why there's no app time (`noAppTime`): `(stat caption, note)`.
pub fn no_app_time(
    this: &Insights,
    d: &OverviewData,
    f: &Figures,
    cx: &Context<Insights>,
) -> Option<(String, String)> {
    let (screen, audio) = d.presence?;
    if screen {
        return None;
    }
    let policy = this.status.settings.as_ref().map(|s| &s.retention_policy);
    match parts::retention(f.range.0, f.range.1, policy, clock::now_ms(cx)) {
        Some((true, days)) => Some((
            "tracked · removed by retention".into(),
            format!("Removed by your {days}-day retention, so there's no app time. The summaries and the read are kept."),
        )),
        _ if audio => Some((
            "app time · audio only".into(),
            "This range has audio only — app time comes from screen recording.".into(),
        )),
        _ => None,
    }
}

/// The days the worker summarizes on its own (`null` = go deeper: all).
fn backfill_days(this: &Insights) -> Option<i64> {
    let u = &this.status.settings.as_ref()?.user_context;
    (!u.backfill_go_deeper).then_some(u.backfill_window_days as i64)
}

/// OV-20: older than the window the worker summarizes on its own.
fn older_note(this: &Insights, f: &Figures, cx: &Context<Insights>) -> Option<String> {
    let days = backfill_days(this)?;
    let now = clock::now_ms(cx);
    if !f.engine_on || f.range.1 > now - days * stats::DAY_MS || !f.acts.is_empty() {
        return None;
    }
    let policy = this.status.settings.as_ref().map(|s| &s.retention_policy);
    let removed = matches!(parts::retention(f.range.0, f.range.1, policy, now), Some((true, _)));
    Some(format!(
        "Older than the {days} days Mnema summarizes on its own.{}",
        if removed { "" } else { " The footage is still on the Timeline." }
    ))
}

/// "Recording since": the session's start, clamped to the start of today
/// ("2:10 PM" today, "Oct 3, 2:10 PM" otherwise, as `clockLabel`).
fn recording_since(this: &Insights, cx: &Context<Insights>) -> Option<String> {
    let off = clock::now(cx).offset();
    let now = clock::now_ms(cx);
    let t = this.recording_since?.max(stats::start_of_day(now, off));
    Some(if stats::start_of_day(t, off) == stats::start_of_day(now, off) {
        stats::clock(t, off)
    } else {
        format!("{}, {}", stats::month_day(t, off), stats::clock(t, off))
    })
}

/// 2b: recording, but the worker hasn't written an activity for this range.
fn waiting_first(this: &Insights, d: &OverviewData, f: &Figures, cx: &Context<Insights>) -> bool {
    let (running, paused) = this.recording;
    f.engine_on && at_latest(this, cx) && running && !paused && d.engine_error.is_none() && f.acts.is_empty()
}

/// The Category / Focus tiles' copy when the range has nothing summarized (OV-23).
pub fn engine_empty_note(this: &Insights, d: &OverviewData, f: &Figures, cx: &Context<Insights>) -> Option<String> {
    if older_note(this, f, cx).is_some() {
        let days = backfill_days(this).unwrap_or(0);
        Some(format!("Not summarized — this {} is older than {days} days.", this.period.1.name()))
    } else if waiting_first(this, d, f, cx) {
        Some("Fills in after the first summary.".into())
    } else if at_latest(this, cx) {
        None
    } else {
        Some("Nothing summarized for this range.".into())
    }
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
    let r = this.read_of(Of::Overview);
    let b = Button::new("insights-reread", if r.regenerating { "Re-reading…" } else { "Re-read" })
        .ghost()
        .sm()
        .disabled(r.regenerating || r.loading)
        .on_click(move |_, _, cx| {
            let _ = me.update(cx, |i, cx| i.reread(Of::Overview, cx));
        });
    if r.regenerating {
        b.spinner(true)
    } else {
        b.icon(IconName::RotateCcw.path())
    }
}

/// 4b: summarizing keeps failing — the cause, not "still learning…" forever.
fn failing(this: &Insights, f: &Figures, p: &Palette, off: time::UtcOffset) -> Option<AnyElement> {
    let ctx = this.status.ctx.as_ref()?;
    let fl = ctx.summarizing_failure.as_ref().filter(|_| f.engine_on)?;
    let since = ctx
        .covered_until_ms
        .map(|c| format!("; everything since {} is on the Timeline", stats::clock(c, off)))
        .unwrap_or_default();
    let text_ = format!(
        "{} turned away the last {}. {} Recording isn't affected{since}.",
        fl.provider.as_deref().unwrap_or("The AI provider"),
        if fl.failures == 1 { "try".to_string() } else { format!("{} tries", fl.failures) },
        fl.reason
    );
    let open = Button::new("insights-failing-open", "Open engine settings")
        .sm()
        .on_click(|_, _, cx| crate::window::open_settings(Some("intelligence"), cx));
    Some(
        div()
            .id("insights-failing")
            .role(gpui_kit::Role::Alert)
            .mb(space::S5)
            .child(crate::ui::notice::notice(
                Tone::Danger,
                IconName::TriangleAlert.path(),
                "Summaries are failing",
                Some(text_.into()),
                vec![open.into_any_element()],
                p,
            ))
            .into_any_element(),
    )
}

fn stat_cells(
    this: &Insights,
    d: &OverviewData,
    f: &Figures,
    p: &Palette,
    cx: &Context<Insights>,
) -> Vec<AnyElement> {
    let no_app = no_app_time(this, d, f, cx);
    let policy = this.status.settings.as_ref().map(|s| &s.retention_policy);
    let partly = matches!(
        parts::retention(f.range.0, f.range.1, policy, clock::now_ms(cx)),
        Some((false, _))
    ) && d.presence.is_some();
    let prev_word = if this.period.1 == Unit::Day {
        "yesterday".to_string()
    } else {
        format!("last {}", this.period.1.name())
    };
    let usage_failed = d.usage_error.is_some() && d.usage.is_none();
    let r = this.read_of(Of::Overview);
    let too_few = r.reason == Some(DigestAbsentReason::TooFew);
    let has_digest = r.digest.is_some();
    let prev = d
        .prev_usage
        .as_ref()
        .map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum::<i64>());
    let delta = prev.filter(|v| *v > 0).map(|v| f.tracked - v);
    let mut out = Vec::new();
    let tracked_delta = if usage_failed {
        Some("couldn't load".into())
    } else if let Some((cap, _)) = &no_app {
        Some(cap.clone())
    } else if f.nothing {
        Some("no capture".into())
    } else if f.cap_only || !too_few || has_digest {
        if partly {
            Some("partly removed by retention".into())
        } else {
            delta.map(|dm| format!("{} vs {prev_word}", signed_duration(dm)))
        }
    } else {
        Some("app active time".into())
    };
    let up = delta.is_some_and(|x| x > 0) && !f.cap_only;
    let value = if usage_failed || no_app.is_some() {
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
        } else if f.nothing || no_app.is_some() {
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
