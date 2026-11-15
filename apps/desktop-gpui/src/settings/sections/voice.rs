//! Intelligence › Speakers › Your voice (`VoiceEnrollment.svelte`): the
//! voiceprint read-out, the sentence to read, a 15 s take with the level meter
//! and countdown (`voice_take.rs`), review, and auto-labelling.

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Window};

use super::voice_take::{self, VoiceTake};
use crate::settings::row::{hint, line, row, section};
use crate::settings::{icons, on, Settings};
use crate::theme::{space, text, Palette};
use crate::ui::switch::Switch;
use crate::ui::{Button, Tone};
use crate::voice::{self, Phase};

/// `ENROLLMENT_CLIP_MS` in seconds.
const CLIP_SECONDS: u64 = voice::CLIP.as_secs();

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

/// `take.update(..)` for a button's `on_click`.
fn act(
    t: &gpui_kit::Entity<VoiceTake>,
    f: fn(&mut VoiceTake, &mut Context<VoiceTake>),
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static {
    let t = t.clone();
    move |_, _, cx| t.update(cx, f)
}

pub fn render(
    s: &Settings,
    p: &Palette,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let d = s.d();
    let a = &d.speaker_analysis;
    let owner = s.data.voice_owner.as_deref();
    let take = voice_take::state(s, window, cx);
    let v = take.read(cx);
    let t = &v.take;
    let readout = readout(
        owner,
        a.separate_speakers,
        a.recognize_saved_people,
        a.auto_label_owner,
    );
    let mut actions: Vec<AnyElement> = Vec::new();
    if let Phase::Review(_) = t.phase {
        actions.push(
            Button::new("voice-use", "Use this take")
                .on_click(act(&take, VoiceTake::use_take))
                .into_any_element(),
        );
        actions.push(
            Button::new("voice-discard", "Discard")
                .ghost()
                .on_click(act(&take, VoiceTake::discard))
                .into_any_element(),
        );
    } else {
        let label = match t.phase {
            Phase::Recording { .. } => "Recording".to_string(),
            Phase::Judging => "Checking the take".to_string(),
            _ if owner.is_some() => format!("Record again ({CLIP_SECONDS}s)"),
            _ => format!("Record {CLIP_SECONDS} seconds"),
        };
        actions.push(
            Button::new("voice-record", label)
                .spinner(t.busy())
                .disabled(t.busy())
                .on_click(act(&take, VoiceTake::start))
                .into_any_element(),
        );
        if matches!(t.phase, Phase::Recording { .. }) {
            actions.push(
                Button::new("voice-cancel", "Cancel")
                    .ghost()
                    .on_click(act(&take, VoiceTake::cancel))
                    .into_any_element(),
            );
        }
        if let Some(id) = s.data.voice_owner_id {
            let name = owner.unwrap_or("you").to_string();
            let deleting = s.ui.busy.contains("Voiceprint");
            actions.push(
                Button::new("voice-delete", if deleting { "Deleting" } else { "Delete voiceprint" })
                    .danger()
                    .spinner(deleting)
                    .disabled(t.busy() || deleting)
                    .on_click(crate::settings::click(cx, move |_, cx| {
                        crate::settings::commands::confirm_as(
                            cx,
                            "Delete voiceprint",
                            &format!("Delete the voiceprint saved for {name}? Mnema stops recognising your voice and labels your turns \u{201c}Speaker 1\u{201d} again. Your recordings and transcripts are not touched."),
                            "Delete",
                            "Keep",
                            move |s, cx| {
                                s.run_command("Voiceprint", cx, move |core| {
                                    crate::settings::live::delete_person(core, id)
                                })
                            },
                        );
                    }))
                    .into_any_element(),
            );
        }
    }
    let status: Option<AnyElement> = match &t.phase {
        Phase::Recording { .. } => Some(
            div()
                .id("voice-meter")
                .role(gpui_kit::Role::Status)
                .aria_label(format!("Recording, {}s left", t.seconds_left()))
                .flex()
                .items_center()
                .gap(space::S3)
                .child(voice::meter(&t.levels, px(3.), 24., p.source_mic, false))
                .child(
                    div()
                        .text_size(text::BASE)
                        .text_color(p.text_muted)
                        .child(format!("Recording \u{b7} {}s left", t.seconds_left())),
                )
                .test_support()
                .into_any_element(),
        ),
        Phase::Review(_) => Some(
            div()
                .flex()
                .items_center()
                .gap(space::S3)
                .child(
                    Button::new("voice-play", if v.playing { "Stop" } else { "Play the take" })
                        .disabled(v.play_failed)
                        .on_click(act(&take, VoiceTake::toggle_playback)),
                )
                .child(hint(
                    if v.play_failed {
                        "Playback is unavailable, but the take was recorded \u{2014} use it or record again."
                    } else {
                        "Listen back before you save it."
                    },
                    p,
                )
                // Wraps beside the button instead of running past the row.
                .min_w_0())
                .into_any_element(),
        ),
        Phase::Rejected(r) => Some(line(Tone::Warn, icons::ALERT, voice::rejection(r).1, p).into_any_element()),
        Phase::Failed(e) => Some(line(Tone::Danger, icons::ALERT, e.clone(), p).into_any_element()),
        _ => None,
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .w_full()
        // `voice.loading`: the owner arrives with the slow reads.
        .child(hint(
            if s.data.voice_checked || s.data.complete {
                readout
            } else {
                "Checking for a saved voiceprint\u{2026}".to_string()
            },
            p,
        ))
        .when(v.just_enrolled, |b| {
            b.child(hint("Saved. Recognising saved people was switched on for you.", p))
        })
        .child(
            // `.voice-sentence`: a quotation rule, no box.
            div()
                .px(px(12.))
                .py(px(10.))
                .border_l_2()
                .border_color(p.border)
                .text_size(text::MD)
                .line_height(gpui_kit::relative(1.6))
                .text_color(p.text)
                .child(voice::SENTENCE),
        )
        .children(status)
        .child(div().flex().gap(space::S2).children(actions))
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
