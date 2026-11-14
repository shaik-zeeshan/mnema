//! 4 / 8 Your settings (`YourSettingsScreen.svelte`): a READ-ONLY manifest, a
//! 2×4 grid of tick + name + one short value, a mono facts row, two actions.
//! Every change is a round trip to Change settings.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context};

use super::{back, button, primary};
use crate::onboarding::flow::{self, Step};
use crate::onboarding::parts::{self, display, head, Mark};
use crate::onboarding::{storage, Onboarding};
use crate::theme::{self, space, text, Palette, FONT_MONO};
use crate::ui::kicker;
use crate::ui::Tracking as _;

struct Row {
    name: &'static str,
    value: String,
    on: bool,
    warned: bool,
    grant: bool,
    unconfirmed: bool,
}

fn row(name: &'static str, value: impl Into<String>, on: bool) -> Row {
    Row {
        name,
        value: value.into(),
        on,
        warned: false,
        grant: false,
        unconfirmed: false,
    }
}

/// `formatSize`: decimal MB / GB, one place.
fn size(bytes: f64) -> String {
    if bytes <= 0. {
        "0 MB".into()
    } else if bytes >= 1e9 {
        format!("{:.1} GB", bytes / 1e9)
    } else {
        format!("{} MB", (bytes / 1e6).round())
    }
}

impl Onboarding {
    fn manifest(&self) -> Vec<Row> {
        let f = &self.features;
        let not_granted = |name| Row {
            warned: true,
            grant: true,
            ..row(name, "not granted", false)
        };
        let audio = f.microphone || f.system_audio;
        vec![
            if f.perms.screen {
                row("Screen capture", on_off(f.screen), f.screen)
            } else {
                not_granted("Screen capture")
            },
            row("Read on-screen text", on_off(f.ocr), f.ocr),
            if f.perms.microphone {
                row("Microphone", on_off(f.microphone), f.microphone)
            } else {
                not_granted("Microphone")
            },
            Row {
                unconfirmed: f.system_audio,
                ..row(
                    "System audio",
                    if !f.system_audio {
                        "off"
                    } else if f.system_audio_needs_request() {
                        "on \u{b7} macOS can\u{2019}t confirm the grant"
                    } else {
                        "on"
                    },
                    f.system_audio,
                )
            },
            row(
                "Transcription",
                if f.transcription {
                    "on"
                } else if audio {
                    "off"
                } else {
                    "off \u{2014} no audio source"
                },
                f.transcription,
            ),
            row(
                "Who\u{2019}s speaking",
                if f.speaker_separation {
                    "on"
                } else if f.transcription {
                    "off"
                } else {
                    "off \u{2014} needs transcription"
                },
                f.speaker_separation,
            ),
            row(
                "Semantic Search",
                on_off(f.semantic_search),
                f.semantic_search,
            ),
            row(
                "AI features",
                if !f.ai_features {
                    "off \u{2014} needs a provider"
                } else if self.ai_ready() {
                    "on"
                } else {
                    "on \u{b7} nothing connected yet"
                },
                f.ai_features,
            ),
        ]
    }

    fn cell(&self, r: &Row, ix: usize, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let m = if r.unconfirmed {
            Mark::Half
        } else if r.on {
            Mark::On
        } else {
            Mark::Off
        };
        div()
            .flex()
            .items_center()
            .gap(space::S3)
            .h(px(42.))
            .border_b_1()
            .border_color(p.hairline)
            .child(parts::mark(m, p))
            .child(
                div()
                    .whitespace_nowrap()
                    .font_weight(theme::W_MEDIUM)
                    .text_size(text::MD)
                    .text_color(if r.on { p.text_strong } else { p.text_subtle })
                    .child(r.name),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .whitespace_nowrap()
                    .font_family(FONT_MONO)
                    .text_size(text::BASE)
                    .text_color(if r.warned {
                        p.warn
                    } else if r.on {
                        p.text_muted
                    } else {
                        p.text_subtle
                    })
                    .child(r.value.clone())
                    .when(r.grant, |d| {
                        d.child(
                            button(
                                gpui_kit::ElementId::Name(format!("ob-grant-{ix}").into()),
                                "Grant",
                                cx,
                                |this, _, cx| this.go(Step::Permissions, cx),
                            )
                            .ghost()
                            .sm(),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(in crate::onboarding) fn your_settings(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let rows = self.manifest();
        // Two columns of four, capture | processing.
        let col = |ixs: [usize; 4], this: &Self, cx: &mut Context<Self>| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .children(ixs.map(|k| this.cell(&rows[k], k, cx, &p)))
        };
        let left = col([0, 1, 2, 3], self, cx);
        let right = col([4, 5, 6, 7], self, cx);
        let manifest = div()
            .flex()
            .gap(space::S6)
            .border_t_1()
            .border_color(p.hairline)
            .child(left)
            .child(right);
        let work = self.work_list();
        let approx = work.iter().any(|w| w.approx());
        let downloads = if work.is_empty() {
            "Nothing".to_string()
        } else {
            format!(
                "{}{}",
                if approx { "~" } else { "" },
                size(flow::work_bytes(&work) as f64)
            )
        };
        let dir = self.draft.save_directory.trim();
        let saves = if dir.is_empty() {
            self.probe
                .as_ref()
                .map_or("the default folder".into(), |p| {
                    storage::sentence_path(&p.path, 40)
                })
        } else {
            storage::sentence_path(dir, 40)
        };
        let keeps = storage::RETENTION[storage::retention_index(&self.draft.retention_policy)]
            .1
            .map_or("Everything".to_string(), |d| format!("{d} days"));
        let mb = storage::daily_mb(self.interval_s(), self.pixels());
        let daily = if mb >= 1000. {
            format!("{:.1} GB", mb / 1000.)
        } else {
            format!("{} MB", mb.round())
        };
        let fact = |k: &'static str, v: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    parts::mono(k.to_uppercase(), text::XS, p.text_subtle)
                        .font_weight(theme::W_MEDIUM)
                        .tracking(0.1),
                )
                .child(parts::mono(v, text::LG, p.text_strong).tracking(-0.02))
        };
        let facts = div()
            .flex()
            .gap(space::S7)
            .child(fact("Download", downloads))
            .child(fact("Saves to", saves))
            .child(fact("Keeps", keeps))
            .child(fact("Per day", format!("~{daily}")));
        let col = vec![
            head(vec![
                kicker("Your settings", &p).into_any_element(),
                display("Already answered.", &p).into_any_element(),
            ])
            .into_any_element(),
            manifest.into_any_element(),
            facts.into_any_element(),
        ];
        let foot = parts::footer(
            Some("Every line here stays changeable in Settings.".into_any_element()),
            vec![
                back(cx),
                button("ob-change", "Change settings", cx, |this, _, cx| {
                    this.go(Step::Change, cx)
                })
                .into_any_element(),
                primary("ob-next", "Start", cx, |this, _, cx| this.next(cx)).into_any_element(),
            ],
            self.inset,
            &p,
        );
        (
            parts::main_col("ob-settings", false, false, self.inset, col),
            Some(foot.into_any_element()),
        )
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}
