//! Intelligence › Providers (`Providers.svelte`, `ProviderRow.svelte`): the AI
//! master switch, one row per provider instance, Add provider, the global
//! default model (the picker Chat shares) and Test connection.

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, SharedString, Window};

use crate::chat::format::{kind_label, provider_label_by_id};
use crate::settings::model::Json;
use crate::settings::row::{line, row, section};
use crate::settings::{click, icons, on, Settings};
use crate::theme::{radius, space, text, Palette, FONT_MONO};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::menu::menu_item;
use crate::ui::model_picker::{EngineRef, ModelPicker, Spec, Trigger};
use crate::ui::popover::Anchor;
use crate::ui::switch::Switch;
use crate::ui::{float, notice, popover, with_tip, Button, Force, Tone};

const KINDS: [AiProviderKind; 6] = [
    AiProviderKind::Anthropic,
    AiProviderKind::Openai,
    AiProviderKind::Chatgpt,
    AiProviderKind::OpenaiCompatible,
    AiProviderKind::Ollama,
    AiProviderKind::Llamafile,
];

pub(crate) fn cloud(kind: AiProviderKind) -> bool {
    !kind.is_local()
}

pub(crate) fn local_default(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Ollama => "http://localhost:11434",
        AiProviderKind::Llamafile => "http://localhost:8080",
        _ => "",
    }
}

/// `aiProviderKindDescription`: the Add menu's tip per kind.
fn kind_description(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Anthropic => "Claude models \u{2014} your own API key",
        AiProviderKind::Openai => "GPT models \u{2014} your own API key",
        AiProviderKind::Chatgpt => {
            "Your ChatGPT Plus/Pro subscription \u{2014} sign in, no API key"
        }
        AiProviderKind::OpenaiCompatible => {
            "Fireworks, OpenRouter, Together \u{2014} custom base URL + key"
        }
        AiProviderKind::Ollama => "Local runtime, default endpoint http://localhost:11434",
        AiProviderKind::Llamafile => {
            "Local OpenAI-compatible server, default http://localhost:8080"
        }
    }
}

/// `allocateProviderId`: the first instance keeps `id == kind`, then `kind-2`, `kind-3`…
pub(crate) fn new_id(kind: AiProviderKind, providers: &[AiProviderConfig]) -> String {
    let base = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    if !providers.iter().any(|p| p.id == base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !providers.iter().any(|p| &p.id == id))
        .unwrap_or(base)
}

