//! Intelligence › Speakers › Your voice (`VoiceEnrollment.svelte`): the
//! voiceprint read-out, the sentence to read, Record, and auto-labelling.

use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context};

use crate::settings::row::{hint, row, section};
use crate::settings::{on, Settings};
use crate::theme::{radius, space, text, Palette};
use crate::ui::switch::Switch;
use crate::ui::Button;

/// `ENROLLMENT_SENTENCE`.
pub const SENTENCE: &str = "I keep my work on this machine, and I would like Mnema to know my voice. I work on a few things at once, and I would like the transcript to say who said what. Reading this out loud takes about fifteen seconds.";
/// `ENROLLMENT_CLIP_MS` in seconds.
const CLIP_SECONDS: u32 = 15;

/// `recognitionReadout`.
pub fn readout(owner: Option<&str>, separate: bool, recognize: bool, auto_label: bool) -> String {
    let Some(name) = owner else {
        return "No voiceprint saved. Your turns stay labelled \u{201c}Speaker 1\u{201d} until you record one.".into();
    };
    let who = if name.trim().is_empty() {
        "you"
    } else {
        name.trim()
    };
    if !separate {
        format!(
            "Voiceprint saved for {who}. Speaker separation is off, so nothing is being labelled."
        )
    } else if !recognize {
        format!("Voiceprint saved for {who}. Recognising saved people is off, so the voiceprint is not being used.")
    } else if auto_label {
        format!("Voiceprint saved for {who}. Your turns are labelled with your name on their own when the match is confident.")
    } else {
        format!(
            "Voiceprint saved for {who}. Mnema suggests your name and waits for you to confirm it."
        )
    }
}

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let a = &d.speaker_analysis;
    let owner = s.data.voice_owner.as_deref();
    let record = if owner.is_some() {
        format!("Record again ({CLIP_SECONDS}s)")
    } else {
        format!("Record {CLIP_SECONDS} seconds")
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .w_full()
        .child(hint(readout(owner, a.separate_speakers, a.recognize_saved_people, a.auto_label_owner), p))
        .child(
            div()
                .p(space::S3)
                .rounded(radius::MD)
                .border_1()
                .border_color(p.hairline)
                .bg(p.surface_subtle)
                .text_size(text::MD)
                .line_height(gpui_kit::relative(1.6))
                .text_color(p.text)
                .child(SENTENCE),
        )
        // shortcut: no live level meter or countdown here (onboarding's Voice step has
        // them); the button reads "Recording…" for the take, then the verdict shows above.
        .child({
            let recording = s.ui.busy.contains("Voiceprint");
            let owner_id = s.data.voice_owner_id;
            div()
                .flex()
                .gap(space::S2)
                .child(
                    Button::new("voice-record", if recording { "Recording\u{2026} read the sentence aloud".to_string() } else { record })
                        .disabled(recording)
                        .on_click(crate::settings::click(cx, |s, cx| {
                            s.run_command("Voiceprint", cx, |core| {
                                crate::settings::live::enroll_voice(core, CLIP_SECONDS as u64 * 1000)
                            })
                        })),
                )
                .when_some(owner_id, |d, id| {
                    d.child(Button::new("voice-delete", "Delete voiceprint").danger().on_click(
                        crate::settings::click(cx, move |_, cx| {
                            crate::settings::confirm(
                                cx,
                                "Delete your voiceprint?",
                                "Mnema stops labelling your voice. Turns already labelled keep their name.",
                                "Delete",
                                move |s, cx| {
                                    s.run_command("Voiceprint", cx, move |core| {
                                        crate::settings::live::delete_person(core, id)
                                    })
                                },
                            );
                        }),
                    ))
                })
        })
        .child(hint("The voiceprint never leaves this device. Recognition is imperfect and will not label every turn.", p));
    let rows = vec![
        row("Voiceprint").full().control(body).build(p).into_any_element(),
        row("Label my voice automatically")
            .desc("When Mnema is confident a voice is yours, it applies your name without asking. Turn this off and it suggests your name and waits for you to confirm. Automatic labels are marked as automatic and can be undone. Only your own voice is ever labelled this way.")
            .control(
                Switch::new("voice-auto", "Label my voice automatically", a.auto_label_owner)
                    .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.speaker_analysis.auto_label_owner = v, cx))),
            )
            .build(p)
            .into_any_element(),
    ];
    section(
        "Your voice",
        Some("Record a short sample so Mnema can label your turns with your name instead of \u{201c}Speaker 1\u{201d}."),
        Vec::new(),
        false,
        rows,
        p,
    )
    .into_any_element()
}
