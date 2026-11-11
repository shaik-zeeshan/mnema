//! Intelligence › User Context (`UserContext.svelte`): derivation on/off, its
//! status + Derive now, the budget tier, history reach, and Wipe (asks first).

use capture_types::{DerivationBudgetTier, RetentionPolicy};
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context};

use super::ix;
use crate::settings::model::{short_datetime, Json};
use crate::settings::row::{line, row, section};
use crate::settings::{click, confirm, icons, on, reload, Settings};
use crate::theme::{space, Palette};
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;
use crate::ui::{notice, progress, Button, Tone};

/// `distillationWithheldLine`: what the last distillation pass held back.
fn withheld_line(summary: &serde_json::Value) -> Option<String> {
    let n = |k: &str| summary.num_at(&[k]).unwrap_or(0.) as i64;
    let parts = [
        (n("guardrailSuppressed"), "by the privacy guardrail"),
        (n("belowFormationBar"), "needing more evidence"),
        (n("resurfaceBlocked"), "honoring a dismissal"),
        (n("ungrounded"), "without grounding"),
    ];
    let reasons: Vec<String> = parts
        .iter()
        .filter(|(c, _)| *c > 0)
        .map(|(c, why)| format!("{c} {why}"))
        .collect();
    if reasons.is_empty() {
        return None;
    }
    let total: i64 = parts.iter().map(|(c, _)| c).sum();
    Some(format!(
        "Last distillation held back {total} draft {}: {}.",
        if total == 1 {
            "conclusion"
        } else {
            "conclusions"
        },
        reasons.join(", ")
    ))
}

/// The one-shot commands this section runs (`run_command` names).
pub(crate) const DERIVE: &str = "Derive latest now";
pub(crate) const WIPE: &str = "Wipe User Context";

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

/// `.mx-inline` with no icon or tone.
fn plain(t: String, p: &Palette) -> gpui_kit::Div {
    notice::inline(Tone::Neutral, None, vec![t.into_any_element()], p)
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
        None if s.data.complete => "Couldn\u{2019}t read the status.".into(),
        None => "Loading\u{2026}".into(),
    };
    let engine = st.is_some_and(|v| v.bool_at(&["engineAvailable"]));
    let backfilling = st.is_some_and(|v| v.bool_at(&["backfilling"]));
    let paused = st.is_some() && !engine && !backfilling;
    let up_to_date =
        engine && !backfilling && st.and_then(|v| v.num_at(&["activityCount"])).unwrap_or(0.) > 0.;
    let deriving = s.ui.busy.contains(DERIVE);
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
        .child(
            Button::new(
                "uc-derive",
                if deriving {
                    "Deriving\u{2026}"
                } else {
                    "Derive latest now"
                },
            )
            .disabled(!engine || deriving)
            .busy(deriving)
            .spinner(deriving)
            .on_click(click(cx, |s, cx| {
                s.run_command(DERIVE, cx, crate::settings::live::derive_now)
            })),
        );
    let withheld = st
        .and_then(|v| v.get("lastDistillation"))
        .and_then(withheld_line);
    // Derive now's answer and a failed wipe speak here, not in the page notice.
    let run_note =
        s.ui.note
            .as_ref()
            .filter(|n| n.1 == DERIVE)
            .map(|n| n.2.clone());
    let wipe_error =
        s.ui.note
            .as_ref()
            .filter(|n| n.1 == WIPE && !n.0)
            .map(|n| n.2.clone());
    let status = if backfilling {
        let so_far = st.and_then(|v| v.num_at(&["activityCount"])).unwrap_or(0.) as i64;
        row("Deriving activities")
            .desc("Building your understanding \u{2014} deriving from your history in the background, newest first.")
            .full()
            .aside(actions)
            .control(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(progress::progress_meta("Backfilling", format!("{so_far} activities so far"), p))
                    .child(progress::progress(None, p)),
            )
    } else {
        row(if paused {
            "Derivation paused"
        } else {
            "Deriving activities"
        })
        .desc(counts)
        .control(actions)
    };
    let status = status
        .when(paused, |r| {
            // `aiRuntimeReasonLabel`: "Unavailable" with no reason, never a raw code.
            let reason = st.and_then(|v| v.str_at(&["reason"])).map_or_else(
                || "Unavailable".to_string(),
                |r| crate::insights::model::reason_label(r, Some(&d)),
            );
            r.below(line(Tone::Warn, icons::INFO, reason, p))
        })
        .when(up_to_date, |r| {
            r.below(line(
                Tone::Ok,
                icons::CHECK,
                "Up to date for the covered window.",
                p,
            ))
        })
        .when(withheld.is_some(), |r| {
            r.below(plain(withheld.clone().unwrap_or_default(), p))
        })
        .when(run_note.is_some(), |r| {
            r.below(plain(run_note.clone().unwrap_or_default(), p))
        })
        .when(wipe_error.is_some(), |r| {
            r.below(line(
                Tone::Danger,
                icons::ALERT,
                wipe_error.clone().unwrap_or_default(),
                p,
            ))
        });

    let ai = &d.ai_runtime;
    // `isCloudAiProviderInstance`: by kind; a default whose provider is gone is not cloud.
    let default_local = ai.default_model.as_ref().map(|m| {
        !ai.providers
            .iter()
            .any(|pv| pv.id == m.provider && !pv.kind.is_local())
    });
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
    let wiping = s.ui.busy.contains(WIPE);
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
            .control(Button::new("uc-wipe", if wiping { "Wiping\u{2026}" } else { "Wipe\u{2026}" }).danger().disabled(wiping).busy(wiping).spinner(wiping).on_click(click(cx, |_, cx| {
                confirm(
                    cx,
                    "Wipe User Context?",
                    "This permanently clears everything Mnema has derived about you \u{2014} all Activities, Conclusions, and your Dismissal corrections. Your raw recordings and settings are kept. AI features will be turned off; turning them back on starts learning from scratch.",
                    "Wipe",
                    |s, cx| s.run_command(WIPE, cx, crate::settings::live::wipe_user_context),
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
