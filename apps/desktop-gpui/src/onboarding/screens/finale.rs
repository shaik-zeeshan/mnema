//! 8 / 8 Finale (`FinaleScreen.svelte`): capture starts HERE, on arrival, and
//! the screen ends on evidence: the real first frame off the pipeline, its
//! timestamp, and the first words OCR read off it. Four states: starting,
//! running (or audio-only), idle (nothing granted), failed (the real reason).

use capture_types::CapturePermissionState as P;
use gpui_kit::prelude::*;
use gpui_kit::{div, img, px, relative, AnyElement, Context, ObjectFit};

use super::{button, primary};
use crate::onboarding::parts::{self, fine, head};
use crate::onboarding::{FinalePhase, Onboarding};
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::notice::notice;
use crate::ui::Tracking as _;
use crate::ui::{alpha, progress, Tone};

const HEAD_W: f32 = 360.;
const EXTERNAL: &str = "icons/external-link.svg";

fn pill(
    label: String,
    fg: gpui_kit::Hsla,
    border: gpui_kit::Hsla,
    dot: Option<(gpui_kit::Hsla, gpui_kit::Hsla)>,
    spin: bool,
) -> AnyElement {
    div()
        .self_start()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(size::REC_PILL_H)
        .pl(px(7.))
        .pr(px(9.))
        .rounded_full()
        .border_1()
        .border_color(border)
        .font_family(FONT_MONO)
        .font_weight(theme::W_MEDIUM)
        .text_size(text::SM)
        .tracking(0.04)
        .text_color(fg)
        .when(spin, |d| d.child(progress::spinner(size::SPIN_SM)))
        .when_some(dot, |d, (c, glow)| {
            d.child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(c)
                    .shadow(vec![crate::ui::spread(glow, px(3.))]),
            )
        })
        .child(label)
        .into_any_element()
}

fn title(lines: &[&str], p: &Palette) -> AnyElement {
    div()
        .mt(space::S4)
        .flex()
        .flex_col()
        .font_weight(theme::W_SEMIBOLD)
        .text_size(parts::FINALE_TITLE)
        .tracking(-0.042)
        .line_height(relative(0.98))
        .text_color(p.text_strong)
        .children(lines.iter().map(|l| div().child(l.to_string())))
        .into_any_element()
}

