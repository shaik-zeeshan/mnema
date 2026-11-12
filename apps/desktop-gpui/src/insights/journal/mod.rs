//! Journal (`DayTimeline.svelte`): one local day. The day capsule + Today on
//! the bar, `[` / `]` step days; the digest lede beside four stats; the day
//! spine; the river of cards (newest first, each expanding into its receipt)
//! beside the week strip, the day mix and apps.

pub mod audio;
mod audio_view;
pub mod day;
pub mod receipt;
mod river;
mod spine;

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

use super::model::{JournalData, Tab};
use super::overview::engine_on;
use super::parts::{
    display, lede, mono_label, prose, read, stat, stat_text, stat_value, top_value,
};
use super::stats::{self, category_color, category_label, humanize_ms};
use super::read::Of;
use super::Insights;
use capture_types::DigestAbsentReason;
use crate::theme::{space, Palette};
use crate::ui::calendar::Unit;
use crate::ui::date_input::DateInput;
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::{clock, Button, Tone};

pub fn controls(this: &Insights, _: &Palette, cx: &mut Context<Insights>) -> Vec<AnyElement> {
    let today = clock::now(cx).date();
    let off = clock::now(cx).offset();
    let min = this
        .status
        .ctx
        .as_ref()
        .and_then(|c| c.earliest_capture_at_ms)
        .map(|ms| stats::local(ms, off).date())
        .unwrap_or(today - time::Duration::days(120));
    let heat: Vec<(i64, i64)> = this
        .journal
        .as_ref()
        .and_then(|j| j.history.as_ref())
        .map(|h| day::captured_hours(&h.activity_heatmap, off))
        .unwrap_or_default();
    let density = move |d: time::Date| {
        let s = stats::date_ms(d, off);
        match heat.iter().find(|(k, _)| *k == s).map_or(0, |h| h.1) {
            0 => 0,
            1..=2 => 1,
            3..=5 => 2,
            _ => 3,
        }
    };
    let entity = cx.entity().downgrade();
    let date = DateInput::new("insights-day", this.day, today)
        .align_end()
        .range(min.min(this.day), today)
        .density(density)
        .on_change(move |start, _, _, _, cx| {
            entity.update(cx, |this, cx| this.set_day(start, cx)).ok();
        });
    let at_today = this.day >= today;
    let today_btn = Button::new("insights-today", "Today")
        .disabled(at_today)
        .on_click(cx.listener(move |this, _, _, cx| this.set_day(today, cx)));
    vec![date.into_any_element(), today_btn.into_any_element()]
}

/// `[` / `]` on the Journal tab.
pub fn on_key(this: &mut Insights, key: &str, cx: &mut Context<Insights>) -> bool {
    if this.tab != Tab::Journal {
        return false;
    }
    match key {
        "[" => this.step_day(-1, cx),
        "]" => this.step_day(1, cx),
        _ => return false,
    }
    true
}

const PITCH: &str = "Your journal is written by AI";

fn center(title: String, body: &str, acts: Vec<AnyElement>, p: &Palette) -> AnyElement {
    center_tone(title, body, acts, Tone::Neutral, p)
}

fn center_tone(title: String, body: &str, acts: Vec<AnyElement>, tone: Tone, p: &Palette) -> AnyElement {
    div()
        .pt(space::S6)
        .flex()
        .justify_center()
        .child(empty(
            EmptyLayout::Center,
            tone,
            if tone == Tone::Danger {
                IconName::TriangleAlert.path()
            } else if title == PITCH {
                IconName::Sparkle.path()
            } else {
                IconName::Inbox.path()
            },
            title,
            body.to_string(),
            acts,
            p,
        ))
        .into_any_element()
}

