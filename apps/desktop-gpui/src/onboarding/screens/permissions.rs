//! 2 / 8 Permissions (`PermissionsScreen.svelte`): a sequence of moments, one
//! question at a time, Screen Recording first; each answered one shrinks to a
//! ledger line. A denial gates nothing (macOS never re-prompts, so a gate would
//! trap the user); the one hold is the relaunch a same-process screen grant
//! needs. System audio never gets the tick: its grant can't be read (ADR 0052).

use capture_types::CapturePermissionState as P;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, IntoElement, Window};

use super::{back, button, primary};
use crate::onboarding::flow::{is_denied, is_granted, screen_gate};
use crate::onboarding::parts::{self, acts, display, fine, head, mark, prose, Mark};
use crate::onboarding::{Moment, Onboarding};
use crate::theme::{self, space, text, Palette};
use crate::ui::{kicker, notice::notice, Tone};

const EXTERNAL: &str = "icons/external-link.svg";

impl Onboarding {
    fn perm_value(&self, m: Moment) -> P {
        match m {
            Moment::Screen => self.perms.screen,
            Moment::Microphone => self.perms.microphone,
            Moment::SystemAudio => self.perms.system_audio,
            Moment::Accessibility => P::Unknown,
        }
    }

    fn sound_arriving(&self) -> bool {
        self.sys_levels.iter().any(|l| *l > 0.)
    }

    fn ledger_value(&self, m: Moment) -> String {
        let v = self.perm_value(m);
        match m {
            Moment::Accessibility if self.gecko.as_ref().is_some_and(|(t, _)| *t) => {
                "granted".into()
            }
            Moment::Accessibility => "skipped".into(),
            Moment::SystemAudio if self.sound_arriving() => "sound is arriving".into(),
            // Evidence a past session heard, never a live reading.
            Moment::SystemAudio if v == P::AssumedWorking => "sound heard earlier".into(),
            Moment::SystemAudio if self.sys_prompt_raised => "requested \u{b7} unconfirmed".into(),
            Moment::SystemAudio => "not requested".into(),
            Moment::Screen if is_granted(v) && self.screen_needs_relaunch => {
                "granted \u{b7} needs one relaunch".into()
            }
            _ if is_granted(v) => "granted".into(),
            _ if is_denied(v) => "denied".into(),
            _ => "not requested".into(),
        }
    }

