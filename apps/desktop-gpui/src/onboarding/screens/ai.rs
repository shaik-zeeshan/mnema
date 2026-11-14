//! Change settings › AI features (`AiSetup.svelte`, `AiScanList.svelte`,
//! `AiModelPicker.svelte`, mockup `#ob-ai-*`): the later / now fork, then the
//! provider kind, the key (or the ChatGPT sign-in, or a local scan), the
//! connections and the default model. Skipping stays a first-class choice.

use capture_types::AiProviderKind;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Hsla, SharedString, Window};

use super::{button, on_ix};
use crate::chat::format::{kind_label, provider_instance_label};
use crate::onboarding::ai::{host_of, Check, Failure, Login, Probe, SCAN_TARGETS};
use crate::onboarding::flow::Feature;
use crate::onboarding::parts::{self, fine, head};
use crate::onboarding::Onboarding;
use crate::settings::icons;
use crate::theme::{self, size, space, text, Palette};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::input::TextInput;
use crate::ui::model_picker::{EngineRef, ModelPicker, PoolModel, Spec, Trigger};
use crate::ui::notice::{inline, notice};
use crate::ui::progress::spinner;
use crate::ui::select::Select;
use crate::ui::switch::Switch;
use crate::ui::Tracking as _;
use crate::ui::{icon, kicker, Tone};

/// The kind picker, local first (the Svelte optgroups' order).
const KINDS: [AiProviderKind; 6] = [
    AiProviderKind::Ollama,
    AiProviderKind::Llamafile,
    AiProviderKind::Anthropic,
    AiProviderKind::Openai,
    AiProviderKind::Chatgpt,
    AiProviderKind::OpenaiCompatible,
];

fn lamp(color: Hsla) -> gpui_kit::Div {
    div().flex_none().size(px(7.)).rounded_full().bg(color)
}

fn line(tone: Tone, lead: AnyElement, text_: String, p: &Palette) -> gpui_kit::Div {
    inline(
        tone,
        Some(lead),
        vec![div().child(text_).into_any_element()],
        p,
    )
}

fn check_word(c: Option<&Check>) -> String {
    match c {
        Some(Check::Live { models, latency_ms }) => format!(
            "{} model{} \u{b7} {latency_ms} ms",
            models.len(),
            if models.len() == 1 { "" } else { "s" }
        ),
        Some(Check::Checking) => "checking\u{2026}".into(),
        Some(Check::Failed(f)) => f.word(),
        None => "not tested".into(),
    }
}

impl Onboarding {
    pub(super) fn ai_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Vec<AnyElement> {
        self.ai_inputs(window, cx);
        let later = self.ai.mode == Some(false);
        let mut v = vec![head(vec![
            kicker(if later { "AI features \u{b7} off" } else { "AI features \u{b7} optional" }, p).into_any_element(),
            fine("Ask AI, daily digests and User Context need a reasoning engine. Recording, search, transcription and speaker names do not.", p).into_any_element(),
        ])
        .into_any_element()];
        if self.features.ai_features && !self.ai_ready() {
            v.push(
                notice(
                    Tone::Warn,
                    icons::ALERT,
                    "The AI features row is on, with nothing to run it",
                    Some("Connect a provider here, or switch the row off.".into()),
                    vec![],
                    p,
                )
                .into_any_element(),
            );
        }
        let opts = vec![
            ChoiceOption::new("Set this up later")
                .desc("Skipping downloads nothing and needs no account."),
            ChoiceOption::new("I have a key \u{2014} set it up now")
                .desc("A local engine on this Mac, or your own cloud account."),
        ];
        let selected = self.ai.mode.map_or(usize::MAX, |now| now as usize);
        v.push(
            div()
                .w_full()
                .child(
                    Choice::new("ob-ai-fork", "AI features", opts, selected)
                        .on_change(on_ix(cx, |this, ix, cx| this.ai_set_mode(ix == 1, cx))),
                )
                .into_any_element(),
        );
        match self.ai.mode {
            Some(false) => v.push(fine("Ask AI, daily digests and User Context stay dark; everything else keeps working. Turn it on later at Settings \u{2192} Intelligence \u{2192} AI features.", p).into_any_element()),
            Some(true) => v.extend(self.ai_now(cx, p)),
            None => {}
        }
        v
    }