pub fn render(
    this: &mut Insights,
    p: &Palette,
    _: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let Some(j) = this
        .journal
        .clone()
        .filter(|j| this.status.loaded && !j.stale)
    else {
        return div()
            .id("insights-journal-loading")
            .child(crate::ui::skeleton::lines(4, p))
            .into_any_element();
    };
    let off = clock::now(cx).offset();
    let now = clock::now_ms(cx);
    let range = stats::window(this.day, Unit::Day, off);
    let day_label = stats::day_label(range.0, off);
    let at_latest = now < range.1;
    if let Some(e) = &j.error {
        log::debug!("insights: journal load failed ({} chars)", e.len());
        let retry = Button::new("insights-journal-retry", "Try again")
            .sm()
            .icon(IconName::RotateCcw.path())
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));
        let timeline = Button::new("insights-journal-error-timeline", "Open in Timeline")
            .ghost()
            .sm()
            .on_click(cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::OpenTimeline)));
        return center_tone(
            format!("Couldn't load {day_label}"),
            "Reading this day's activities failed. Nothing is lost — the footage is still on the Timeline.",
            vec![retry.into_any_element(), timeline.into_any_element()],
            Tone::Danger,
            p,
        );
    }
    let view = day_view(this, &j, range, now, at_latest);
    let (model, cards) = (&view.0, &view.1);
    let ctx = this.status.ctx.as_ref();
    let pitch = this.status.ai.as_ref().is_some_and(|a| !a.has_providers);
    let (running, paused) = this.recording;
    if pitch && cards.is_empty() {
        return center(
            PITCH.into(),
            "Once a provider is connected, Mnema writes up what you do as activities every few minutes — with a digest of the day on top. Capture is already on the Timeline.",
            vec![
                Button::new("insights-journal-setup", "Set up AI")
                    .sm()
                    .on_click(|_, _, cx| crate::window::open_settings(Some("intelligence"), cx))
                    .into_any_element(),
                Button::new("insights-journal-timeline", "Open Timeline")
                    .ghost()
                    .sm()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::OpenTimeline)))
                    .into_any_element(),
            ],
            p,
        );
    }
    let live = at_latest && running && !paused;
    // Capture-run shape: captured, nothing written up, nothing being derived.
    let no_derive = !ctx.is_some_and(|c| c.engine_available);
    let capture_run = cards.is_empty() && model.has_capture && no_derive;
    let pending = model.pending.active || (live && cards.is_empty());
    if cards.is_empty() && !pending && !capture_run {
        let policy = this.status.settings.as_ref().map(|s| &s.retention_policy);
        let retention = super::parts::retention(range.0, range.1, policy, now);
        let backfill = this
            .status
            .settings
            .as_ref()
            .filter(|s| !s.user_context.backfill_go_deeper)
            .map(|s| s.user_context.backfill_window_days as i64);
        let mut acts = Vec::new();
        let (title, body) = if !model.has_capture && at_latest && (!running || paused) {
            let label = if paused { "Resume recording" } else { "Start recording" };
            acts.push(
                Button::new("insights-journal-record", label)
                    .sm()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::StartRecording)))
                    .into_any_element(),
            );
            (
                if paused { "Recording is paused" } else { "Mnema isn't recording" }.to_string(),
                format!("{} and today's journal fills in as you work. The first card usually lands within about 10 minutes.", if paused { "Resume" } else { "Start recording" }),
            )
        } else if !model.has_capture {
            match retention {
                Some((true, days)) => (format!("Nothing left from {day_label}"), format!("Removed by your {days}-day retention.")),
                _ => (format!("Nothing captured on {day_label}"), "There's no capture on this day, so there's no journal to show. Days with any recording at all show whatever was captured.".into()),
            }
        } else if let Some(days) = backfill.filter(|d| range.1 <= now - d * stats::DAY_MS) {
            let kept = !matches!(retention, Some((true, _)));
            ("This day wasn't summarized".into(), format!("Older than the {days} days Mnema summarizes on its own.{}", if kept { " The footage is still on the Timeline." } else { "" }))
        } else if ctx.is_some_and(|c| c.backfilling) {
            ("Not summarized yet".into(), "Mnema is catching up on older history, newest first — past days fill in as it goes. The footage is already on the Timeline.".into())
        } else {
            (format!("No cards for {day_label}"), "Nothing in this day's capture was summarized into a card. The footage is on the Timeline.".into())
        };
        if acts.is_empty() && !at_latest {
            let today = clock::now(cx).date();
            acts.push(
                Button::new("insights-go-today", "Go to today")
                    .sm()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_day(today, cx)))
                    .into_any_element(),
            );
        }
        return center(title, &body, acts, p);
    }
    let why = capture_run.then(|| {
        if ctx.and_then(|c| c.reason.as_deref()) == Some("user_context_disabled") {
            "Continuous derivation is off, so nothing was written up."
        } else if this.status.ai.as_ref().is_some_and(|a| !a.enabled) {
            "AI features were off, so nothing was written up."
        } else {
            "AI wasn't available, so nothing was written up."
        }
    });
    let runs: Option<Vec<spine::Run>> = why.map(|_| {
        let frames: Vec<i64> = j.frames.iter().map(|f| f.1).collect();
        let raw: Vec<(i64, i64)> = day::capture_runs(&frames, &j.audio)
            .into_iter()
            .map(|(s, e)| (s.max(range.0), e.min(range.1)))
            .collect();
        let apps = this
            .run_apps
            .as_ref()
            .filter(|r| r.0 == this.day)
            .map(|r| r.1.clone())
            .unwrap_or_default();
        if this.run_apps.as_ref().is_none_or(|r| r.0 != this.day) {
            this.load_run_apps(raw.clone(), cx);
        }
        raw.into_iter()
            .enumerate()
            .map(|(i, (s, e))| (s, e, apps.get(i).cloned().unwrap_or_default()))
            .collect()
    });
    let shown = day::shown_activities(cards);
    let day_acts = stats::in_range(&j.activities, range.0, range.1);
    let tracked: i64 = j
        .usage
        .as_ref()
        .map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum())
        .unwrap_or(0);
    div()
        .flex()
        .flex_col()
        .child(read(
            lede_col(
                this,
                why,
                &model,
                cards.len(),
                tracked,
                at_latest,
                &day_label,
                p,
                cx,
            ),
            lede_stats(
                &j,
                why.is_some(),
                &model,
                &day_acts,
                &shown,
                cards.len(),
                tracked,
                at_latest,
                now,
                range,
                p,
                off,
            ),
            this.width,
            p,
        ))
        .child(spine::render(
            this,
            &model,
            &cards,
            runs.as_deref(),
            at_latest.then_some(now),
            p,
            off,
            cx,
        ))
        .child(river::render(
            this,
            &j,
            &model,
            &cards,
            &shown,
            (range.0, why, pending),
            p,
            cx,
        ))
        .into_any_element()
}

