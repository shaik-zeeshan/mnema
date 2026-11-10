//! 3 / 8 Capture & Storage (`CaptureStorageScreen.svelte` + `CaptureSentence`):
//! THE ONLY SCREEN WITH HARD GATES. Four spec rows and the verdict panel; the
//! panel prints every storage failure and its repairs, and Continue is held
//! (with a "continue held" chip) while `capture_block` names one.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, FontWeight, Hsla, Window};

use super::{back, button, on_ix, primary};
use crate::onboarding::parts::{self, display, fine, head};
use crate::onboarding::storage::{self, Seg, Tone as VTone};
use crate::onboarding::Onboarding;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::segmented::Segmented;
use crate::ui::{kicker, mix, Tone};

const LABEL_W: f32 = 136.;
const APP_COLORS: usize = 4;

fn row(label: &'static str, ctl: impl IntoElement, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(space::S4)
        .min_h(px(52.))
        .py(px(9.))
        .child(
            div()
                .flex_none()
                .w(px(LABEL_W))
                .font_weight(theme::W_MEDIUM)
                .text_size(text::MD)
                .text_color(p.text_muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(ctl),
        )
        .into_any_element()
}

/// The verdict's runs: figures and the verdict clause in strong.
fn runs(segs: &[(Seg, String)], verdict_color: Hsla, p: &Palette) -> gpui_kit::StyledText {
    let style = |s: Seg| -> Option<(FontWeight, Hsla)> {
        match s {
            Seg::Text => None,
            Seg::Fig => Some((theme::W_MEDIUM, p.text_strong)),
            Seg::Verdict => Some((theme::W_SEMIBOLD, verdict_color)),
        }
    };
    let owned: Vec<(&str, Option<(FontWeight, Hsla)>)> =
        segs.iter().map(|(k, t)| (t.as_str(), style(*k))).collect();
    crate::chat::inline_runs(&owned)
}

impl Onboarding {
    pub(in crate::onboarding) fn capture(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let interval = self.interval_s();
        let px_ = self.pixels();
        let v = storage::verdict(&storage::Input {
            interval_s: interval,
            pixels: px_,
            retention: &self.draft.retention_policy,
            probe: self.probe.as_ref(),
            state: self.probe_state,
            downloads: self.download_bytes(),
            semantic_on: self.features.semantic_search,
        });
        let rate_ix = storage::nearest_ladder_index(self.draft.screen_frame_rate);
        let rate = Segmented::new(
            "ob-rate",
            "How often to take a snapshot",
            storage::LADDER.iter().map(|s| storage::short_rate(*s)),
            rate_ix,
        )
        .mono()
        .on_change(on_ix(cx, |this, ix, cx| this.set_rate(ix, cx)));
        let keep = Segmented::new(
            "ob-keep",
            "How long to keep it",
            storage::RETENTION
                .iter()
                .map(|(_, d)| d.map_or("Everything".to_string(), |d| format!("Last {d} days"))),
            storage::retention_index(&self.draft.retention_policy),
        )
        .on_change(on_ix(cx, |this, ix, cx| this.set_retention(ix, cx)));

        let shown = if self.draft.save_directory.trim().is_empty() {
            self.probe
                .as_ref()
                .map(|p| p.path.clone())
                .unwrap_or_default()
        } else {
            self.draft.save_directory.clone()
        };
        let bad = v.clause.is_some();
        let path = div()
            .flex()
            .items_center()
            .h(size::H_SM)
            .px(space::S2)
            .rounded(radius::SM)
            .font_family(FONT_MONO)
            .text_size(text::BASE)
            .text_color(if bad { p.danger_text } else { p.text_strong })
            .bg(if bad { p.danger_bg } else { p.wash })
            .border_1()
            .border_color(if bad { p.danger_border } else { p.hairline })
            .child({
                let s = storage::sentence_path(&shown, 28);
                if s.is_empty() {
                    "\u{2026}".to_string()
                } else {
                    s
                }
            });
        let folder = div()
            .flex()
            .items_center()
            .gap(space::S2)
            .child(path)
            .children(v.clause.clone().map(|c| {
                div()
                    .whitespace_nowrap()
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::MD)
                    .text_color(p.danger_text)
                    .child(format!("\u{2014} {c}"))
            }))
            .child(
                button("ob-choose", "Choose\u{2026}", cx, |this, _, cx| {
                    this.pick_folder(cx)
                })
                .sm()
                .icon("icons/folder.svg"),
            );

        let cats = [
            p.cat_research,
            p.cat_organizing,
            p.cat_communication,
            p.cat_learning,
        ];
        let add = self.app_add(cx);
        let apps = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .children(
                self.draft
                    .privacy
                    .excluded_apps
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let id = a.id.clone();
                        let c = cats[i % APP_COLORS];
                        let on = a.enabled;
                        let glyph = div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(18.))
                            .rounded(px(5.))
                            .font_weight(theme::W_SEMIBOLD)
                            .text_size(text::XS)
                            .text_color(p.text_strong)
                            .bg(mix(c, p.surface_raised, 0.24))
                            .when(!on, |d| d.opacity(0.45))
                            .child(a.display_name.chars().next().unwrap_or('?').to_string());
                        div()
                            .id(gpui_kit::ElementId::Name(format!("ob-app-{id}").into()))
                            .role(gpui_kit::Role::Button)
                            .aria_label(gpui_kit::SharedString::from(format!(
                                "{} \u{2014} {}",
                                a.display_name,
                                if on {
                                    "never recorded, activate to record it"
                                } else {
                                    "recorded, activate to hide it again"
                                }
                            )))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .h(size::H_SM)
                            .pl(px(4.))
                            .pr(space::S2)
                            .rounded(radius::SM)
                            .border_1()
                            .border_color(p.border)
                            .bg(p.surface_raised)
                            .cursor_pointer()
                            .text_size(text::BASE)
                            .text_color(if on { p.text_strong } else { p.text_subtle })
                            .when(!on, |d| d.line_through())
                            .child(glyph)
                            .child(a.display_name.clone())
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_app(&id, cx)))
                    }),
            )
            .child(add);

        let tone = match v.tone {
            VTone::Bad => Some(Tone::Danger),
            VTone::Warn => Some(Tone::Warn),
            _ => None,
        };
        let (vt, vbg, vbd) = tone.map_or((p.accent, p.surface, p.border), |t| t.colors(&p));
        let verdict_color = if v.tone == VTone::Ok {
            p.accent
        } else if tone.is_some() {
            vt
        } else {
            p.text_strong
        };
        let mut plan = v.plan.clone();
        if let Some((_, first)) = plan.first_mut() {
            *first = format!(
                "{} \u{b7} {first}",
                capitalize(&storage::interval_phrase(interval))
            );
        }
        let repairs: Vec<AnyElement> = v
            .repairs
            .iter()
            .enumerate()
            .map(|(i, (label, act))| {
                let act = *act;
                button(
                    gpui_kit::ElementId::Name(format!("ob-repair-{i}").into()),
                    label.clone(),
                    cx,
                    move |this, _, cx| this.repair(act, cx),
                )
                .sm()
                .into_any_element()
            })
            .collect();
        let panel = div()
            .id("ob-verdict")
            .role(gpui_kit::Role::Status)
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(space::S4)
            .py(space::S3)
            .rounded(radius::LG)
            .border_1()
            .border_color(if tone.is_some() {
                mix(vbd, p.border, 0.7)
            } else {
                p.border
            })
            .bg(if tone.is_some() {
                mix(vbg, p.surface, 0.6)
            } else {
                p.surface
            })
            .child(
                div()
                    .text_size(text::BASE)
                    .line_height(relative(1.4))
                    .text_color(p.text_subtle)
                    .child(runs(&plan, p.text_strong, &p)),
            )
            .child(
                div()
                    .text_size(text::MD)
                    .line_height(relative(1.45))
                    .text_color(p.text)
                    .child(if v.probing {
                        runs(
                            &[(Seg::Text, "Checking that folder\u{2026}".into())],
                            p.text,
                            &p,
                        )
                    } else {
                        runs(&v.text, verdict_color, &p)
                    }),
            )
            .when(!repairs.is_empty(), |d| {
                d.child(parts::acts(repairs).mt(px(4.)))
            });

        let rows = vec![
            row(
                "Take a snapshot",
                div()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .child(rate)
                    .child(fine(storage::interval_phrase(interval), &p)),
                &p,
            ),
            row("Keep", keep, &p),
            row("Saves to", folder, &p),
            row("Never recorded", apps, &p),
        ];
        let col = vec![
            head(vec![
                kicker("Capture & Storage", &p).into_any_element(),
                display("Four things to settle.", &p).into_any_element(),
            ])
            .into_any_element(),
            parts::ruled(None, rows, &p).into_any_element(),
            panel.into_any_element(),
            fine("Estimate measured on a real Mac: 270 MB a day at one snapshot every 3 s, pause-on-inactivity on.", &p)
                .into_any_element(),
        ];
        let held = !self.can_continue();
        let mut acts = vec![back(cx)];
        if held {
            acts.push(chip("continue held", ChipKind::Warn, &p).into_any_element());
        }
        acts.push(
            primary("ob-next", "Your settings", cx, |this, _, cx| this.next(cx))
                .disabled(held)
                .into_any_element(),
        );
        (
            parts::main_col("ob-capture", false, false, self.inset, col),
            Some(parts::footer(None, acts, self.inset, &p).into_any_element()),
        )
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}