pub fn render(
    s: &Settings,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let d = s.d();
    let ai = &d.ai_runtime;
    let status = s.data.ai_status.clone();
    let reason = status
        .as_ref()
        .and_then(|v| v.str_at(&["reason"]))
        .unwrap_or_default()
        .to_string();
    let label_of = |r: &str| crate::insights::model::reason_label(r, Some(&d));
    let mut rows: Vec<AnyElement> = Vec::new();
    // "Nothing is ready yet" = setup is incomplete; a set-up provider that
    // fails speaks on its own row instead.
    let not_ready = ai.enabled
        && status.as_ref().is_some_and(|v| !v.bool_at(&["available"]))
        && [
            "no_providers",
            "no_default_model",
            "provider_not_connected",
            "no_provider_key",
        ]
        .iter()
        .any(|c| reason.to_lowercase().starts_with(c));
    if not_ready {
        rows.push(
            div()
                .py(space::S3)
                .border_b_1()
                .border_color(p.hairline)
                .child(notice::notice(
                    Tone::Warn,
                    icons::INFO,
                    "AI features are on, but nothing is ready yet",
                    Some(
                        format!(
                            "{} Connect a provider and choose a default model below.",
                            label_of(&reason)
                        )
                        .into(),
                    ),
                    Vec::new(),
                    p,
                ))
                .into_any_element(),
        );
    }
    rows.push(
        row("Enable AI features")
            .desc("The master switch for everything AI in Mnema. While off, nothing is sent to any AI model.")
            .control(Switch::new("ai-enabled", "Enable AI features", ai.enabled).on_toggle(on(cx, |s, v: bool, cx| {
                // A "Connection succeeded" line never sits beside a switched-off master.
                if !v {
                    s.reset_test();
                }
                s.edit(|d| d.ai_runtime.enabled = v, cx)
            })))
            .build(p)
            .into_any_element(),
    );
    // One anchor: only one of the two triggers (empty state / row) renders.
    let at = Anchor::new("pv-add-at", window, cx);
    let add_menu = |s: &Settings, small: bool, cx: &mut Context<Settings>| {
        let open = s.ui.add_provider_open;
        let trigger = Button::new(
            if small { "pv-add-empty" } else { "pv-add" },
            "Add provider\u{2026}",
        )
        .icon(icons::PLUS)
        .when(small, |b| b.sm())
        .disabled(s.ui.pv.removing)
        .on_click(click(cx, |s, cx| {
            s.ui.add_provider_open = !s.ui.add_provider_open;
            cx.notify();
        }));
        let items: Vec<AnyElement> = KINDS
            .iter()
            .map(|k| {
                let k = *k;
                with_tip(
                    div()
                        .id(SharedString::from(format!("pv-kind-tip-{}", kind_label(k))))
                        .child(menu_item(
                            SharedString::from(format!("pv-kind-{}", kind_label(k))),
                            kind_label(k),
                            Some(if cloud(k) { "cloud" } else { "local" }.into()),
                            Force::None,
                            click(cx, move |s, cx| s.add_provider(k, cx)),
                            p,
                        )),
                    kind_description(k),
                )
                .into_any_element()
            })
            .collect();
        let out = at.clone();
        let close = cx.listener(move |s, e: &gpui_kit::MouseDownEvent, _, cx| {
            // The trigger toggles it; Svelte's window click skips the anchor too.
            if out.outside(e, cx) {
                s.ui.add_provider_open = false;
                cx.notify();
            }
        });
        at.track(div()).relative().child(trigger).when(open, |d| {
            d.child(float(
                popover("pv-add-menu", "Add provider", p)
                    .top(px(36.))
                    .right_0()
                    .min_w(px(260.))
                    .p(px(4.))
                    .children(items)
                    .on_mouse_down_out(close)
                    .test_support(),
            ))
        })
    };
    if ai.providers.is_empty() {
        rows.push(
            div()
                .py(space::S3)
                .border_b_1()
                .border_color(p.hairline)
                .child(empty(
                    EmptyLayout::Compact,
                    Tone::Neutral,
                    icons::SPARKLES,
                    "No providers connected yet",
                    "Connect one, then choose a default model.",
                    vec![add_menu(s, true, cx).into_any_element()],
                    p,
                ))
                .into_any_element(),
        );
    } else {
        for pv in &ai.providers {
            rows.push(super::provider_row::render(s, pv, &reason, p, cx));
        }
        if let Some(e) = &s.ui.pv.removal_error {
            rows.push(
                div()
                    .py(space::S3)
                    .border_b_1()
                    .border_color(p.hairline)
                    .child(line(Tone::Danger, icons::ALERT, e.clone(), p))
                    .into_any_element(),
            );
        }
        let kinds = KINDS
            .iter()
            .map(|k| kind_label(*k))
            .collect::<Vec<_>>()
            .join(", ")
            + ".";
        rows.push(
            row("Add a provider")
                .desc(kinds)
                .control(add_menu(s, false, cx))
                .build(p)
                .into_any_element(),
        );
    }
    let label = ai
        .default_model
        .as_ref()
        .filter(|m| !m.model.trim().is_empty())
        .map(|m| {
            format!(
                "{} \u{b7} {}",
                provider_label_by_id(&ai.providers, &m.provider),
                m.model
            )
        });
    let spec = Spec {
        providers: ai.providers.clone(),
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
        pool: s.pool.clone(),
        loading: s.pool_loading,
        failures: s.ui.pv.failures.clone(),
        selected: ai
            .default_model
            .as_ref()
            .map(|m| (Some(m.provider.clone()), m.model.clone())),
        first_provider: ai.default_model.as_ref().map(|m| m.provider.clone()),
        can_star: true,
        ..Default::default()
    };
    let picker = ModelPicker::new("default-model", spec)
        .disabled(ai.providers.is_empty())
        .on_open({
            let this = cx.entity().downgrade();
            move |_, cx| {
                let _ = this.update(cx, |s, cx| s.load_pool(cx));
            }
        })
        .on_favorite(on(cx, |s, engine: EngineRef, cx| {
            s.toggle_favorite(engine, cx)
        }))
        .trigger(Trigger::Block {
            label: label
                .clone()
                .unwrap_or_else(|| "Choose a default model".into()),
            placeholder: ai.default_model.is_none(),
        })
        .on_select(on(cx, |s, pick: Option<EngineRef>, cx| {
            if let Some((provider, model)) = pick {
                // The last test line named the previous default.
                s.reset_test();
                s.edit(
                    |d| d.ai_runtime.default_model = Some(AiEngineRef { provider, model }),
                    cx,
                )
            }
        }));
    rows.push(
        row("Global default model")
            .desc("Used by every AI feature unless it sets its own override.")
            .control(div().w(px(280.)).child(picker))
            .when(!s.ui.pv.failures.is_empty(), |r| {
                r.below(line(
                    Tone::Warn,
                    icons::INFO,
                    "Couldn\u{2019}t list every provider\u{2019}s models \u{2014} you can still type any model id.",
                    p,
                ))
            })
            .build(p)
            .into_any_element(),
    );
    rows.push(runtime_row(s, status.as_ref(), &d, p, cx));
    section(
        "Providers",
        Some("One global default model; every AI feature inherits it."),
        Vec::new(),
        false,
        rows,
        p,
    )
    .into_any_element()
}