impl Onboarding {
    pub(in crate::onboarding) fn finale_step(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let req = self.start_request();
        let screen_granted = self.perms.screen == P::Granted;
        let running = self.finale == FinalePhase::Running;
        let audio_only = running && !req.capture_screen;
        let audio_name = match (req.capture_microphone, req.capture_system_audio) {
            (true, true) => "Audio",
            (true, false) => "Microphone",
            _ => "System audio",
        };
        let screen_off = if screen_granted {
            "Screen capture is turned off"
        } else {
            "Screen Recording is off"
        };
        let now = crate::ui::clock::now(cx);
        if running && self.rec_tick.is_none() {
            self.rec_tick = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }));
        }
        let elapsed = self
            .rec_started
            .map_or(0, |t| (now - t).whole_seconds().max(0));
        let rec = format!(
            "REC {:02}:{:02}:{:02}",
            elapsed / 3600,
            elapsed / 60 % 60,
            elapsed % 60
        );
        let left: Vec<AnyElement> = match &self.finale {
            FinalePhase::Running => vec![
                pill(
                    rec,
                    p.status_running_fg,
                    p.status_running_border,
                    Some((p.status_running_dot, p.status_running_dot_glow)),
                    false,
                ),
                title(&["It has", "already", "started."], &p),
                fine(
                    if audio_only {
                        format!(
                            "{audio_name} only \u{2014} {screen_off}, so there are no frames yet."
                        )
                    } else {
                        "Capture began the moment you arrived here.".into()
                    },
                    &p,
                )
                .into_any_element(),
            ],
            FinalePhase::Idle => vec![
                pill(
                    "NOT RECORDING".into(),
                    p.text_muted,
                    p.border,
                    Some((p.status_dot, alpha(p.status_dot, 0.))),
                    false,
                ),
                title(&["Nothing is", "being", "recorded."], &p),
                fine(
                    if screen_granted {
                        "Every capture source is turned off, so Mnema is idle."
                    } else {
                        "You declined Screen Recording, so Mnema is idle."
                    },
                    &p,
                )
                .into_any_element(),
            ],
            FinalePhase::Failed(why) => {
                let mut v = vec![
                    pill(
                        "NOT RECORDING".into(),
                        p.danger_text,
                        p.danger_border,
                        Some((p.danger, alpha(p.danger, 0.))),
                        false,
                    ),
                    title(&["Capture could", "not start."], &p),
                    parts::mono(why.clone(), text::MD, p.danger_text)
                        .line_height(relative(1.45))
                        .into_any_element(),
                ];
                if screen_granted && self.completed {
                    v.push(fine("Screen Recording is granted, so this is almost certainly the stale-stream case: macOS hands the capture stream out at launch, and Mnema has not been relaunched since the grant.", &p).into_any_element());
                }
                v
            }
            FinalePhase::Starting => vec![
                pill(
                    "STARTING\u{2026}".into(),
                    p.text_muted,
                    p.border_strong,
                    None,
                    true,
                ),
                title(&["Setting up."], &p),
                fine("Saving your settings, then starting capture.", &p).into_any_element(),
            ],
        };
        let interval = self.interval_s();
        let ev = self.evidence.clone();
        let label = if audio_only {
            format!("Listening \u{2014} {} only", audio_name.to_lowercase())
        } else if running {
            "Your first frame".into()
        } else {
            "What you would be seeing".into()
        };
        let empty = |s: String| {
            div()
                .w_full()
                .h(px(300.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::LG)
                .border_1()
                .border_dashed()
                .border_color(p.border)
                .child(fine(s, &p))
                .into_any_element()
        };
        let shot: AnyElement = match ev.as_ref().and_then(|e| e.image.clone()) {
            Some(path) if !audio_only => div()
                .w_full()
                .h(px(300.))
                .overflow_hidden()
                .rounded(radius::LG)
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                // GPUI clips to a rectangle: the picture rounds itself inside the border.
                .child(
                    img(path)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded(radius::LG - px(1.)),
                )
                .into_any_element(),
            _ if audio_only => empty(format!("No screen frames \u{2014} {screen_off}.")),
            _ if running && ev.is_some() => {
                empty("Frame captured \u{2014} its image is still being written.".into())
            }
            _ if running || self.finale == FinalePhase::Starting => empty(format!(
                "first frame due within {} {}",
                if interval.fract() == 0. {
                    format!("{}", interval as i64)
                } else {
                    format!("{interval}")
                },
                if interval == 1. { "second" } else { "seconds" }
            )),
            _ => empty("No frame has been captured.".into()),
        };
        let mut evidence =
            vec![head(vec![parts::label(label, &p).into_any_element(), shot]).into_any_element()];
        if let Some(e) = ev.as_ref().filter(|_| !audio_only) {
            let at = time::OffsetDateTime::from_unix_timestamp_nanos(
                e.captured_at_ms as i128 * 1_000_000,
            )
            .map(|t| t.to_offset(now.offset()))
            .unwrap_or(now);
            let ago = (now - at).whole_seconds().max(0);
            evidence.push(
                parts::mono(
                    format!(
                        "Captured {} \u{b7} {ago} {} ago",
                        crate::timeline::model::clock_label(at, true),
                        if ago == 1 { "second" } else { "seconds" }
                    ),
                    text::BASE,
                    p.text_subtle,
                )
                .into_any_element(),
            );
            if let Some(ocr) = e
                .ocr
                .as_deref()
                .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
            {
                let line: String = ocr.chars().take(64).collect();
                evidence.push(
                    head(vec![
                        parts::label("First words read off it \u{b7} already searchable", &p)
                            .into_any_element(),
                        div()
                            .px(space::S3)
                            .py(space::S3)
                            .rounded(radius::MD)
                            .bg(p.surface)
                            .border_1()
                            .border_color(p.border)
                            .font_family(FONT_MONO)
                            .text_size(text::MD)
                            .line_height(relative(1.5))
                            .text_color(p.text)
                            .child(div().bg(p.accent_bg).child(line))
                            .into_any_element(),
                    ])
                    .into_any_element(),
                );
            }
        } else if self.finale == FinalePhase::Idle {
            evidence.push(fine("Privacy & Security \u{2192} Screen Recording \u{2192} Mnema, then relaunch once.", &p).into_any_element());
        } else if audio_only && !screen_granted {
            evidence.push(
                notice(
                    Tone::Warn,
                    "icons/lock.svg",
                    "Screen Recording is off for Mnema",
                    Some("Turn Screen Recording on in System Settings, then relaunch once \u{2014} frames start on their own.".into()),
                    vec![button("ob-finale-settings", "Open System Settings", cx, |this, _, cx| this.open_helper("screen", cx))
                        .sm()
                        .icon(EXTERNAL)
                        .icon_after()
                        .into_any_element()],
                    &p,
                )
                .into_any_element(),
            );
        }
        let split = div()
            .flex()
            .items_center()
            .gap(space::S7)
            .child(div().flex_none().w(px(HEAD_W)).child(head(left)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(space::S5)
                    .children(evidence),
            );

        let pending: Vec<String> = self
            .work_list()
            .into_iter()
            .filter(|i| {
                self.features.get(i.feature)
                    && self.dl.state(i) != crate::onboarding::downloads::DlState::Ready
            })
            .map(|i| {
                format!(
                    "{} {}%",
                    i.name(),
                    ((self.dl.received(&i) as f64 / i.bytes.max(1) as f64) * 100.).floor()
                )
            })
            .collect();
        let note = match &self.finale {
            FinalePhase::Idle => "Grant it later and capture starts on its own.".to_string(),
            FinalePhase::Failed(_) => String::new(),
            _ if !pending.is_empty() => format!(
                "Preparing \u{b7} {} \u{2014} {}.",
                pending.join(" \u{b7} "),
                if pending.len() > 1 {
                    "they turn themselves on"
                } else {
                    "turns itself on"
                }
            ),
            FinalePhase::Running => "Everything else runs in the background from here.".into(),
            FinalePhase::Starting => String::new(),
        };
        let acts: Vec<AnyElement> = match &self.finale {
            FinalePhase::Idle => vec![
                super::back(cx),
                button("ob-open-anyway", "Open Mnema anyway", cx, |this, w, cx| {
                    this.open_mnema(w, cx)
                })
                .into_any_element(),
                button("ob-recheck", "Re-check", cx, |this, _, cx| {
                    this.recheck_finale(cx)
                })
                .into_any_element(),
                button(
                    "ob-open-settings",
                    "Open System Settings",
                    cx,
                    |this, _, cx| this.open_helper("screen", cx),
                )
                .primary()
                .icon(EXTERNAL)
                .icon_after()
                .into_any_element(),
            ],
            FinalePhase::Failed(_) => vec![
                super::back(cx),
                button("ob-open-anyway", "Open Mnema anyway", cx, |this, w, cx| {
                    this.open_mnema(w, cx)
                })
                .into_any_element(),
                button("ob-retry", "Try again", cx, |this, _, cx| this.commit(cx))
                    .into_any_element(),
                button("ob-relaunch", "Relaunch Mnema", cx, |this, _, cx| {
                    this.relaunch(cx)
                })
                .primary()
                .into_any_element(),
            ],
            phase => vec![primary("ob-open", "Open Mnema", cx, |this, w, cx| {
                this.open_mnema(w, cx)
            })
            .disabled(*phase == FinalePhase::Starting)
            .into_any_element()],
        };
        let foot = parts::footer_text(note, acts, self.inset, &p);
        (
            parts::main_col(
                "ob-finale",
                true,
                false,
                self.inset,
                vec![split.into_any_element()],
            ),
            Some(foot.into_any_element()),
        )
    }
}
