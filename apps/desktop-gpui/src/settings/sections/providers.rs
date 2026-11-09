//! Intelligence › Providers (`Providers.svelte`, `ProviderRow.svelte`): the AI
//! master switch, one row per provider instance, Add provider, the global
//! default model (the picker Chat shares) and Test connection.

use capture_types::{AiEngineRef, AiProviderConfig, AiProviderKind};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, ClickEvent, Context, SharedString, Window};

use crate::chat::format::{kind_label, provider_instance_label, provider_label_by_id, url_host};
use crate::settings::model::Json;
use crate::settings::row::{line, row, section, state_word};
use crate::settings::{click, icons, on, Settings};
use crate::theme::{space, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::empty::{empty, EmptyLayout};
use crate::ui::menu::menu_item;
use crate::ui::model_picker::{EngineRef, ModelPicker, Spec, Trigger};
use crate::ui::switch::Switch;
use crate::ui::{popover, Button, Force, Tone};

const KINDS: [AiProviderKind; 6] = [
    AiProviderKind::Anthropic,
    AiProviderKind::Openai,
    AiProviderKind::Chatgpt,
    AiProviderKind::OpenaiCompatible,
    AiProviderKind::Ollama,
    AiProviderKind::Llamafile,
];

fn cloud(kind: AiProviderKind) -> bool {
    !kind.is_local()
}

pub(crate) fn local_default(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Ollama => "http://localhost:11434",
        AiProviderKind::Llamafile => "http://localhost:8080",
        _ => "",
    }
}

/// `providerRowState`: the state word and its tone.
fn row_state(pv: &AiProviderConfig, key_saved: bool, reason: &str) -> (&'static str, Option<Tone>) {
    let id = pv.id.to_lowercase();
    let reason = reason.to_lowercase();
    if reason == format!("needs_reconnect:{id}") {
        return ("needs reconnect", Some(Tone::Danger));
    }
    if reason == format!("provider_unreachable:{id}") {
        return ("unreachable", Some(Tone::Warn));
    }
    if pv.kind == AiProviderKind::OpenaiCompatible && pv.base_url.trim().is_empty() {
        return ("needs base URL", Some(Tone::Warn));
    }
    if pv.kind == AiProviderKind::Chatgpt {
        return if key_saved {
            ("connected", Some(Tone::Ok))
        } else {
            ("not connected", None)
        };
    }
    match (cloud(pv.kind), key_saved) {
        (true, false) => ("no key", None),
        (true, true) => ("key saved", Some(Tone::Ok)),
        _ => ("on this Mac", None),
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

fn provider_row(
    s: &Settings,
    pv: &AiProviderConfig,
    reason: &str,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let key_saved = s.data.keys_saved.contains(&pv.id);
    let is_cloud = cloud(pv.kind);
    let key_kind = is_cloud && pv.kind != AiProviderKind::Chatgpt;
    let open = s.ui.provider_open.as_deref() == Some(pv.id.as_str());
    let host = url_host(if pv.base_url.is_empty() {
        local_default(pv.kind)
    } else {
        &pv.base_url
    })
    .to_string();
    let desc = if open {
        key_kind.then(|| {
            if key_saved {
                "A key is saved \u{2014} paste a new one to replace it. It goes to the encrypted vault, never a config file.".to_string()
            } else {
                "Paste an API key. It goes to the encrypted vault, never a config file.".to_string()
            }
        })
    } else if pv.kind == AiProviderKind::Chatgpt {
        None
    } else if key_kind {
        let ending = s
            .data
            .key_endings
            .get(&pv.id)
            .filter(|_| key_saved)
            .map(|e| format!("\u{2026}{e}"));
        let parts: Vec<String> = [
            Some(if key_saved {
                "Key in the encrypted vault".to_string()
            } else {
                "No key yet".into()
            }),
            ending,
            (!host.is_empty()).then(|| host.clone()),
        ]
        .into_iter()
        .flatten()
        .collect();
        Some(parts.join(" \u{b7} "))
    } else {
        Some(format!("{host} \u{b7} no key, nothing leaves this Mac"))
    };
    let (word, tone) = row_state(pv, key_saved, reason);
    let id = pv.id.clone();
    let ctl = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .child(state_word(word, tone, p))
        .when(
            pv.kind != AiProviderKind::Chatgpt && (key_saved || !is_cloud),
            |d| {
                // shortcut: verify_ai_provider's live re-test result line lands with S3's ask_ai events.
                d.child(
                    Button::new(
                        SharedString::from(format!("pv-test-{}", pv.id)),
                        "Test again",
                    )
                    .ghost(),
                )
            },
        )
        .child(
            Button::new(
                SharedString::from(format!("pv-manage-{}", pv.id)),
                if open { "Done" } else { "Manage" },
            )
            .on_click(click(cx, move |s, cx| {
                s.ui.provider_open = if s.ui.provider_open.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id.clone())
                };
                cx.notify();
            })),
        );
    let label = provider_instance_label(pv);
    let r = row(label).desc_opt(desc).extra(chip(
        if is_cloud { "cloud" } else { "local" },
        ChipKind::Neutral,
        p,
    ));
    let r = if open {
        let (save_id, clear_id) = (pv.id.clone(), pv.id.clone());
        let fields = div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .w_full()
            .when(key_kind, |d| {
                d.child(
                    div()
                        .flex()
                        .gap(space::S2)
                        .child(div().flex_1().child(crate::ui::input::TextInput::new(
                            "pv-key",
                            "API key",
                            &s.inputs.provider_key,
                        )))
                        .child(Button::new("pv-save-key", "Save key").on_click(cx.listener(
                            move |s, _: &ClickEvent, window, cx| {
                                s.save_provider_key(save_id.clone(), window, cx)
                            },
                        )))
                        .child(
                            Button::new("pv-clear-key", "Clear")
                                .ghost()
                                .disabled(!key_saved)
                                .on_click(click(cx, move |s, cx| {
                                    s.clear_provider_key(clear_id.clone(), cx)
                                })),
                        ),
                )
            })
            .when_some(s.ui.key_error.clone(), |d, e| {
                d.child(line(Tone::Danger, icons::ALERT, e, p))
            });
        r.full().aside(ctl).control(fields)
    } else {
        r.control(ctl)
    };
    r.build(p).into_any_element()
}