/// `runtimeReasonTone`.
fn reason_tone(reason: &str) -> Tone {
    let code = reason.to_lowercase();
    let danger = [
        "needs_reconnect",
        "no_provider_key",
        "base_url_host_mismatch",
        "invalid_base_url",
        "vault_denied",
    ];
    if danger.iter().any(|c| code.starts_with(c)) {
        Tone::Danger
    } else {
        Tone::Warn
    }
}

/// `aiRuntimeReasonLabel` for a command rejection: a known code reads as its
/// label, anything else (an HTTP error's words) as itself.
fn rejection_label(e: &str, d: &capture_types::RecordingSettings) -> String {
    let l = crate::insights::model::reason_label(e, Some(d));
    if l == crate::insights::model::reason_label("\u{0}", None) {
        e.to_string()
    } else {
        l
    }
}

/// The "AI runtime" row: Test connection and what the runtime says.
fn runtime_row(
    s: &Settings,
    status: Option<&serde_json::Value>,
    d: &capture_types::RecordingSettings,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let ai = &d.ai_runtime;
    let test = &s.ui.pv.test;
    let label = |id: &str| provider_label_by_id(&ai.providers, id);
    let mut lines: Vec<AnyElement> = Vec::new();
    if test.running {
        lines.push(spin_line("Testing the default model\u{2026}", p));
    } else if let Some(Err(e)) = &test.result {
        lines.push(
            line(
                Tone::Danger,
                icons::ALERT,
                format!("Test connection failed \u{2014} {}", rejection_label(e, d)),
                p,
            )
            .into_any_element(),
        );
    } else if let Some(Ok(r)) = &test.result {
        let ok = r.bool_at(&["ok"]);
        let message = r
            .str_at(&["message"])
            .filter(|m| !m.is_empty())
            .unwrap_or("Connection succeeded.");
        let model = r
            .str_at(&["model"])
            .filter(|m| !m.is_empty())
            .unwrap_or("(none)");
        let text = format!(
            "{message} {} \u{b7} {model}",
            label(r.str_at(&["provider"]).unwrap_or_default())
        );
        lines.push(if ok {
            line(Tone::Ok, icons::CHECK, text, p).into_any_element()
        } else {
            line(Tone::Danger, icons::ALERT, text, p).into_any_element()
        });
        if let Some(raw) = r.str_at(&["rawJson"]).filter(|r| !r.is_empty()) {
            lines.push(raw_response(raw.to_string(), test.raw_open, p, cx));
        }
    } else if let Some(v) = status {
        if v.bool_at(&["available"]) {
            let m = v.at(&["defaultModel"]);
            let which = match (m.str_at(&["provider"]), m.str_at(&["model"])) {
                (Some(pv), Some(model)) => format!("{} \u{b7} {model}", label(pv)),
                _ => "(none)".into(),
            };
            lines.push(
                line(
                    Tone::Ok,
                    icons::CHECK,
                    format!("Default model {which} is configured and reachable."),
                    p,
                )
                .into_any_element(),
            );
        } else {
            let reason = v.str_at(&["reason"]).unwrap_or_default();
            let tone = reason_tone(reason);
            let icon = if tone == Tone::Danger {
                icons::ALERT
            } else {
                icons::INFO
            };
            let text = if reason.is_empty() {
                "Unavailable".to_string()
            } else {
                crate::insights::model::reason_label(reason, Some(d))
            };
            lines.push(
                line(tone, icon, format!("Isn\u{2019}t ready \u{2014} {text}"), p)
                    .into_any_element(),
            );
        }
    } else if !s.data.complete && s.data.ai_status_error.is_none() {
        lines.push(spin_line("Checking providers\u{2026}", p));
    }
    if let Some(e) = &s.data.ai_status_error {
        lines.push(line(Tone::Danger, icons::ALERT, e.clone(), p).into_any_element());
    }
    let mut r = row("AI runtime").control(
        Button::new(
            "ai-test",
            if test.running {
                "Testing\u{2026}"
            } else {
                "Test connection"
            },
        )
        .disabled(test.running || ai.default_model.is_none())
        .busy(test.running)
        .spinner(test.running)
        .on_click(click(cx, |s, cx| s.run_test_connection(cx))),
    );
    for l in lines {
        r = r.below(l);
    }
    r.build(p).into_any_element()
}

