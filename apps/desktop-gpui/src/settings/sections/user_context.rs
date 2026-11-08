//! Intelligence › User Context (`UserContext.svelte`): derivation on/off, its
//! status + Derive now, the budget tier, history reach, and Wipe (asks first).

use capture_types::{DerivationBudgetTier, RetentionPolicy};
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context};

use super::ix;
use crate::chat::format::provider_is_local;
use crate::settings::model::{short_datetime, Json};
use crate::settings::row::{line, row, section};
use crate::settings::{click, confirm, icons, on, reload, Settings};
use crate::theme::{space, Palette};
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};

const TIERS: [DerivationBudgetTier; 3] = [
    DerivationBudgetTier::Light,
    DerivationBudgetTier::Balanced,
    DerivationBudgetTier::Thorough,
];

/// `Intl.NumberFormat(notation: compact, maximumFractionDigits: 1)`.
fn compact(n: f64) -> String {
    let (v, suffix) = if n >= 1e9 {
        (n / 1e9, "B")
    } else if n >= 1e6 {
        (n / 1e6, "M")
    } else if n >= 1e3 {
        (n / 1e3, "K")
    } else {
        (n, "")
    };
    let s = format!("{v:.1}");
    format!("{}{suffix}", s.trim_end_matches(".0"))
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let uc = &d.user_context;
    let st = s.data.user_context.as_ref();
    let plural = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let now = crate::ui::clock::now(cx);
    let counts = match st {
        Some(v) => {
            let last = v
                .num_at(&["lastDerivedAtMs"])
                .map_or("never".to_string(), |ms| short_datetime(ms as i64, now));
            format!(
                "{} \u{b7} {} \u{b7} last run {last} \u{b7} \u{2248} {} tokens so far (estimated)",
                plural(
                    v.num_at(&["activityCount"]).unwrap_or(0.) as i64,
                    "activity",
                    "activities"
                ),
                plural(
                    v.num_at(&["conclusionCount"]).unwrap_or(0.) as i64,
                    "conclusion",
                    "conclusions"
                ),
                compact(v.num_at(&["tokenUsage", "totalTokens"]).unwrap_or(0.)),
            )
        }
        None => "Loading\u{2026}".into(),
    };
    let engine = st.is_some_and(|v| v.bool_at(&["engineAvailable"]));
    let backfilling = st.is_some_and(|v| v.bool_at(&["backfilling"]));
    let paused = st.is_some() && !engine && !backfilling;
    let up_to_date =
        engine && !backfilling && st.and_then(|v| v.num_at(&["activityCount"])).unwrap_or(0.) > 0.;
    let actions = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(reload::button(
            s,
            "reload-uc",
            "Refresh derivation status",
            false,
            |s, cx| s.reload_all(cx),
            cx,
        ))
        // shortcut: user_context_run_derivation_now is in app-core; its run message line
        // lands with the user_context_changed event wiring (S3).
        .child(Button::new("uc-derive", "Derive latest now").disabled(!engine));
    let status = row(if paused {
        "Derivation paused"
    } else {
        "Deriving activities"
    })
    .desc(counts)
    .control(actions)
    .when(paused, |r| {
        let reason = st
            .and_then(|v| v.str_at(&["reason"]))
            .unwrap_or_default()
            .to_string();
        r.below(line(Tone::Warn, icons::INFO, reason, p))
    })
    .when(up_to_date, |r| {
        r.below(line(
            Tone::Ok,
            icons::CHECK,
            "Up to date for the covered window.",
            p,
        ))
    });

    let ai = &d.ai_runtime;
    let default_local = ai
        .default_model
        .as_ref()
        .map(|m| provider_is_local(ai.providers.iter().find(|pv| pv.id == m.provider)));
    let cloud_default = default_local == Some(false);
    let gate = match default_local {
        Some(false) => None,
        Some(true) => Some("Local models use fixed pacing."),
        None => Some("Choose a cloud default model first."),
    };
    let keep_days = match d.retention_policy {
        RetentionPolicy::Days7 => Some(7),
        RetentionPolicy::Days14 => Some(14),
        RetentionPolicy::Days30 => Some(30),
        RetentionPolicy::Never => None,
    };
    let reach = if uc.backfill_go_deeper {
        match keep_days {
            None => "Reaches back to your first capture.".to_string(),
            Some(k) => format!("Reaches back about {k} days \u{2014} everything you keep."),
        }
    } else {
        let r = keep_days.map_or(uc.backfill_window_days, |k| uc.backfill_window_days.min(k));
        format!(
            "Reaches back about {r} {}. Turn on to include everything you keep.",
            if r == 1 { "day" } else { "days" }
        )
    };
    let rows = vec![
        row("Derive context continuously")
            .desc("Builds a quiet picture of your projects and people from captured activity.")
            .control(Switch::new("uc-enabled", "Derive context continuously", uc.enabled).on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.user_context.enabled = v, cx))))
            .build(p)
            .into_any_element(),
        status.build(p).into_any_element(),
        row("Derivation Budget")
            .desc("Paces background work so tokens are spent as a trickle, never a one-time bill. Applies to a cloud default model.")
            .control(
                Segmented::new("uc-budget", "Derivation budget intensity", ["Light", "Balanced", "Thorough"], ix(&TIERS, &uc.derivation_budget_tier))
                    .md()
                    .disabled(!cloud_default)
                    .on_change(on(cx, |s, i: usize, cx| s.edit(|d| d.user_context.derivation_budget_tier = TIERS[i].clone(), cx))),
            )
            .when(gate.is_some(), |r| r.below(line(Tone::Neutral, icons::INFO, gate.unwrap_or_default(), p)))
            .build(p)
            .into_any_element(),
        row("Derive all of history")
            .desc(reach)
            .control(
                Switch::new("uc-deeper", "Go deeper \u{2014} derive all of history", uc.backfill_go_deeper)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.user_context.backfill_go_deeper = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Wipe User Context")
            .desc("Permanently clears everything derived about you \u{2014} it outlives capture retention. Captures are untouched. Asks first.")
            .control(Button::new("uc-wipe", "Wipe\u{2026}").danger().on_click(click(cx, |_, cx| {
                confirm(
                    cx,
                    "Wipe User Context?",
                    "This permanently clears everything Mnema has derived about you \u{2014} all Activities, Conclusions, and your Dismissal corrections. Your raw recordings and settings are kept. AI features will be turned off; turning them back on starts learning from scratch.",
                    "Wipe",
                    // shortcut: wipe_user_context is src-tauri until S3 (it stops the worker first).
                    |_, _| log::info!("settings: wipe user context requested (lands with S3)"),
                );
            })))
            .build(p)
            .into_any_element(),
    ];
    section("User Context", None, Vec::new(), false, rows, p).into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_numbers() {
        assert_eq!(super::compact(1_242_000.), "1.2M");
        assert_eq!(super::compact(900.), "900");
        assert_eq!(super::compact(2_000.), "2K");
    }
}
