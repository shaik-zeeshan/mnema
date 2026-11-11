//! Intelligence › Transcription (`Transcription.svelte`): master + per-source
//! switches, the provider list (Deepgram asks for consent), the model row,
//! language, Parakeet memory options and delete unused.

use capture_types::{AudioTranscriptionMemoryMode, AudioTranscriptionProvider};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::model_row::{delete_unused_row, model_files, no_status_row, status_row, ModelRow};
use super::{ix, languages};
use crate::settings::data::Family;
use crate::settings::model::{provider_models, selected_model};
use crate::settings::row::{row, section, Src};
use crate::settings::{confirm, icons, on, reload, Settings};
use crate::theme::Palette;
use crate::ui::chip::ChipKind;
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::combobox::Combobox;
use crate::ui::input::TextInput;
use crate::ui::segmented::Segmented;
use crate::ui::stepper::Stepper;
use crate::ui::switch::Switch;
use crate::ui::Button;

const PROVIDERS: [AudioTranscriptionProvider; 4] = [
    AudioTranscriptionProvider::Parakeet,
    AudioTranscriptionProvider::LocalWhisper,
    AudioTranscriptionProvider::AppleSpeechOnDevice,
    AudioTranscriptionProvider::Deepgram,
];
const MEMORY: [AudioTranscriptionMemoryMode; 3] = [
    AudioTranscriptionMemoryMode::Balanced,
    AudioTranscriptionMemoryMode::LowMemory,
    AudioTranscriptionMemoryMode::Performance,
];

fn id(p: AudioTranscriptionProvider) -> &'static str {
    match p {
        AudioTranscriptionProvider::Parakeet => "parakeet",
        AudioTranscriptionProvider::LocalWhisper => "local_whisper",
        AudioTranscriptionProvider::AppleSpeechOnDevice => "apple_speech_on_device",
        AudioTranscriptionProvider::Deepgram => "deepgram",
    }
}