    fn ai_now(&self, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        let ai = &self.ai;
        let kind = ai.kind;
        let local = kind.is_local();
        let providers = &self.draft.ai_runtime.providers;
        let status = match (&self.draft.ai_runtime.default_model, self.ai_missing()) {
            (Some(d), None) => format!("Ready \u{2014} {} will answer.", d.model),
            (_, Some(why)) => why,
            (None, None) => String::new(),
        };
        let this = cx.entity().downgrade();
        // `.ob-ai-row`: hairlines above and below.
        let mut v = vec![div()
            .flex()
            .items_center()
            .gap(space::S4)
            .py(px(10.))
            .border_t_1()
            .border_b_1()
            .border_color(p.hairline)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MD)
                            .text_color(p.text_strong)
                            .child("AI features"),
                    )
                    .child(
                        div()
                            .text_size(text::BASE)
                            .line_height(relative(1.35))
                            .text_color(p.text_subtle)
                            .child(status),
                    ),
            )
            .child(
                Switch::new("ob-ai-switch", "AI features", self.features.ai_features).on_toggle({
                    let this = this.clone();
                    move |_, _, cx| {
                        let _ = this.update(cx, |o, cx| o.toggle(Feature::AiFeatures, cx));
                    }
                }),
            )
            .into_any_element()];

        v.push(
            div()
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_size(text::MD)
                        .text_color(p.text_strong)
                        .child(if providers.is_empty() {
                            "Connect an engine"
                        } else {
                            "Connect another engine"
                        }),
                )
                .child(fine(
                    if local {
                        "If one is already running here, nothing has to be typed."
                    } else if kind == AiProviderKind::Chatgpt {
                        "Your own ChatGPT Plus/Pro plan. Sign in in the browser \u{2014} no API key."
                    } else {
                        "Your own account. There is no Save \u{2014} the key proves itself."
                    },
                    p,
                ))
                .into_any_element(),
        );
        // The connect row: kind, then what that kind needs.
        let kind_ix = KINDS.iter().position(|k| *k == kind);
        let mut row = vec![div()
            .w(px(200.))
            .child(
                Select::new("ob-ai-kind", "Provider", KINDS.map(kind_label), kind_ix).on_change(
                    on_ix(cx, |this, ix, cx| this.ai_set_kind(KINDS[ix.min(5)], cx)),
                ),
            )
            .into_any_element()];
        let chatgpt = providers.iter().find(|p| p.kind == AiProviderKind::Chatgpt);
        if local {
            let scanning = ai
                .scan
                .as_ref()
                .is_some_and(|s| s.iter().any(|r| *r == Probe::Trying));
            let label = if scanning {
                "Scanning\u{2026}"
            } else if ai.scan.is_some() {
                "Scan again"
            } else {
                "Scan this Mac"
            };
            row.push(
                button("ob-ai-scan", label, cx, |this, _, cx| this.ai_scan(cx))
                    .disabled(scanning)
                    .into_any_element(),
            );
        } else if kind == AiProviderKind::Chatgpt {
            let starting = matches!(ai.login, Some((_, Login::Starting)));
            let waiting = matches!(ai.login, Some((_, Login::Waiting { .. })));
            let connected = chatgpt.is_some_and(|c| ai.keys_saved.contains(&c.id));
            if !waiting {
                let label = if starting {
                    "Contacting OpenAI\u{2026}"
                } else if connected {
                    "Sign in again"
                } else {
                    "Sign in with ChatGPT"
                };
                row.push(
                    button("ob-ai-chatgpt", label, cx, |this, _, cx| {
                        this.ai_chatgpt_sign_in(cx)
                    })
                    .busy(starting)
                    .disabled(starting || ai.removing)
                    .into_any_element(),
                );
            }
        } else if let Some(key) = &ai.key {
            let rejected = ai.focus.as_ref().and_then(|f| ai.checks.get(f))
                == Some(&Check::Failed(Failure::Rejected));
            row.push(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(TextInput::new("ob-ai-key", "API key", key).invalid(rejected))
                    .into_any_element(),
            );
            row.push(
                button("ob-ai-verify", "Verify now", cx, |this, window, cx| {
                    this.ai_connect_cloud(window, cx)
                })
                .disabled(ai.removing)
                .into_any_element(),
            );
        }
        v.push(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .children(row)
                .into_any_element(),
        );
        if kind == AiProviderKind::OpenaiCompatible {
            if let Some(url) = &ai.url {
                v.push(TextInput::new("ob-ai-base-url", "Base URL", url).into_any_element());
            }
        }
        if let Some((_, Login::Waiting { code, .. })) = &ai.login {
            v.push(self.ai_code_block(code.clone(), cx, p));
        }
        if let Some(l) = self.ai_status_line(cx, p) {
            v.push(l);
        }
        let host = host_of(kind, "");
        v.push(
            parts::mono(
                format!(
                    "this Mac \u{2014}\u{2014} {host}  {}",
                    if local {
                        "\u{b7} nothing leaves"
                    } else {
                        "\u{b7} prompts and their context leave"
                    }
                ),
                text::SM,
                if local { p.text_subtle } else { p.warn },
            )
            .into_any_element(),
        );
        if local {
            v.extend(self.ai_scan_rows(cx, p));
            if let Some(url) = &ai.endpoint {
                v.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(space::S2)
                        .child(div().flex_1().child(TextInput::new(
                            "ob-ai-endpoint",
                            "Endpoint on another machine",
                            url,
                        )))
                        .child(button(
                            "ob-ai-endpoint-go",
                            "Connect",
                            cx,
                            |this, window, cx| this.ai_connect_manual(window, cx),
                        ))
                        .into_any_element(),
                );
            }
        }
        for pv in providers {
            v.push(self.ai_card(pv, cx, p));
        }
        v.push(self.ai_default_row(cx, p));
        if let Some(note) = &ai.restored_note {
            v.push(
                line(
                    Tone::Warn,
                    icon(icons::ALERT, size::ICON_SM).into_any_element(),
                    note.clone(),
                    p,
                )
                .into_any_element(),
            );
        }
        if providers
            .iter()
            .any(|p| !p.kind.is_local() && p.kind != AiProviderKind::Chatgpt)
        {
            v.push(fine("The key goes into the app\u{2019}s encrypted vault (day.mnema.vault, unlocked by one keychain item) \u{2014} never a config file, and this window forgets the string the moment it is sent.", p).into_any_element());
        }
        if chatgpt.is_some() {
            v.push(fine("The ChatGPT sign-in token is stored in the app\u{2019}s encrypted vault \u{2014} never in a config file. Usage counts against your own plan.", p).into_any_element());
        }
        v
    }

    fn ai_code_block(&self, code: String, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let copy = code.clone();
        div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .child(fine(
                "Enter this code at auth.openai.com/codex/device \u{2014} waiting for approval\u{2026}",
                p,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .child(parts::mono(code, text::XL, p.text_strong).tracking(0.12))
                    .child(
                        // `ChatgptConnect.svelte`: "Copied" for 1.5 s.
                        crate::ui::CopyButton::label("ob-ai-copy-code", "Copy code", copy)
                            .revert(crate::chat::turn_error::CODE_COPIED_FOR),
                    )
                    .child(
                        button("ob-ai-open-page", "Open the page", cx, |this, _, cx| {
                            this.ai_chatgpt_open(cx)
                        })
                        .icon(icons::EXTERNAL)
                        .sm(),
                    )
                    .child(
                        button("ob-ai-cancel-login", "Cancel", cx, |this, _, cx| {
                            this.ai_chatgpt_cancel(cx)
                        })
                        .ghost()
                        .sm(),
                    ),
            )
            .into_any_element()
    }

    /// The one line under the connect row: the focused instance's proof.
    fn ai_status_line(&self, cx: &mut Context<Self>, p: &Palette) -> Option<AnyElement> {
        let alert = || icon(icons::ALERT, size::ICON_SM).into_any_element();
        if let Some(note) = &self.ai.note {
            return Some(line(Tone::Danger, alert(), note.clone(), p).into_any_element());
        }
        let id = self.ai.focus.clone()?;
        let pv = self
            .draft
            .ai_runtime
            .providers
            .iter()
            .find(|p| p.id == id)?;
        let label = provider_instance_label(pv);
        let host = host_of(pv.kind, &pv.base_url);
        let el = match self.ai.checks.get(&id)? {
            Check::Checking => line(
                Tone::Neutral,
                spinner(size::SPIN_SM).into_any_element(),
                format!("Verifying {label}\u{2026}"),
                p,
            ),
            Check::Live { models, latency_ms } => line(
                Tone::Ok,
                icon(icons::CHECK, size::ICON_SM).into_any_element(),
                format!(
                    "{} model{} listed in {latency_ms} ms",
                    models.len(),
                    if models.len() == 1 { "" } else { "s" }
                ),
                p,
            ),
            Check::Failed(Failure::Rejected) => line(
                Tone::Danger,
                alert(),
                format!("{label} rejected that key \u{2014} authentication failed."),
                p,
            )
            .child(
                button("ob-ai-another-key", "Paste another key", cx, move |this, _, cx| {
                    this.ai_remove(id.clone(), cx)
                })
                .ghost()
                .sm(),
            ),
            Check::Failed(Failure::Unreachable) => line(
                Tone::Warn,
                icon(icons::INFO, size::ICON_SM).into_any_element(),
                format!("Can\u{2019}t reach {host} right now \u{2014} check the connection (or that the engine is running). Nothing was judged."),
                p,
            )
            .child(
                button("ob-ai-retry", "Try again", cx, move |this, _, cx| {
                    this.ai_verify(id.clone(), cx)
                })
                .ghost()
                .sm(),
            ),
            Check::Failed(Failure::NeedsSignIn) => line(
                Tone::Warn,
                alert(),
                "Sign in with ChatGPT to finish connecting.".into(),
                p,
            ),
            Check::Failed(Failure::Other(why)) => line(
                Tone::Danger,
                alert(),
                format!("{label} is not answering \u{2014} {why}."),
                p,
            ),
        };
        Some(el.into_any_element())
    }

    fn ai_scan_rows(&self, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        let Some(rows) = &self.ai.scan else {
            return Vec::new();
        };
        let mut v: Vec<AnyElement> = SCAN_TARGETS
            .iter()
            .zip(rows)
            .map(|((port, kind, name), probe)| {
                let (port, kind) = (*port, *kind);
                let (lamp_c, models, latency) = match probe {
                    Probe::Trying => (p.text_faint, None, "\u{2026}".to_string()),
                    Probe::Dead => (p.text_faint, None, "no answer".to_string()),
                    Probe::Live { models, latency_ms } => {
                        (p.accent, Some(*models), format!("{latency_ms} ms"))
                    }
                };
                let act: Option<AnyElement> = models.map(|_| {
                    if self.ai_port_connected(port) {
                        chip("connected", ChipKind::Neutral, p).into_any_element()
                    } else {
                        button(
                            SharedString::from(format!("ob-ai-use-{port}")),
                            "Use this",
                            cx,
                            move |this, _, cx| {
                                this.ai_connect_local(kind, format!("http://localhost:{port}"), cx)
                            },
                        )
                        .sm()
                        .into_any_element()
                    }
                });
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .py(space::S1)
                    .text_size(text::BASE)
                    .text_color(if models.is_some() {
                        p.text
                    } else {
                        p.text_subtle
                    })
                    .child(lamp(lamp_c))
                    .child(
                        div()
                            .flex_1()
                            .child(format!("localhost:{port} \u{b7} {name}")),
                    )
                    .children(models.map(|n| {
                        chip(
                            format!("{n} model{}", if n == 1 { "" } else { "s" }),
                            if n > 0 {
                                ChipKind::Accent
                            } else {
                                ChipKind::Warn
                            },
                            p,
                        )
                    }))
                    .child(
                        div()
                            .w(px(72.))
                            .child(parts::mono(latency, text::SM, p.text_subtle)),
                    )
                    .children(act)
                    .into_any_element()
            })
            .collect();
        if rows.iter().all(|r| *r == Probe::Dead) {
            v.push(notice(Tone::Warn, icons::INFO, "Nothing is listening on the usual ports", Some("That is an answer, not an error. Start Ollama and scan again, or pick a cloud account above.".into()), vec![], p).into_any_element());
        } else if matches!(rows.first(), Some(Probe::Live { models: 0, .. })) {
            v.push(line(Tone::Warn, icon(icons::ALERT, size::ICON_SM).into_any_element(), "Ollama answered, but has no models to run. Pull one (ollama pull llama3.2), then scan again.".into(), p).into_any_element());
        }
        v
    }

    fn ai_card(
        &self,
        pv: &capture_types::AiProviderConfig,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let check = self.ai.checks.get(&pv.id);
        let (lamp_c, kind) = match check {
            Some(Check::Live { .. }) => (p.accent, ChipKind::Accent),
            Some(Check::Failed(Failure::Unreachable)) => (p.warn, ChipKind::Warn),
            Some(Check::Failed(_)) => (p.danger, ChipKind::Danger),
            _ => (p.text_faint, ChipKind::Neutral),
        };
        let is_default = self
            .draft
            .ai_runtime
            .default_model
            .as_ref()
            .is_some_and(|d| d.provider == pv.id);
        let (test_id, remove_id) = (pv.id.clone(), pv.id.clone());
        div()
            .flex()
            .items_center()
            .gap(space::S2)
            .px(space::S3)
            .py(space::S2)
            .rounded(theme::radius::LG)
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            // `.ob-lamp.live`: a 3 px accent-glow halo.
            .child(
                lamp(lamp_c).when(matches!(check, Some(Check::Live { .. })), |d| {
                    d.shadow(vec![crate::ui::ring(p.accent_glow)])
                }),
            )
            .child(
                div()
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::MD)
                    .text_color(p.text_strong)
                    .child(provider_instance_label(pv)),
            )
            .when(is_default, |d| d.child(chip("default", ChipKind::Info, p)))
            .child(chip(check_word(check), kind, p))
            .when(
                !pv.kind.is_local() && self.ai.keys_saved.contains(&pv.id),
                |d| {
                    d.child(chip(
                        if pv.kind == AiProviderKind::Chatgpt {
                            "signed in"
                        } else {
                            "key in vault"
                        },
                        ChipKind::Neutral,
                        p,
                    ))
                },
            )
            .child(div().flex_1())
            .child(
                button(
                    SharedString::from(format!("ob-ai-test-{}", pv.id)),
                    "Test",
                    cx,
                    move |this, _, cx| this.ai_verify(test_id.clone(), cx),
                )
                .sm()
                .disabled(check == Some(&Check::Checking)),
            )
            .child(
                button(
                    SharedString::from(format!("ob-ai-remove-{}", pv.id)),
                    "Remove",
                    cx,
                    move |this, _, cx| this.ai_remove(remove_id.clone(), cx),
                )
                .ghost()
                .sm()
                .disabled(self.ai.removing),
            )
            .into_any_element()
    }

    /// Only models a connected engine listed just now are offerable.
    fn ai_default_row(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let ai = &self.draft.ai_runtime;
        let pool: Vec<PoolModel> = ai
            .providers
            .iter()
            .flat_map(|pv| {
                self.ai
                    .checks
                    .get(&pv.id)
                    .map(|c| c.models().to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|id| PoolModel {
                        id,
                        provider: pv.id.clone(),
                        context_window: None,
                    })
            })
            .collect();
        let label = ai.default_model.as_ref().map(|d| d.model.clone());
        let spec = Spec {
            providers: ai.providers.clone(),
            pool,
            selected: ai
                .default_model
                .as_ref()
                .map(|d| (Some(d.provider.clone()), d.model.clone())),
            first_provider: ai.default_model.as_ref().map(|d| d.provider.clone()),
            ..Default::default()
        };
        let this = cx.entity().downgrade();
        let picker = ModelPicker::new("ob-ai-model", spec)
            .trigger(Trigger::Block {
                label: label
                    .clone()
                    .unwrap_or_else(|| "Choose a default model".into()),
                placeholder: label.is_none(),
            })
            .on_select(move |pick: Option<EngineRef>, _, cx| {
                if let Some((provider, model)) = pick {
                    let _ = this.update(cx, |o, cx| o.ai_pick(provider, model, cx));
                }
            });
        div()
            .flex()
            .items_center()
            .gap(space::S4)
            .py(px(10.))
            .border_t_1()
            .border_b_1()
            .border_color(p.hairline)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MD)
                            .text_color(p.text_strong)
                            .child("Default model"),
                    )
                    .child(fine(
                        if ai.providers.is_empty() {
                            "Nothing to choose from yet \u{2014} connect an engine first."
                        } else {
                            "Only models a connected engine listed just now."
                        },
                        p,
                    )),
            )
            .child(div().w(px(300.)).child(picker))
            .into_any_element()
    }
}