    fn ledger(&self, p: &Palette) -> Option<AnyElement> {
        let cur = self.current_moment();
        let settled: Vec<Moment> = self
            .moments()
            .into_iter()
            .filter(|m| Some(*m) != cur && self.answered(*m))
            .collect();
        if settled.is_empty() {
            return None;
        }
        let rows = settled
            .into_iter()
            .map(|m| {
                let v = self.perm_value(m);
                let half = (m == Moment::SystemAudio && self.sound_arriving())
                    || (m == Moment::Screen && self.screen_needs_relaunch);
                let on = match m {
                    Moment::Accessibility => self.gecko.as_ref().is_some_and(|(t, _)| *t),
                    Moment::SystemAudio => false,
                    _ => is_granted(v),
                };
                let warn = (m != Moment::Accessibility && is_denied(v))
                    || (m == Moment::SystemAudio && v == P::PossiblyBlocked);
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .h(px(36.))
                    .child(mark(
                        if half {
                            Mark::Half
                        } else if on {
                            Mark::On
                        } else {
                            Mark::Off
                        },
                        p,
                    ))
                    .child(
                        div()
                            .w(px(160.))
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MD)
                            .text_color(p.text)
                            .child(m.name()),
                    )
                    .child(parts::mono(
                        self.ledger_value(m),
                        text::BASE,
                        if warn { p.warn } else { p.text_subtle },
                    ))
                    .into_any_element()
            })
            .collect();
        Some(parts::ruled(Some(px(520.)), rows, p).into_any_element())
    }

    fn fix_row(
        &self,
        m: Moment,
        where_: &'static str,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let kind = m.wire();
        acts(vec![
            button(
                "ob-open-settings",
                "Open System Settings",
                cx,
                move |this, _, cx| this.open_helper(kind, cx),
            )
            .icon(EXTERNAL)
            .icon_after()
            .into_any_element(),
            button("ob-recheck", "Re-check", cx, |this, _, cx| this.recheck(cx)).into_any_element(),
            fine(
                format!("Privacy & Security \u{2192} {where_} \u{2192} Mnema"),
                p,
            )
            .ml(space::S2)
            .into_any_element(),
        ])
        .into_any_element()
    }

    fn request_row(
        &self,
        m: Moment,
        label: &'static str,
        hint: &'static str,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let busy = self.requesting == Some(m);
        acts(vec![
            button(
                "ob-request",
                if busy { "Requesting\u{2026}" } else { label },
                cx,
                move |this, _, cx| this.request(m, cx),
            )
            .disabled(self.requesting.is_some())
            .into_any_element(),
            fine(hint, p).ml(space::S2).into_any_element(),
        ])
        .into_any_element()
    }

    fn moment(&self, cur: Option<Moment>, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        let el = |e: gpui_kit::Div| e.into_any_element();
        match cur {
            Some(Moment::Screen) if is_denied(self.perms.screen) => vec![
                el(display("Screen recording", p)),
                el(prose("You said no \u{2014} and this is the one Mnema cannot run without. macOS will not ask again, so the switch has to be flipped in System Settings.", p)),
                self.fix_row(Moment::Screen, "Screen Recording", cx, p),
                el(fine("Flip it, come back, press Re-check \u{2014} no need to quit Mnema yourself.", p)),
            ],
            Some(Moment::Screen) => vec![
                el(display("Your screen.", p)),
                el(prose("The one Mnema is built on \u{2014} it records the screen so you can scrub back to it.", p)),
                self.request_row(Moment::Screen, "Request access", "macOS raises its own prompt. Nothing is recorded until you finish setup.", cx, p),
            ],
            Some(Moment::Microphone) if is_denied(self.perms.microphone) => vec![
                el(display("Microphone", p)),
                el(prose("You said no. Mnema keeps working \u{2014} without transcripts or speaker names.", p)),
                self.fix_row(Moment::Microphone, "Microphone", cx, p),
                el(fine("macOS never asks again on its own, which is why this screen never blocks.", p)),
            ],
            Some(Moment::Microphone) => vec![
                el(display("Your microphone.", p)),
                el(prose("Only what you say, transcribed on this Mac \u{2014} no audio ever leaves it.", p)),
                self.request_row(Moment::Microphone, "Request access", "Off by itself, this only costs you transcripts and speaker names.", cx, p),
            ],
            Some(Moment::SystemAudio) => {
                let sound = self.sound_arriving();
                let state = if sound {
                    "Sound is arriving."
                } else if self.probing_sys {
                    "Listening\u{2026}"
                } else if self.sys_prompt_raised {
                    "Requested \u{b7} unconfirmed"
                } else {
                    "Nothing has arrived yet"
                };
                let color = if sound {
                    p.source_sysaudio
                } else if self.probing_sys {
                    p.chart_grey_4
                } else {
                    p.chart_grey_2
                };
                let level = div()
                    .self_start()
                    .flex()
                    .items_center()
                    .gap(space::S5)
                    .px(space::S4)
                    .py(space::S3)
                    .rounded(theme::radius::LG)
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.border)
                    .child(parts::meter(&self.sys_levels, px(4.), 30., color, true))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .font_weight(theme::W_SEMIBOLD)
                                    .text_size(text::MD)
                                    .text_color(p.text_strong)
                                    .child(state),
                            )
                            .when(!sound, |d| {
                                d.child(fine(
                                    format!(
                                        "{}Silent is not denied \u{2014} it might just be a quiet Mac.",
                                        if self.probing_sys { "Play something \u{2014} a video, a song. " } else { "" }
                                    ),
                                    p,
                                ))
                            }),
                    );
                let mut v = vec![
                    el(display("Play anything.", p)),
                    el(prose("macOS will not tell us whether you granted this \u{2014} sound arriving is the only proof.", p)),
                    el(level),
                ];
                if self.perms.system_audio == P::PossiblyBlocked {
                    v.push(el(notice(
                        Tone::Info,
                        "icons/info.svg",
                        "System audio may be blocked",
                        Some("A minute of silence while something played. Check the switch, or carry on \u{2014} the mic still records.".into()),
                        vec![button("ob-sys-settings", "Open System Settings", cx, |this, _, cx| this.open_helper("systemAudio", cx))
                            .sm()
                            .icon(EXTERNAL)
                            .icon_after()
                            .into_any_element()],
                        p,
                    )));
                }
                v.push(self.request_row(
                    Moment::SystemAudio,
                    if self.sys_prompt_raised { "Request again" } else { "Request" },
                    "A prompt you closed by accident looks exactly like a No.",
                    cx,
                    p,
                ));
                v
            }
            Some(Moment::Accessibility) => {
                let names = self.gecko.as_ref().map(|(_, n)| n.join(" and ")).filter(|n| !n.is_empty());
                vec![
                    el(display("Browser page addresses.", p)),
                    el(prose(
                        format!(
                            "Optional, and only for {} \u{2014} every other browser tells Mnema its page address already.",
                            names.unwrap_or_else(|| "Firefox and Zen".into())
                        ),
                        p,
                    )),
                    if self.acc_asked {
                        self.fix_row(Moment::Accessibility, "Accessibility", cx, p)
                    } else {
                        self.request_row(Moment::Accessibility, "Request access", "Privacy & Security \u{2192} Accessibility \u{2192} Mnema", cx, p)
                    },
                    el(fine("The broadest grant there is, so it never rides along with the others.", p)),
                ]
            }
            None => vec![
                el(display("That is everything.", p)),
                el(prose("Nothing left to ask \u{2014} all of it stays changeable in Settings.", p)),
            ],
        }
    }

    pub(in crate::onboarding) fn permissions(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        self.sync_sys_poll(cx);
        let p = theme::tokens(cx);
        let cur = self.current_moment();
        let upcoming = self.upcoming();
        let mut col: Vec<AnyElement> = vec![kicker("One at a time", &p).into_any_element()];
        if self.screen_needs_relaunch {
            col.push(
                notice(
                    Tone::Info,
                    "icons/rotate-ccw.svg",
                    "Screen recording needs one relaunch",
                    Some("macOS gives that grant to a fresh copy of Mnema. Setup resumes right here.".into()),
                    vec![],
                    &p,
                )
                .max_w(px(640.))
                .into_any_element(),
            );
        }
        col.extend(self.ledger(&p));
        col.push(
            head(self.moment(cur, cx, &p))
                .gap(space::S4)
                .into_any_element(),
        );

        let (ready, gate_label) = screen_gate(self.perms.screen, self.screen_needs_relaunch);
        let is_last = cur.is_none() || upcoming.is_empty();
        let label = if !ready {
            gate_label
        } else if is_last {
            "Capture & Storage"
        } else if cur == Some(Moment::Screen) {
            gate_label
        } else if cur == Some(Moment::Microphone) && is_denied(self.perms.microphone) {
            "Continue without the microphone"
        } else {
            "Continue"
        };
        let note = div()
            .flex()
            .gap(space::S2)
            .children(upcoming.iter().enumerate().map(|(i, m)| {
                div()
                    .flex()
                    .gap(px(4.))
                    .when(i > 0, |d| d.ml(space::S3))
                    .child(if i == 0 { "next \u{b7}" } else { "then \u{b7}" })
                    .child(
                        div()
                            .font_weight(theme::W_MEDIUM)
                            .text_color(p.text)
                            .child(m.name()),
                    )
                    .when(*m == Moment::Accessibility, |d| d.child("(optional)"))
            }));
        let foot = parts::footer(
            Some(note.into_any_element()),
            vec![
                back(cx),
                primary("ob-next", label, cx, |this, _, cx| {
                    this.advance_permissions(cx)
                })
                .into_any_element(),
            ],
            &p,
        );
        (
            parts::main_col("ob-perm", false, false, col),
            Some(foot.into_any_element()),
        )
    }
}
