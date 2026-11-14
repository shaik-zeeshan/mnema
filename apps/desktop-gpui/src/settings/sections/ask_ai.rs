//! Intelligence › Ask AI (`AskAi.svelte`): the switch, auto-answer, the model
//! override (the shared picker), the tool-call cap and page fetches.

use capture_types::QuickRecallAutoAnswer;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::ix;
use crate::chat::format::provider_label_by_id;
use crate::settings::model::Json;
use crate::settings::row::{line, row, section};
use crate::settings::{click, icons, on, Settings};
use crate::theme::{space, Palette};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::model_picker::{ModelPicker, Spec, Trigger};
use crate::ui::notice;
use crate::ui::stepper::Stepper;
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};

const AUTO: [QuickRecallAutoAnswer; 3] = [
    QuickRecallAutoAnswer::Always,
    QuickRecallAutoAnswer::LocalOnly,
    QuickRecallAutoAnswer::Never,
];
/// `ASK_AI_MAX_TOOL_CALL_LIMIT`.
const MAX_TOOL_CALLS: i64 = 64;

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let a = &d.access;
    let enabled = a.ask_ai_enabled;
    let avail = s.data.ask_ai.as_ref();
    let available = avail.is_some_and(|v| v.bool_at(&["available"]));
    let detail = match avail {
        None if s.data.complete => "Couldn\u{2019}t check Ask AI availability.".to_string(),
        None => "Checking Ask AI availability\u{2026}".to_string(),
        Some(_) if available => "Ask AI can answer over your redacted capture history.".into(),
        Some(v) => crate::insights::model::ask_ai_reason_label(v.str_at(&["reason"]), Some(&d)),
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    // Readiness is only worth saying while Ask AI is on and a check landed.
    if enabled && avail.is_some() && !available {
        rows.push(
            div()
                .py(space::S3)
                .border_b_1()
                .border_color(p.hairline)
                .child(notice::notice(
                    Tone::Warn,
                    icons::INFO,
                    "Ask AI isn\u{2019}t ready yet",
                    Some(detail.clone().into()),
                    vec![Button::new("askai-go-providers", "Go to Providers")
                        .ghost()
                        .sm()
                        .on_click(click(cx, |s, cx| {
                            s.navigate(crate::settings::groups::SectionId::Intelligence, cx)
                        }))
                        .into_any_element()],
                    p,
                ))
                .into_any_element(),
        );
    }
    rows.push(
        row("Enable Ask AI")
            .desc("Allow Quick Recall and Chat to answer questions over your redacted capture history.")
            .control(Switch::new("askai-enabled", "Enable Ask AI", enabled).on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.access.ask_ai_enabled = v, cx))))
            .when(enabled && available, |r| r.below(line(Tone::Ok, icons::CHECK, detail.clone(), p)))
            .when(avail.is_none() && s.data.complete, |r| r.below(line(Tone::Danger, icons::ALERT, detail.clone(), p)))
            .build(p)
            .into_any_element(),
    );
    let opts = vec![
        ChoiceOption::new("Always")
            .icon(icons::CLOUD)
            .desc("Any model, cloud included. Every pause on a question spends a call and sends results to the provider.")
            .disabled(!enabled),
        ChoiceOption::new("Only with local models")
            .tag("default")
            .icon(icons::CHIP)
            .desc("Answers on its own when the model runs on this Mac. With a cloud model, press Tab to ask.")
            .disabled(!enabled),
        ChoiceOption::new("Never").icon(icons::OFF).desc("Quick Recall only searches until you press Tab to ask.").disabled(!enabled),
    ];
    rows.push(
        row("Answer questions automatically")
            .desc("When you pause on a question in Quick Recall, start answering without a keypress. Each answer is one model call.")
            .full()
            .control(div().w_full().child(
                Choice::new("auto-answer", "Answer questions automatically", opts, ix(&AUTO, &a.quick_recall_auto_answer))
                    .on_change(on(cx, |s, i: usize, cx| s.edit(|d| d.access.quick_recall_auto_answer = AUTO[i].clone(), cx))),
            ))
            .build(p)
            .into_any_element(),
    );
    let ai = &d.ai_runtime;
    let model = a.ask_ai_model.clone().unwrap_or_default();
    let provider = a.ask_ai_model_provider.clone().unwrap_or_default();
    let label = if model.is_empty() {
        "Global default model".to_string()
    } else if provider.is_empty() {
        model.clone()
    } else {
        format!(
            "{} \u{b7} {model}",
            provider_label_by_id(&ai.providers, &provider)
        )
    };
    let spec = Spec {
        providers: ai.providers.clone(),
        pool: s.pool.clone(),
        loading: s.pool_loading,
        failures: s.ui.pv.failures.clone(),
        favorites: ai
            .favorite_models
            .iter()
            .map(|e| (e.provider.clone(), e.model.clone()))
            .collect(),
        recents: ai
            .recent_models
            .iter()
            .map(|e| (e.provider.clone(), e.model.clone()))
            .collect(),
        sentinel: Some((
            "Global default model".into(),
            ai.default_model
                .as_ref()
                .map(|m| (m.provider.clone(), m.model.clone())),
        )),
        sentinel_selected: model.is_empty(),
        selected: (!model.is_empty()).then(|| {
            (
                (!provider.is_empty()).then(|| provider.clone()),
                model.clone(),
            )
        }),
        first_provider: ai.default_model.as_ref().map(|m| m.provider.clone()),
        can_star: true,
        ..Default::default()
    };
    let picker = ModelPicker::new("askai-model", spec)
        .trigger(Trigger::Block {
            label,
            placeholder: false,
        })
        .disabled(!enabled)
        .on_favorite(on(cx, |s, engine: (String, String), cx| {
            s.toggle_favorite(engine, cx)
        }))
        .on_open({
            let this = cx.entity().downgrade();
            move |_, cx| {
                let _ = this.update(cx, |s, cx| s.load_pool(cx));
            }
        })
        .on_select(on(cx, |s, pick: Option<(String, String)>, cx| {
            s.edit(
                |d| {
                    // Saved with its provider so it runs there.
                    d.access.ask_ai_model = pick.as_ref().map(|(_, m)| m.clone());
                    d.access.ask_ai_model_provider = pick.map(|(pv, _)| pv);
                },
                cx,
            )
        }));
    rows.push(
        row("Model override")
            .desc("Optional override for Quick Recall and Chat. A pinned chat thread still wins.")
            .control(div().w(px(280.)).child(picker))
            .when(!s.ui.pv.failures.is_empty(), |r| {
                r.below(line(
                    Tone::Warn,
                    icons::INFO,
                    "Couldn\u{2019}t list models \u{2014} you can still type any model id.",
                    p,
                ))
            })
            .build(p)
            .into_any_element(),
    );
    let limited = a.ask_ai_max_tool_calls > 0;
    rows.push(
        row("Limit tool calls per question")
            .desc("Cap how many follow-up searches Ask AI can run for one question.")
            .when(limited, |r| {
                r.control(
                    Stepper::new("askai-tool-calls", "Max tool calls", &s.inputs.tool_calls)
                        .range(Some(1), Some(MAX_TOOL_CALLS))
                        .disabled(!enabled),
                )
            })
            .control(
                Switch::new("askai-limit", "Limit tool calls per question", limited)
                    .disabled(!enabled)
                    .on_toggle(on(cx, |s, v: bool, cx| {
                        // Back on, the cap is the one the stepper last held.
                        let last = s.inputs.tool_calls.read(cx).value().trim().parse::<u32>();
                        let cap = last.ok().filter(|n| *n > 0).unwrap_or(12);
                        s.edit(
                            |d| d.access.ask_ai_max_tool_calls = if v { cap } else { 0 },
                            cx,
                        )
                    })),
            )
            .when(!limited, |r| {
                r.below(line(
                    Tone::Warn,
                    icons::INFO,
                    "No cap: one question can run unlimited searches over your retained history.",
                    p,
                ))
            })
            .build(p)
            .into_any_element(),
    );
    rows.push(
        row("Fetch pages you visited")
            .desc("Lets Ask AI re-request a page you visited (its address minus any secrets) to check its current state.")
            .control(
                Switch::new("askai-fetch", "Fetch pages you visited", a.ask_ai_web_fetch_enabled)
                    .disabled(!enabled)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.access.ask_ai_web_fetch_enabled = v, cx))),
            )
            .build(p)
            .into_any_element(),
    );
    section("Ask AI", None, Vec::new(), false, rows, p).into_any_element()
}