/// What the day model is built from besides the journal itself.
type DayKey = (
    (i64, i64),
    Option<i64>,
    bool,
    Option<String>,
    Option<capture_types::RetentionPolicy>,
    i64,
    (bool, (bool, bool), Option<i64>),
    (usize, usize),
);

/// The last day model and the journal it was built from (kept alive, so the
/// pointer can't be reused by another load).
pub struct DayMemo {
    journal: Rc<JournalData>,
    key: DayKey,
    pub(crate) view: Rc<(day::DayModel, Vec<day::Card>)>,
}

/// `buildJournalDay` + the folded cards, rebuilt only when an input changes: a
/// new load, the day, the engine's watermark, recording, or the minute (retention
/// reads `now`). A hover or a playing receipt reuses the last build.
fn day_view(
    this: &mut Insights,
    j: &Rc<JournalData>,
    range: (i64, i64),
    now: i64,
    at_latest: bool,
) -> Rc<(day::DayModel, Vec<day::Card>)> {
    let ctx = this.status.ctx.as_ref();
    let retention = this.status.settings.as_ref().map(|s| s.retention_policy);
    let key: DayKey = (
        range,
        ctx.and_then(|c| c.covered_until_ms),
        ctx.is_some_and(|c| c.engine_available),
        ctx.and_then(|c| c.reason.clone()),
        retention,
        now.div_euclid(60_000),
        (at_latest, this.recording, this.recording_since),
        // In-place edits (tests) keep the pointer; their lengths still move.
        (j.activities.len(), j.frames.len()),
    );
    if let Some(m) = this
        .journal_day
        .as_ref()
        .filter(|m| Rc::ptr_eq(&m.journal, j) && m.key == key)
    {
        return m.view.clone();
    }
    let frames: Vec<i64> = j.frames.iter().map(|f| f.1).collect();
    let mut model = day::build(day::DayInput {
        activities: &j.activities,
        frames: &frames,
        audio: &j.audio,
        failed: &j.failed,
        covered_until: key.1,
        engine_available: key.2,
        engine_reason: key.3.clone(),
        day: range,
        retention: retention.as_ref(),
        now,
    });
    let cards = day::fold_cards(&model.slots);
    let (running, paused) = this.recording;
    // Recording today with nothing summarized yet: the first stretch shows
    // before the first frame lands (JR-04/05), from the session's start.
    if !model.pending.active && at_latest && running && !paused && model.slots.is_empty() {
        model.pending = day::Pending {
            active: true,
            since: this.recording_since.map(|s| s.max(range.0)),
            unavailable: (!key.2).then(|| key.3.clone().unwrap_or_default()),
        };
    }
    let view = Rc::new((model, cards));
    this.journal_day = Some(DayMemo {
        journal: j.clone(),
        key,
        view: view.clone(),
    });
    view
}

