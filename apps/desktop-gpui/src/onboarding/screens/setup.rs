//! 6 / 8 Setup (`SetupScreen.svelte`): downloads, and the gate that no longer
//! exists. Continue is live on arrival and has NO disabled binding at all: not
//! while downloading, not on a failure, a cancel, or an empty work list.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, FontWeight, Hsla};

use super::{back, button, primary};
use crate::onboarding::downloads::DlState;
use crate::onboarding::parts::{self, fine, head};
use crate::onboarding::Onboarding;
use crate::settings::model::format_bytes;
use crate::theme::{self, leading, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::notice::{inline, notice};
use crate::ui::{icon, Tone};

/// The big percentage and its smaller "%" (`.ob-pct`).
const PCT_LH: f32 = 0.9;
const PCT_UNIT: gpui_kit::Pixels = px(28.);

fn line(runs: &[(&str, Option<(FontWeight, Hsla)>)], p: &Palette) -> AnyElement {
    div()
        .text_size(px(15.))
        .line_height(relative(1.4))
        .text_color(p.text_muted)
        .child(crate::chat::inline_runs(runs))
        .into_any_element()
}

impl Onboarding {
    pub(in crate::onboarding) fn setup(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let items = self.work_list();
        let f = self.features;
        let active: Vec<_> = items.iter().filter(|i| f.get(i.feature)).cloned().collect();
        let bytes: u64 = active.iter().map(|i| i.bytes).sum();
        let got: u64 = active
            .iter()
            .map(|i| self.dl.received(i).min(i.bytes))
            .sum();
        let pct = self.dl.percent(&items, &f);
        let total = format!(
            "{}{}",
            if active.iter().any(|i| i.approx()) {
                "~"
            } else {
                ""
            },
            format_bytes(bytes as f64)
        );
        let shortfall = self.download_shortfall().filter(|_| !self.dl.disk_override);
        let failed = active
            .iter()
            .filter(|i| self.dl.state(i) == DlState::Failed)
            .count();
        let now = active
            .iter()
            .find(|i| self.dl.state(i) == DlState::Downloading);
        let strong = Some((theme::W_SEMIBOLD, p.text_strong));
        let num = Some((theme::W_REGULAR, p.text_strong));
        let got_s = format!("{} of {total}", format_bytes(got as f64));
        let status = if items.is_empty() {
            line(
                &[(
                    "Nothing to download \u{2014} everything you need is already here.",
                    None,
                )],
                &p,
            )
        } else if shortfall.is_some() {
            line(
                &[
                    ("Downloads are held until there is room \u{b7} ", None),
                    (&total, num),
                    (" to fetch.", None),
                ],
                &p,
            )
        } else if let Some(i) = now {
            line(
                &[
                    ("Downloading ", None),
                    (i.name(), strong),
                    (" \u{b7} ", None),
                    (&got_s, num),
                ],
                &p,
            )
        } else if active.is_empty() {
            line(&[("Every download was cancelled.", None)], &p)
        } else if self.dl.all_ready(&items, &f) {
            line(
                &[
                    ("All set \u{b7} ", None),
                    (&total, num),
                    (" downloaded.", None),
                ],
                &p,
            )
        } else if failed > 0 {
            let n = if failed == 1 {
                "One download".to_string()
            } else {
                format!("{failed} downloads")
            };
            line(
                &[
                    (&n, None),
                    (" didn\u{2019}t finish \u{b7} the rest carry on.", None),
                ],
                &p,
            )
        } else {
            line(
                &[
                    ("Starting ", None),
                    (active[0].name(), strong),
                    (" \u{b7} ", None),
                    (&total, num),
                    (" to fetch.", None),
                ],
                &p,
            )
        };
        let pct_el = baseline::row(
            vec![
                (
                    Line::new(FONT_MONO, parts::PCT, PCT_LH),
                    div().child(pct.to_string()).into_any_element(),
                ),
                (
                    Line::new(FONT_MONO, PCT_UNIT, PCT_LH),
                    div()
                        .text_color(p.text_subtle)
                        .child("%")
                        .into_any_element(),
                ),
            ],
            cx,
        )
        .font_weight(gpui_kit::FontWeight::LIGHT)
        .text_color(p.text_strong);

        let mut col = vec![
            crate::ui::kicker("Setup", &p).into_any_element(),
            head(vec![pct_el.into_any_element(), status]).into_any_element(),
        ];
        if let Some((free, need)) = shortfall {
            col.push(
                notice(
                    Tone::Warn,
                    "icons/hard-drive.svg",
                    "Not enough room for the downloads",
                    Some(
                        format!(
                            "{} free \u{b7} {total} needed. Free some space, or start them anyway.",
                            format_bytes(free)
                        )
                        .into(),
                    ),
                    vec![
                        button("ob-dl-anyway", "Download anyway", cx, |this, _, cx| {
                            this.dl.disk_override = true;
                            this.drive_downloads(cx);
                            cx.notify();
                        })
                        .sm()
                        .into_any_element(),
                    ],
                    &p,
                )
                .max_w(px(620.))
                .into_any_element(),
            );
            let _ = need;
        }
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(k, item)| {
                let off = !f.get(item.feature);
                let state = self.dl.state(item);
                let it = item.clone();
                let st: AnyElement = match state {
                    // Ready wins over a switched-off feature: the bytes ARE on disk.
                    DlState::Ready => div()
                        .flex()
                        .items_center()
                        .gap(space::S2)
                        .text_size(text::BASE)
                        .text_color(p.accent)
                        .child(icon("icons/check.svg", px(14.)))
                        .child("Ready")
                        .into_any_element(),
                    _ if off => div()
                        .flex()
                        .items_center()
                        .gap(space::S2)
                        .text_size(text::BASE)
                        .text_color(p.warn)
                        .child("Turned off")
                        .child(
                            button(
                                format!("ob-restore-{k}"),
                                "Turn it back on",
                                cx,
                                move |this, _, cx| this.restore_item(&it, cx),
                            )
                            .sm(),
                        )
                        .into_any_element(),
                    DlState::Downloading => div()
                        .flex()
                        .items_center()
                        .gap(space::S2)
                        .text_size(text::BASE)
                        .text_color(p.text_subtle)
                        .child("Downloading")
                        .child(
                            button(
                                format!("ob-cancel-{k}"),
                                "Cancel",
                                cx,
                                move |this, _, cx| this.cancel_item(&it, cx),
                            )
                            .sm(),
                        )
                        .into_any_element(),
                    DlState::Failed => inline(
                        Tone::Danger,
                        Some(icon("icons/triangle-alert.svg", px(13.)).into_any_element()),
                        vec![
                            div()
                                .child(
                                    self.dl
                                        .errors
                                        .get(&crate::onboarding::downloads::id(item))
                                        .cloned()
                                        .unwrap_or_else(|| "Download failed.".into()),
                                )
                                .into_any_element(),
                            button(format!("ob-retry-{k}"), "Retry", cx, move |this, _, cx| {
                                this.retry_item(&it, cx)
                            })
                            .ghost()
                            .sm()
                            .icon("icons/rotate-ccw.svg")
                            .into_any_element(),
                        ],
                        &p,
                    )
                    .into_any_element(),
                    DlState::Queued => div()
                        .text_size(text::BASE)
                        .text_color(p.text_subtle)
                        .child(if shortfall.is_some() {
                            "Held"
                        } else {
                            "Queued"
                        })
                        .into_any_element(),
                };
                let frac = if item.bytes > 0 {
                    self.dl.received(item) as f32 / item.bytes as f32
                } else {
                    0.
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .py(space::S3)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S3)
                            .child(
                                baseline::row(
                                    vec![
                                        (
                                            Line::new(FONT_UI, text::MD, leading::BODY),
                                            div().child(item.name()).into_any_element(),
                                        ),
                                        (
                                            Line::new(FONT_MONO, text::BASE, leading::ONE),
                                            parts::mono(
                                                format!(
                                                    "{}{}",
                                                    if item.approx() { "about " } else { "" },
                                                    format_bytes(item.bytes as f64)
                                                ),
                                                text::BASE,
                                                p.text_subtle,
                                            )
                                            .into_any_element(),
                                        ),
                                    ],
                                    cx,
                                )
                                .flex_1()
                                .gap(space::S2)
                                .font_weight(theme::W_MEDIUM)
                                .text_color(
                                    if off && state != DlState::Ready {
                                        p.text_subtle
                                    } else {
                                        p.text_strong
                                    },
                                ),
                            )
                            .child(st),
                    )
                    .when(
                        !off && (state == DlState::Downloading
                            || (state == DlState::Failed && frac > 0.)),
                        |d| d.child(parts::track(frac, state == DlState::Failed, None, &p)),
                    )
                    .when(!off && state == DlState::Failed, |d| {
                        d.child(fine(
                            format!(
                                "{} stays off until this arrives. Continue still works.",
                                item.name()
                            ),
                            &p,
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        col.push(if rows.is_empty() {
            fine(
                "Every model your settings need is already on this machine.",
                &p,
            )
            .into_any_element()
        } else {
            parts::ruled(Some(px(620.)), rows, &p).into_any_element()
        });
        let foot = parts::footer(
            Some(
                "Live since you arrived, and it never disables. Downloads carry on behind you."
                    .into_any_element(),
            ),
            vec![
                back(cx),
                primary("ob-next", "Continue", cx, |this, _, cx| this.next(cx)).into_any_element(),
            ],
            self.inset,
            &p,
        );
        (
            parts::main_col("ob-setup", false, false, self.inset, col),
            Some(foot.into_any_element()),
        )
    }
}