fn choose(s: &mut Settings, i: usize, cx: &mut gpui_kit::Context<Settings>) {
    let next = PROVIDERS[i];
    let set = move |s: &mut Settings, cx: &mut gpui_kit::Context<Settings>| {
        s.edit(
            |d| {
                d.transcription.provider = next;
                d.transcription.model_id = None;
            },
            cx,
        )
    };
    if next == AudioTranscriptionProvider::Deepgram && s.d().transcription.provider != next {
        confirm(
            cx,
            "Send audio to Deepgram?",
            "Switching to Deepgram uploads your microphone and system-audio recordings to Deepgram's cloud service, under your own Deepgram account and data policies. Only audio recorded from now on is affected \u{2014} existing transcripts stay on your device. Continue?",
            "Use Deepgram",
            set,
        );
    } else {
        set(s, cx);
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let t = &d.transcription;
    let off = !t.enabled;
    let opts = vec![
        ChoiceOption::new("Parakeet").icon(icons::CHIP).tag("on-device").desc("Fast, English-first transcription on the Neural Engine."),
        ChoiceOption::new("Local Whisper").icon(icons::CHIP).tag("on-device").desc("Many languages; slower and heavier on memory."),
        ChoiceOption::new("Apple Speech").icon(icons::SPEECH).tag("on-device").desc("Built into macOS; needs Speech Recognition permission."),
        ChoiceOption::new("Deepgram")
            .icon(icons::CLOUD)
            .tag("cloud")
            .desc("Uploads mic and system audio to your own Deepgram account. Needs an API key and your consent."),
    ];
    let mut rows = vec![
        row("Enable audio transcription")
            .desc("Master switch for source-specific audio transcription.")
            .control(Switch::new("tx-enabled", "Enable audio transcription", t.enabled).on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.transcription.enabled = v, cx))))
            .build(p)
            .into_any_element(),
        row("Transcribe microphone")
            .src(Src::Mic)
            .disabled(off)
            .control(
                Switch::new("tx-mic", "Transcribe microphone", t.microphone_enabled)
                    .disabled(off).on_color(p.source_mic)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.transcription.microphone_enabled = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Transcribe system audio")
            .src(Src::SysAudio)
            .desc("Transcribe system audio only when speech is detected.")
            .disabled(off)
            .control(
                Switch::new("tx-sys", "Transcribe system audio", t.system_audio_enabled)
                    .disabled(off).on_color(p.source_sysaudio)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.transcription.system_audio_enabled = v, cx))),
            )
            .build(p)
            .into_any_element(),
        row("Provider")
            .desc("Switching to Deepgram asks for consent first \u{2014} it uploads mic and system audio.")
            .full()
            .control(div().w_full().child(Choice::new("tx-provider", "Transcription provider", opts, ix(&PROVIDERS, &t.provider)).list().on_change(on(cx, choose))))
            .build(p)
            .into_any_element(),
    ];
    if t.provider == AudioTranscriptionProvider::Deepgram {
        let key_desc = if s.data.deepgram_key {
            "Key stored encrypted on this Mac."
        } else {
            "Paste the API key from your Deepgram account."
        };
        // The standing rejection on file (`transcription_deepgram_auth_status`).
        let rejected = s.data.deepgram_auth.clone().map(|problem| {
            crate::settings::row::line(crate::ui::Tone::Danger, icons::ALERT, problem, p)
        });
        use crate::settings::{commands, live};
        rows.push(
            row("Deepgram API key")
                .desc(key_desc)
                .full()
                .control(
                    div()
                        .flex()
                        .gap(crate::theme::space::S2)
                        .w_full()
                        .child(div().flex_1().child(TextInput::new(
                            "deepgram-key",
                            "Deepgram API key",
                            &s.inputs.deepgram_key,
                        )))
                        .child(
                            Button::new("deepgram-save", "Save key")
                                .disabled(s.ui.busy.contains("Deepgram key"))
                                .on_click(commands::click_w(cx, |s, window, cx| {
                                    let input = s.inputs.deepgram_key.clone();
                                    let key = commands::take_input(&input, window, cx);
                                    if !key.is_empty() {
                                        s.run_command("Deepgram key", cx, move |core| {
                                            live::deepgram_save(core, key)
                                        });
                                    }
                                })),
                        )
                        .when(s.data.deepgram_key, |d| {
                            d.child(
                                Button::new("deepgram-check", "Check connection")
                                    .ghost()
                                    .on_click(crate::settings::click(cx, |s, cx| {
                                        s.run_command("Deepgram key", cx, live::deepgram_test)
                                    })),
                            )
                            .child(
                                Button::new("deepgram-clear", "Remove key")
                                    .ghost()
                                    .on_click(crate::settings::click(cx, |s, cx| {
                                        s.run_command("Deepgram key", cx, live::deepgram_clear)
                                    })),
                            )
                        }),
                )
                .when(rejected.is_some(), |r| {
                    r.below(rejected.unwrap_or_else(div))
                })
                .build(p)
                .into_any_element(),
        );
    }
    let status = s.data.transcription_models.as_ref();
    let m = selected_model(status, id(t.provider), t.model_id.as_deref());
    let models: Vec<(Option<String>, String)> = provider_models(status)
        .into_iter()
        .find(|(pv, _, _)| pv == id(t.provider))
        .map(|(_, _, ms)| {
            ms.into_iter()
                .map(|m| (m.model_id, m.display_name))
                .collect()
        })
        .unwrap_or_default();
    let select = (models.len() > 1).then(|| {
        let ids: Vec<Option<String>> = models.iter().map(|(i, _)| i.clone()).collect();
        let sel = m
            .as_ref()
            .and_then(|m| ids.iter().position(|i| *i == m.model_id));
        div()
            .w(px(240.))
            .child(
                Combobox::new(
                    "tx-model",
                    "Transcription model",
                    models.iter().map(|(_, n)| n.clone()),
                    sel,
                )
                .search_placeholder("Search models\u{2026}")
                .on_change(on(cx, move |s, i: usize, cx| {
                    let id = ids[i].clone();
                    s.edit(|d| d.transcription.model_id = id, cx)
                })),
            )
            .into_any_element()
    });
    // Apple Speech: denied / not asked / unusable is a permission notice, never "missing".
    let speech = m
        .as_ref()
        .filter(|m| m.provider == "apple_speech_on_device")
        .and_then(|m| m.availability.clone())
        .filter(|a| a != "available");
    if let Some(a) = &speech {
        rows.push(apple_speech_notice(s, a, p, cx));
    } else {
        let mut model = match &m {
            None => no_status_row(
                s,
                Family::Transcription,
                "Model",
                "Couldn\u{2019}t load model status: ",
                "Checking installed transcription models\u{2026}",
                "No model status is available for the selected provider.",
            ),
            Some(m) if m.os_managed => ModelRow {
                label: "Model",
                desc: Some(if t.provider == AudioTranscriptionProvider::Deepgram {
                    "Deepgram runs in the cloud. Availability depends on the API key above; there is no local model to download.".into()
                } else {
                    format!(
                        "{} \u{b7} {}",
                        m.display_name,
                        if m.availability.is_some() {
                            // `transcriptionStatusLabel`: Apple Speech names its permission.
                            "permission granted"
                        } else if m.available {
                            "available"
                        } else {
                            "unavailable"
                        }
                    )
                }),
                status: (m.available && t.provider != AudioTranscriptionProvider::Deepgram)
                    .then_some("available"),
                chip: (!m.available).then(|| ("unavailable".into(), ChipKind::Warn)),
                files: model_files(m),
                ..Default::default()
            },
            Some(m) => {
                let failed = m.status == "failed";
                let mut r = status_row("Model", m, failed, "available");
                r.desc = Some(if m.available {
                    format!("{} \u{b7} installed", m.display_name)
                } else if failed {
                    m.display_name.clone()
                } else {
                    let missing = if m.missing_files > 0 {
                        format!(
                            " ({} {} missing)",
                            m.missing_files,
                            if m.missing_files == 1 {
                                "file"
                            } else {
                                "files"
                            }
                        )
                    } else {
                        String::new()
                    };
                    format!(
                        "{} isn\u{2019}t downloaded{missing}. Audio waits until it is.",
                        m.display_name
                    )
                });
                r.folder = status
                    .and_then(|v| v["modelsDirectory"].as_str())
                    .map(str::to_string);
                r.start = m
                    .model_id
                    .clone()
                    .map(|id| (Family::Transcription, m.provider.clone(), id));
                r.name = Some(m.display_name.clone());
                r.running_desc = Some(
                    "Download is checked (sha256) before it\u{2019}s marked installed.".into(),
                );
                r
            }
        };
        model.controls.extend(select);
        rows.push(model.build("tx-model-row", s, p, cx));
    }
    let stored = if t.language.is_empty() {
        "auto"
    } else {
        t.language.as_str()
    };
    let (sel, shown) = languages::pick(&languages::TRANSCRIPTION, stored);
    rows.push(
        row("Language")
            .desc("Auto detects the spoken language, or pick a hint. Applies to future audio; queued jobs keep theirs.")
            .control(
                div().w(px(240.)).child(
                    Combobox::new(
                        "tx-language",
                        "Transcription language",
                        languages::TRANSCRIPTION.iter().map(|(_, l)| *l),
                        sel,
                    )
                    .placeholder(shown)
                    .search_placeholder("Search languages\u{2026}")
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit(|d| d.transcription.language = languages::TRANSCRIPTION[i].0.into(), cx)
                    })),
                ),
            )
            .build(p)
            .into_any_element(),
    );
    if t.provider == AudioTranscriptionProvider::Parakeet {
        rows.push(
            row("Parakeet memory mode")
                .control(
                    Segmented::new(
                        "tx-memory",
                        "Parakeet memory mode",
                        ["Balanced", "Low memory", "Performance"],
                        ix(&MEMORY, &t.memory_mode),
                    )
                    .md()
                    .on_change(on(cx, |s, i: usize, cx| {
                        s.edit(|d| d.transcription.memory_mode = MEMORY[i].clone(), cx)
                    })),
                )
                .build(p)
                .into_any_element(),
        );
        if t.memory_mode == AudioTranscriptionMemoryMode::Balanced {
            rows.push(
                row("Idle unload")
                    .desc("Free the model after this long without audio.")
                    .control(
                        Stepper::new("tx-idle", "idle unload seconds", &s.inputs.idle_unload)
                            .range(Some(0), Some(1800))
                            .step(30)
                            .unit("s"),
                    )
                    .build(p)
                    .into_any_element(),
            );
        }
        rows.push(
            row("Chunk seconds")
                .desc("Chunking limits peak memory; 0 turns it off. The int8 model also lowers disk and memory use.")
                .control(Stepper::new("tx-chunk", "chunk seconds", &s.inputs.chunk).range(Some(0), Some(300)).step(15).unit("s"))
                .build(p)
                .into_any_element(),
        );
    }
    if m.is_some() {
        rows.push(delete_unused_row(
            "tx-delete-unused",
            Family::Transcription,
            "Removes app-managed transcription model files except the selected one.",
            p,
            cx,
        ));
    }
    section(
        "Transcription",
        Some("Speech-to-text for microphone and system audio."),
        vec![reload::button(
            s,
            "reload-tx",
            "Refresh transcription model status",
            s.ui.models_loading,
            |s, cx| s.reload_all(cx),
            cx,
        )],
        false,
        rows,
        p,
    )
    .into_any_element()
}