#[allow(clippy::too_many_arguments)]
fn lede_col(
    this: &Insights,
    why: Option<&str>,
    model: &day::DayModel,
    shown_n: usize,
    tracked: i64,
    at_latest: bool,
    day_label: &str,
    p: &Palette,
    cx: &mut Context<Insights>,
) -> gpui_kit::Div {
    let off = clock::now(cx).offset();
    let n = model.slots.len();
    let r = this.read_of(Of::Journal);
    let dg = r.digest.clone();
    let reason = r.reason;
    let engine = engine_on(&this.status);
    let tracked_s = humanize_ms(tracked);
    if let Some(why) = why {
        return lede(day_label, p)
            .child(display(format!("{tracked_s} tracked."), Some("Nothing written up.".into()), p))
            .child(prose(format!("{why} Everything captured is on the Timeline."), p));
    }
    let no_derive = !this.status.ctx.as_ref().is_some_and(|c| c.engine_available);
    let digest_off = no_derive
        || matches!(
            reason,
            Some(DigestAbsentReason::DerivationOff | DigestAbsentReason::EngineNotReady)
        );
    let kicker = if digest_off && dg.is_none() {
        day_label.to_string()
    } else {
        format!(
            "{} · {day_label}",
            if at_latest { "Digest so far" } else { "Digest" }
        )
    };
    if n >= 2 && engine && dg.is_none() && (r.loading || r.regenerating) {
        return lede(&kicker, p)
            .child(super::parts::writing_skel(p))
            .child(div().mt(space::S5).child(super::parts::busy_line("Writing the digest…", p)));
    }
    if n < 2 || reason == Some(DigestAbsentReason::TooFew) {
        let (h, b) = match n {
            0 => ("Your day is being written.".to_string(), "Capture is landing. The digest appears once two activities are written up."),
            1 => (format!("Too little captured on {day_label} to write a digest."), "A digest needs at least two activities; this day has one. It's below, as captured."),
            _ => (format!("Too little captured on {day_label} to write a digest."), "A digest needs at least two activities it can use. They're below, as captured."),
        };
        return lede(&kicker, p).child(display(h, None, p)).child(prose(b, p));
    }
    let me = cx.entity().downgrade();
    let regen = move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
        let _ = me.update(cx, |i, cx| i.reread(Of::Journal, cx));
    };
    if let Some(dg) = dg {
        let h = dg.headline.clone().unwrap_or_default();
        let mut it = h.split(" — ");
        let head = it
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{shown_n} activities, {tracked_s} tracked."));
        let rest: Vec<&str> = it.collect();
        let turn = (!rest.is_empty()).then(|| format!("—\u{a0}{}", rest.join(" — ")));
        let covers = this
            .status
            .ctx
            .as_ref()
            .and_then(|c| c.covered_until_ms)
            .filter(|_| at_latest)
            .map(|c| format!(" · covers through {}", stats::clock(c, off)))
            .unwrap_or_default();
        let reread = Button::new(
            "insights-journal-reread",
            if r.regenerating { "Re-reading…" } else { "Re-read" },
        )
        .ghost()
        .sm()
        .disabled(r.regenerating)
        .on_click(regen);
        let reread = if r.regenerating {
            reread.spinner(true)
        } else {
            reread.icon(IconName::RotateCcw.path())
        };
        return lede(&kicker, p)
            .child(display(head, turn, p))
            .child(prose(dg.narrative, p))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .mt(space::S5)
                    .child(mono_label(
                        format!("written {}{covers}", stats::clock(dg.generated_at_ms, off)),
                        p,
                    ))
                    .when(engine, |d| d.child(reread)),
            );
    }
    let what = if this.status.ctx.as_ref().and_then(|c| c.reason.as_deref())
        == Some("user_context_disabled")
        || reason == Some(DigestAbsentReason::DerivationOff)
    {
        "continuous derivation"
    } else {
        "AI"
    };
    let body = if digest_off {
        format!("No digest while {what} is off — the cards below were written before.")
    } else if r.error.is_some() {
        "Couldn't write the digest this time. The cards below are unaffected.".into()
    } else {
        "No digest for this day yet.".into()
    };
    lede(&kicker, p)
        .child(display(
            format!("{shown_n} activities, {tracked_s} tracked."),
            None,
            p,
        ))
        .child(prose(body, p))
        .when(r.error.is_some() && engine, |l| {
            l.child(
                div().mt(space::S5).child(
                    Button::new("insights-journal-digest-retry", "Try again")
                        .sm()
                        .icon(IconName::RotateCcw.path())
                        .on_click(regen),
                ),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn lede_stats(
    j: &JournalData,
    capture_run: bool,
    model: &day::DayModel,
    day_acts: &[capture_types::Activity],
    shown: &[capture_types::Activity],
    shown_n: usize,
    tracked: i64,
    at_latest: bool,
    now: i64,
    range: (i64, i64),
    p: &Palette,
    off: time::UtcOffset,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if model.audio_only {
        out.push(stat_text(
            "Tracked · app time",
            "—",
            Some("app time · screen off".into()),
            p,
        ));
    } else if j.usage.is_some() {
        let d = if at_latest {
            format!("through {}", stats::clock(now, off))
        } else {
            "app active time".into()
        };
        out.push(stat(
            "Tracked · app time",
            stat_value(&humanize_ms(tracked), p),
            Some(d),
            false,
            p,
        ));
    }
    let n = model.slots.len();
    let gaps = model.gaps.len();
    if capture_run {
        out.push(stat_text("Activities", "—", Some("none · needs AI".into()), p));
        return out.into_iter().map(|e| e.into_any_element()).collect();
    }
    out.push(stat_text(
        "Activities",
        &shown_n.to_string(),
        Some(if n > 0 {
            format!("{gaps} away-gap{}", if gaps == 1 { "" } else { "s" })
        } else {
            "none written up yet".into()
        }),
        p,
    ));
    let focused: Vec<_> = day_acts.iter().filter(|a| a.focus.is_some()).collect();
    if n > 0 {
        if let Some(pct) = stats::deep_pct(day_acts) {
            let deep = focused
                .iter()
                .filter(|a| a.focus == Some(capture_types::FocusLevel::Deep))
                .count();
            out.push(stat_text(
                "Deep focus · by activity",
                &format!("{pct}%"),
                Some(format!("{deep} of {} activities", focused.len())),
                p,
            ));
        }
        if let Some(t) = stats::top_category(day_acts, range.0, range.1) {
            let mix = day::category_mix(shown);
            let total: i64 = mix.iter().map(|m| m.1).sum();
            let pct = mix
                .iter()
                .find(|m| m.0 == t.category)
                .map(|m| stats::fmt_pct(m.1, total));
            out.push(stat(
                "Top category",
                top_value(category_label(t.category), category_color(t.category, p), p),
                pct.map(|x| format!("{x}% of activity time")),
                false,
                p,
            ));
        }
    }
    out.into_iter().map(|e| e.into_any_element()).collect()
}