fn spin_line(t: &'static str, p: &Palette) -> AnyElement {
    crate::ui::notice::inline(
        Tone::Neutral,
        Some(crate::ui::progress::spinner(crate::theme::size::SPIN_SM).into_any_element()),
        vec![t.into_any_element()],
        p,
    )
    .into_any_element()
}

/// `<details class="pv-raw">`: "Raw response" over the reply, mono.
fn raw_response(raw: String, open: bool, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .w_full()
        .child(
            div()
                .id("ai-test-raw")
                .test_support()
                .role(gpui_kit::Role::Button)
                .aria_expanded(open)
                .cursor_pointer()
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(crate::ui::icon(
                    if open {
                        IconName::ChevronDown.path()
                    } else {
                        IconName::ChevronRight.path()
                    },
                    px(12.),
                ))
                .child("Raw response")
                .on_click(click(cx, |s, cx| {
                    s.ui.pv.test.raw_open = !s.ui.pv.test.raw_open;
                    cx.notify();
                })),
        )
        .when(open, |d| {
            d.child(
                div()
                    .id("ai-test-raw-body")
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .px(space::S3)
                    .py(space::S2)
                    .rounded(radius::MD)
                    .bg(p.surface_subtle)
                    .font_family(FONT_MONO)
                    .text_size(text::SM)
                    .child(SharedString::from(raw)),
            )
        })
        .into_any_element()
}
