//! Journal (`DayTimeline.svelte`): one local day. The day capsule + Today on
//! the bar, `[` / `]` step days; the digest lede beside four stats; the day
//! spine; the river of cards (newest first, each expanding into its receipt)
//! beside the week strip, the day mix and apps.

pub mod day;
pub mod receipt;
mod river;
mod spine;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

use super::model::{JournalData, Tab};
use super::overview::engine_on;
use super::parts::{
    display, lede, mono_label, prose, read, stat, stat_text, stat_value, top_value,
};
use super::stats::{self, category_color, category_label, humanize_ms};
use super::Insights;
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
    div()
        .pt(space::S7)
        .flex()
        .justify_center()
        .child(empty(
            EmptyLayout::Center,
            Tone::Neutral,
            if title == PITCH {
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
    window: &mut Window,
    cx: &mut Context<Insights>,
) -> AnyElement {
    let Some(j) = this.journal.clone().filter(|_| this.status.loaded) else {
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
        return center(
            format!("Couldn't load {day_label}"),
            "Reading this day's activities failed. Nothing is lost — the footage is still on the Timeline.",
            vec![retry.into_any_element()],
            p,
        );
    }
    let ctx = this.status.ctx.as_ref();
    let pitch = this.status.ai.as_ref().is_some_and(|a| !a.has_providers);
    let frames: Vec<i64> = j.frames.iter().map(|f| f.1).collect();
    let model = day::build(day::DayInput {
        activities: &j.activities,
        frames: &frames,
        audio: &j.audio,
        failed: &j.failed,
        covered_until: ctx.and_then(|c| c.covered_until_ms),
        engine_available: ctx.is_some_and(|c| c.engine_available),
        engine_reason: ctx.and_then(|c| c.reason.clone()),
        day: range,
    });
    let cards = day::fold_cards(&model.slots);
    if pitch && cards.is_empty() {
        return center(
            PITCH.into(),
            "Once a provider is connected, Mnema writes up what you do as activities every few minutes — with a digest of the day on top. Capture is already on the Timeline.",
            vec![
                Button::new("insights-journal-setup", "Set up AI").sm().into_any_element(),
                Button::new("insights-journal-timeline", "Open Timeline")
                    .ghost()
                    .sm()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(super::InsightsEvent::OpenTimeline)))
                    .into_any_element(),
            ],
            p,
        );
    }
    if cards.is_empty() && !model.pending.active {
        let (title, body) = if !model.has_capture {
            (format!("Nothing captured on {day_label}"), "There's no capture on this day, so there's no journal to show. Days with any recording at all show whatever was captured.")
        } else if ctx.is_some_and(|c| c.backfilling) {
            ("Not summarized yet".into(), "Mnema is catching up on older history, newest first — past days fill in as it goes. The footage is already on the Timeline.")
        } else {
            (format!("No cards for {day_label}"), "Nothing in this day's capture was summarized into a card. The footage is on the Timeline.")
        };
        let acts = (!at_latest)
            .then(|| {
                let today = clock::now(cx).date();
                Button::new("insights-go-today", "Go to today")
                    .sm()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_day(today, cx)))
                    .into_any_element()
            })
            .into_iter()
            .collect();
        return center(title, body, acts, p);
    }
    let shown = day::shown_activities(&cards);
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
                &j,
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
            &model,
            &cards,
            at_latest.then_some(now),
            p,
            off,
        ))
        .child(river::render(
            this, &j, &model, &cards, &shown, range.0, p, window, cx,
        ))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn lede_col(
    this: &Insights,
    j: &JournalData,
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
    let digest = j.digest.as_ref().and_then(|r| r.as_ref().ok());
    let dg = digest.and_then(|r| r.digest.clone());
    let reason = digest.and_then(|r| r.reason);
    let engine = engine_on(&this.status);
    let no_derive = !this.status.ctx.as_ref().is_some_and(|c| c.engine_available);
    let digest_off = no_derive
        || matches!(
            reason,
            Some(
                capture_types::DigestAbsentReason::DerivationOff
                    | capture_types::DigestAbsentReason::EngineNotReady
            )
        );
    let kicker = if digest_off && dg.is_none() {
        day_label.to_string()
    } else {
        format!(
            "{} · {day_label}",
            if at_latest { "Digest so far" } else { "Digest" }
        )
    };
    let tracked_s = humanize_ms(tracked);
    if n < 2 || reason == Some(capture_types::DigestAbsentReason::TooFew) {
        let (h, b) = match n {
            0 => ("Your day is being written.".to_string(), "Capture is landing. The digest appears once two activities are written up."),
            1 => (format!("Too little captured on {day_label} to write a digest."), "A digest needs at least two activities; this day has one. It's below, as captured."),
            _ => (format!("Too little captured on {day_label} to write a digest."), "A digest needs at least two activities it can use. They're below, as captured."),
        };
        return lede(&kicker, p)
            .child(display(h, None, p))
            .child(prose(b, p));
    }
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
        return lede(&kicker, p)
            .child(display(head, turn, p))
            .child(prose(dg.narrative, p))
            .child(
                gpui_kit::div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .mt(space::S5)
                    .child(mono_label(
                        format!("written {}{covers}", stats::clock(dg.generated_at_ms, off)),
                        p,
                    ))
                    .when(engine, |d| {
                        d.child(
                            Button::new("insights-journal-reread", "Re-read")
                                .ghost()
                                .sm()
                                .icon(IconName::RotateCcw.path()),
                        )
                    }),
            );
    }
    let what = if this.status.ctx.as_ref().and_then(|c| c.reason.as_deref())
        == Some("user_context_disabled")
    {
        "continuous derivation"
    } else {
        "AI"
    };
    let body = if digest_off {
        format!("No digest while {what} is off — the cards below were written before.")
    } else if j.digest.as_ref().is_some_and(|r| r.is_err()) {
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
}

#[allow(clippy::too_many_arguments)]
fn lede_stats(
    j: &JournalData,
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
