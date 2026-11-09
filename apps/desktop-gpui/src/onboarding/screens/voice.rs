//! 7 / 8 Voice (`VoiceScreen.svelte`): speakrs ready → a 15 s take of a
//! supplied sentence with a live level meter and the embedder's typed
//! rejections; not ready → say so, with "Set this up later" as the primary.
//! Never a gate: skipping is a first-class outcome.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Window};

use super::{back, button, primary};
use crate::onboarding::downloads::DlState;
use crate::onboarding::flow::Subsystem;
use crate::onboarding::parts::{self, display, fine, head, prose};
use crate::onboarding::voice::{self, Phase};
use crate::onboarding::Onboarding;
use crate::settings::model::format_bytes;
use crate::theme::{self, radius, space, text};
use crate::ui::notice::notice;
use crate::ui::{kicker, Tone};

/// `.ob-split`: the instrument column to the right of a 300 px head.
const HEAD_W: f32 = 300.;

impl Onboarding {
    pub(in crate::onboarding) fn voice_step(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let ready = self.voice_ready();
        let (left, right): (AnyElement, AnyElement) = if ready {
            let t = &self.take;
            let busy = t.busy();
            let callout = match &t.phase {
                Phase::Enrolled => Some(notice(
                    Tone::Ok,
                    "icons/check.svg",
                    "Enrolled.",
                    Some("Mnema will pick your voice out from now on.".into()),
                    vec![],
                    &p,
                )),
                Phase::Failed(e) => Some(notice(
                    Tone::Danger,
                    "icons/triangle-alert.svg",
                    "That didn\u{2019}t record.",
                    Some(e.clone().into()),
                    vec![],
                    &p,
                )),
                Phase::Rejected(r) => {
                    let (title, body) = voice::rejection(r);
                    Some(notice(
                        Tone::Warn,
                        "icons/triangle-alert.svg",
                        title,
                        Some(body.into()),
                        vec![],
                        &p,
                    ))
                }
                _ => None,
            };
            let rec_label = match t.phase {
                Phase::Recording { .. } => "Recording\u{2026}",
                Phase::Judging => "Checking\u{2026}",
                _ if t.take == 0 => "Record fifteen seconds",
                _ => "Record again",
            };
            let rec = if t.phase == Phase::Enrolled {
                button("ob-record", "Record again", cx, |this, _, cx| {
                    this.record(cx)
                })
            } else {
                button("ob-record", rec_label, cx, |this, _, cx| this.record(cx))
                    .primary()
                    .icon("icons/mic.svg")
                    .disabled(busy)
            };
            let meter_color = if t.take > 0 || busy {
                p.source_mic
            } else {
                p.chart_grey_2
            };
            let panel = div()
                .flex()
                .flex_col()
                .gap(space::S4)
                .p(space::S5)
                .rounded(radius::LG)
                .bg(p.surface)
                .border_1()
                .border_color(p.border)
                .child(parts::label("Read this out loud", &p))
                .child(
                    div()
                        .max_w(px(50. * 8.5))
                        .text_size(px(17.))
                        .line_height(relative(1.55))
                        .text_color(p.text_strong)
                        .child(format!("\u{201c}{}\u{201d}", voice::SENTENCE)),
                )
                .child(
                    div()
                        .id("ob-take")
                        .role(gpui_kit::Role::Status)
                        .flex()
                        .items_center()
                        .gap(space::S4)
                        .child(parts::meter(&t.levels, px(3.), 36., meter_color, false))
                        .child(parts::mono(t.line(), text::BASE, p.text_muted)),
                )
                .children(callout)
                .child(parts::acts(vec![rec.into_any_element()]));
            (
                head(vec![
                    kicker("Voice", &p).into_any_element(),
                    display("Fifteen seconds so Mnema can use your name.", &p)
                        .text_size(px(34.))
                        .into_any_element(),
                    fine(
                        "Otherwise your side is filed as \u{201c}Speaker\u{a0}1\u{201d}.",
                        &p,
                    )
                    .into_any_element(),
                ])
                .into_any_element(),
                panel.into_any_element(),
            )
        } else {
            let item = self
                .work_list()
                .into_iter()
                .find(|i| i.subsystem == Subsystem::Speaker);
            let state = item.as_ref().map(|i| self.dl.state(i));
            let reason = if !self.features.speaker_separation {
                "Who\u{2019}s speaking is switched off, so there is nothing to match a voice against."
            } else if state == Some(DlState::Downloading) {
                "The model that tells voices apart is still arriving."
            } else if state == Some(DlState::Failed) {
                "The model that tells voices apart didn\u{2019}t finish downloading."
            } else {
                "The model that tells voices apart isn\u{2019}t here yet."
            };
            let mut right = vec![prose(reason, &p).into_any_element()];
            if let (Some(i), Some(DlState::Downloading)) = (&item, state) {
                let got = self.dl.received(i);
                right.push(
                    parts::track(
                        got as f32 / i.bytes.max(1) as f32,
                        false,
                        Some(px(420.)),
                        &p,
                    )
                    .into_any_element(),
                );
                right.push(
                    fine(
                        format!(
                            "{} of {}",
                            format_bytes(got as f64),
                            format_bytes(i.bytes as f64)
                        ),
                        &p,
                    )
                    .into_any_element(),
                );
                right.push(
                    fine(
                        "A card will be waiting on your dashboard the moment it is ready.",
                        &p,
                    )
                    .into_any_element(),
                );
            }
            (
                head(vec![
                    display("Not yet \u{2014} and that is fine.", &p)
                        .text_size(px(34.))
                        .into_any_element(),
                    fine("This step waits for you, never the other way round.", &p)
                        .into_any_element(),
                ])
                .into_any_element(),
                head(right).into_any_element(),
            )
        };
        let split = div()
            .flex()
            .items_center()
            .gap(space::S7)
            .child(div().flex_none().w(px(HEAD_W)).child(left))
            .child(div().flex_1().min_w_0().child(right));
        let mut acts = vec![back(cx)];
        if ready {
            if self.take.phase == Phase::Enrolled {
                acts.push(
                    primary("ob-next", "Continue", cx, |this, _, cx| this.next(cx))
                        .into_any_element(),
                );
            } else {
                acts.push(
                    button(
                        "ob-skip",
                        "Skip \u{2014} you can do this later in Settings",
                        cx,
                        |this, _, cx| this.next(cx),
                    )
                    .into_any_element(),
                );
            }
        } else {
            acts.push(
                button("ob-skip", "Skip voice entirely", cx, |this, _, cx| {
                    this.next(cx)
                })
                .ghost()
                .into_any_element(),
            );
            acts.push(
                primary("ob-next", "Set this up later", cx, |this, _, cx| {
                    this.next(cx)
                })
                .into_any_element(),
            );
        }
        let foot = parts::footer(
            Some("The voiceprint never leaves this device, and recognition labels many turns \u{2014} not every turn.".into_any_element()),
            acts,
            &p,
        );
        (
            parts::main_col("ob-voice", true, false, vec![split.into_any_element()]),
            Some(foot.into_any_element()),
        )
    }
}