/// `appleSpeechPermissionLabel` for the states that reach the notice.
fn speech_label(status: &str) -> &'static str {
    match status {
        "permission_not_determined" => "Permission not requested",
        "permission_denied" => "Permission denied",
        "permission_restricted" => "Permission restricted",
        "unsupported_platform" => "Unsupported platform",
        "framework_unavailable" => "Speech framework unavailable",
        "recognizer_unavailable" => "Recognizer unavailable",
        "on_device_recognition_unavailable" => "On-device recognition unavailable",
        _ => "Permission granted",
    }
}

/// `appleSpeechPermissionHint`.
fn speech_hint(status: &str) -> &'static str {
    match status {
        "permission_not_determined" => "Mnema has not asked macOS for Speech Recognition permission yet. Request it before recording with Apple Speech selected.",
        "permission_denied" => "macOS denied Speech Recognition permission. Enable it in System Settings \u{2192} Privacy & Security \u{2192} Speech Recognition, then refresh.",
        "permission_restricted" => "macOS reports Speech Recognition permission is restricted by policy or parental controls.",
        _ => "Apple Speech cannot be used until this macOS availability check passes.",
    }
}

/// The Model row's Apple Speech permission notice: Get permission while
/// unasked, Open System Settings after a denial / restriction.
fn apple_speech_notice(
    s: &Settings,
    status: &str,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    use crate::settings::live;
    let busy = s.ui.busy.contains("Speech Recognition");
    let act = match status {
        "permission_not_determined" => Some(
            Button::new(
                "speech-allow",
                if busy { "Requesting" } else { "Get permission" },
            )
            .sm()
            .busy(busy)
            .on_click(crate::settings::click(cx, |s, cx| {
                s.run_command("Speech Recognition", cx, live::request_speech_permission)
            })),
        ),
        "permission_denied" | "permission_restricted" => Some(
            Button::new("speech-settings", "Open System Settings")
                .sm()
                .icon(icons::EXTERNAL)
                .icon_after()
                .on_click(crate::settings::click(cx, |s, cx| {
                    s.run_command("Speech Recognition", cx, live::open_speech_pane)
                })),
        ),
        _ => None,
    };
    row("Model")
        .full()
        .control(div().w_full().child(crate::ui::notice::notice(
            crate::ui::Tone::Warn,
            icons::LOCK,
            format!("Apple Speech \u{b7} {}", speech_label(status)),
            Some(speech_hint(status).into()),
            act.into_iter().map(IntoElement::into_any_element).collect(),
            p,
        )))
        .build(p)
        .into_any_element()
}

/// The Deepgram field's placeholder follows the vault: "A key is saved — …"
/// once one is stored (`Transcription.svelte`).
pub fn sync_key_placeholder(
    s: &mut Settings,
    window: &mut gpui_kit::Window,
    cx: &mut Context<Settings>,
) {
    let saved = s.data.deepgram_key;
    if s.ui.deepgram_placeholder == Some(saved) {
        return;
    }
    s.ui.deepgram_placeholder = Some(saved);
    let text = if saved {
        "A key is saved \u{2014} enter a new one to replace it"
    } else {
        "Paste your Deepgram API key"
    };
    s.inputs
        .deepgram_key
        .update(cx, |st, cx| st.set_placeholder(text, window, cx));
}