pub fn render(s: &Settings, p: &Palette, _: &mut Window, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let ai = &d.ai_runtime;
    let status = s.data.ai_status.clone().unwrap_or_default();
    let reason = status.str_at(&["reason"]).unwrap_or_default().to_string();
    let mut rows = vec![row("Enable AI features")
        .desc("The master switch for everything AI in Mnema. While off, nothing is sent to any AI model.")
        .control(Switch::new("ai-enabled", "Enable AI features", ai.enabled).on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.ai_runtime.enabled = v, cx))))
        .build(p)
        .into_any_element()];
    let add_menu = |s: &Settings, small: bool, cx: &mut Context<Settings>| {
        let open = s.ui.add_provider_open;
        let trigger = Button::new(
            if small { "pv-add-empty" } else { "pv-add" },
            "Add provider\u{2026}",
        )
        .icon(icons::PLUS)
        .when(small, |b| b.sm())
        .on_click(click(cx, |s, cx| {
            s.ui.add_provider_open = !s.ui.add_provider_open;
            cx.notify();
        }));
        let items: Vec<AnyElement> = KINDS
            .iter()
            .map(|k| {
                let k = *k;
                menu_item(
                    SharedString::from(format!("pv-kind-{}", kind_label(k))),
                    kind_label(k),
                    Some(if cloud(k) { "cloud" } else { "local" }.into()),
                    Force::None,
                    click(cx, move |s, cx| {
                        s.ui.add_provider_open = false;
                        s.edit(
                            |d| {
                                let id = new_id(k, &d.ai_runtime.providers);
                                d.ai_runtime.providers.push(AiProviderConfig {
                                    id,
                                    kind: k,
                                    label: String::new(),
                                    base_url: String::new(),
                                });
                            },
                            cx,
                        );
                    }),
                    p,
                )
                .into_any_element()
            })
            .collect();
        div().relative().child(trigger).when(open, |d| {
            d.child(
                popover("pv-add-menu", "Add provider", p)
                    .top(px(36.))
                    .right_0()
                    .w(px(220.))
                    .p(px(4.))
                    .children(items),
            )
        })
    };
    if ai.providers.is_empty() {
        rows.push(
            div()
                .py(space::S3)
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
            rows.push(provider_row(s, pv, &reason, p, cx));
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
        selected: ai
            .default_model
            .as_ref()
            .map(|m| (Some(m.provider.clone()), m.model.clone())),
        first_provider: ai.default_model.as_ref().map(|m| m.provider.clone()),
        can_star: true,
        ..Default::default()
    };
    let picker = ModelPicker::new("default-model", spec)
        .on_open({
            let this = cx.entity().downgrade();
            move |_, cx| {
                let _ = this.update(cx, |s, cx| s.load_pool(cx));
            }
        })
        .trigger(Trigger::Block {
            label: label
                .clone()
                .unwrap_or_else(|| "Choose a default model".into()),
            placeholder: label.is_none(),
        })
        .on_select(on(cx, |s, pick: Option<EngineRef>, cx| {
            if let Some((provider, model)) = pick {
                s.edit(
                    |d| d.ai_runtime.default_model = Some(AiEngineRef { provider, model }),
                    cx,
                )
            }
        }));
    rows.push(
        row("Global default model")
            .desc("Used by every AI feature unless it sets its own override.")
            .control(div().w(px(300.)).child(picker))
            .build(p)
            .into_any_element(),
    );
    let runtime_line = if status.bool_at(&["available"]) {
        let m = status.at(&["defaultModel"]);
        let which = match (m.str_at(&["provider"]), m.str_at(&["model"])) {
            (Some(pv), Some(model)) => {
                format!("{} \u{b7} {model}", provider_label_by_id(&ai.providers, pv))
            }
            _ => "(none)".into(),
        };
        Some(line(
            Tone::Ok,
            icons::CHECK,
            format!("Default model {which} is configured and reachable."),
            p,
        ))
    } else if !reason.is_empty() {
        Some(line(
            Tone::Warn,
            icons::INFO,
            format!("Isn\u{2019}t ready \u{2014} {reason}"),
            p,
        ))
    } else {
        None
    };
    rows.push(
        row("AI runtime")
            // shortcut: ai_runtime_test_connection is in app-core but its result panel lands with
            // S3's ask_ai status events; the button is shown, the run is logged only.
            .control(Button::new("ai-test", "Test connection").disabled(ai.default_model.is_none()))
            .when(runtime_line.is_some(), |r| r.below(runtime_line.unwrap()))
            .build(p)
            .into_any_element(),
    );
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
