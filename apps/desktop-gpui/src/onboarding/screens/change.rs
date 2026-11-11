//! 5 / 8 Change settings (`ChangeSettingsScreen.svelte`): the flow's ONE dense
//! screen, four sections behind a tab strip. `toggle` is the only way to flip
//! a feature (its cascades run both ways, with an undo line). AI features on
//! with nothing to run it is a soft warning beside the row, never a hold.
//! Deepgram is never offered here (Settings-only, behind its consent gate).

use capture_types::AudioTranscriptionProvider;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, AnyElement, Context, Hsla, Window};

use super::{button, on_ix, primary};
use crate::onboarding::flow::{Feature, LockFix, ModelFacts, Subsystem};
use crate::onboarding::parts::{self, fine, head};
use crate::onboarding::{storage, Onboarding};
use crate::settings::model::format_bytes;
use crate::theme::{self, leading, space, text, Palette, FONT_MONO, FONT_UI};
use crate::ui::baseline::{self, Line};
use crate::ui::choice::{Choice, ChoiceOption};
use crate::ui::notice::notice;
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;
use crate::ui::{kicker, Tone};

const SECTIONS: [(&str, &str); 4] = [
    ("What Mnema will do", "Turn the last audio source off and transcription and who's speaking switch off with it."),
    ("Engines", "Cloud transcription is not offered here \u{2014} it lives in Settings behind its own consent gate."),
    ("Models", "Sizes come from the model manifests \u{2014} nothing here is a pasted constant."),
    ("AI features", "Nothing here is sent anywhere until you pick a provider."),
];
/// `.ob-met`: a mono label and its figure, over a 3 px track. Memory is
/// measured, never published, so its fill is dashed ("about").
fn meter(label: &'static str, value: String, frac: f32, dashed: bool, p: &Palette) -> AnyElement {
    let fill = div().h_full().w(gpui_kit::relative(frac.clamp(0., 1.)));
    let fill = if dashed {
        // `repeating-linear-gradient(90deg, grey 0 3px, transparent 3px 5px)`.
        fill.flex()
            .gap(px(2.))
            .overflow_hidden()
            .children((0..120).map(|_| div().flex_none().w(px(3.)).h_full().bg(p.chart_grey_4)))
    } else {
        fill.bg(p.chart_grey_4)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .mt(px(4.))
        .child(
            div()
                .flex()
                .justify_between()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .line_height(relative(1.))
                .text_color(p.text_subtle)
                .child(label)
                .child(
                    div()
                        .font_weight(theme::W_MEDIUM)
                        .text_color(p.text)
                        .child(value),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(3.))
                .rounded(px(2.))
                .bg(p.surface_hover)
                .overflow_hidden()
                .child(fill),
        )
        .into_any_element()
}

const CLOUD_LEAD: &str = "Deepgram is real, and it is left out of setup on purpose.";
const CLOUD_NOTE: &str = "Deepgram is real, and it is left out of setup on purpose. Choosing it uploads your microphone and system-audio recordings to your own Deepgram account \u{2014} so it is switched on in Settings \u{2192} Transcription, behind a consent step, and never here.";

/// `feature-cost.ts` anchor shares (MB/day at the 3 s anchor).
const SHARE_OCR: f64 = 47.;
const SHARE_AUDIO: f64 = 23.;
const SHARE_TRANSCRIPT: f64 = 4.5;
const TRANSCRIPT_VECTOR_MB: f64 = 2.;
const ACTIVE_HOURS: f64 = 8.;

/// A kicker with its note on one baseline (`.ob-pane__head`).
pub(super) fn kicker_note(
    label: &str,
    note: gpui_kit::Div,
    p: &Palette,
    cx: &gpui_kit::App,
) -> AnyElement {
    baseline::row(
        vec![
            (
                Line::new(FONT_MONO, text::XS, parts::FINE_LH),
                kicker(label, p).into_any_element(),
            ),
            (
                Line::new(FONT_UI, text::BASE, parts::FINE_LH),
                note.into_any_element(),
            ),
        ],
        cx,
    )
    .gap(space::S3)
    .into_any_element()
}

pub(super) fn provider_id(p: AudioTranscriptionProvider) -> &'static str {
    app_core::audio_transcription_models::provider_id_for_settings(p)
}

impl Onboarding {
    /// MB/day per feature (`featureCost`).
    fn disk_cost(&self, f: Feature) -> f64 {
        let s = &self.features;
        let interval = self.interval_s();
        let scale = storage::daily_mb(interval, storage::ANCHOR_VIDEO_PIXELS) / 270.;
        let sources = s.microphone as u8 as f64 + s.system_audio as u8 as f64;
        let reads = s.screen && s.ocr;
        let dims = match self.draft.semantic_search.model_id.as_deref() {
            Some("multilingual-e5-small") | Some("granite-embedding-small-english-r2") => 384.,
            Some("bge-m3") => 1024.,
            Some("stella_en_400M_v5") => 2048.,
            Some("snowflake-arctic-embed-l-v2.0") => 256.,
            _ => 768.,
        };
        match f {
            Feature::Screen if s.screen => {
                168. * scale * (self.pixels() / storage::ANCHOR_VIDEO_PIXELS)
            }
            Feature::Ocr if reads => SHARE_OCR * scale,
            Feature::Microphone if s.microphone => SHARE_AUDIO * scale,
            Feature::SystemAudio if s.system_audio => SHARE_AUDIO * scale,
            Feature::Transcription if s.transcription => SHARE_TRANSCRIPT * sources * scale,
            Feature::SemanticSearch if s.semantic_search => {
                (if reads {
                    ACTIVE_HOURS * 3600. / interval * dims / 1e6
                } else {
                    0.
                }) + if s.transcription {
                    TRANSCRIPT_VECTOR_MB
                } else {
                    0.
                }
            }
            _ => 0.,
        }
    }

    fn engine_name(&self) -> String {
        let id = provider_id(self.draft.transcription.provider);
        self.engines
            .iter()
            .find(|e| e.provider == id)
            .and_then(|e| {
                e.models
                    .iter()
                    .find(|m| m.0 == self.draft.transcription.model_id)
            })
            .map_or_else(
                || "Whisper base".into(),
                |m| m.1.replace(" (on-device)", ""),
            )
    }

    /// Re-read one subsystem's selected-model facts after a pick.
    pub(super) fn refresh_facts(&mut self) {
        let id = provider_id(self.draft.transcription.provider).to_string();
        let t = self
            .engines
            .iter()
            .find(|e| e.provider == id)
            .and_then(|e| {
                e.models
                    .iter()
                    .find(|m| m.0 == self.draft.transcription.model_id)
            })
            .map(|m| ModelFacts {
                provider: id.clone(),
                model_id: m.0.clone(),
                bytes: m.2,
                installed: m.3,
            });
        let sem = self
            .semantic
            .iter()
            .find(|m| Some(&m.0) == self.draft.semantic_search.model_id.as_ref())
            .map(|m| ModelFacts {
                provider: "local".into(),
                model_id: Some(m.0.clone()),
                bytes: m.2,
                installed: m.3,
            });
        for (sub, facts) in [(Subsystem::Transcription, t), (Subsystem::Semantic, sem)] {
            if let (Some(facts), Some(slot)) =
                (facts, self.models.iter_mut().find(|(s, _)| *s == sub))
            {
                slot.1 = facts;
            }
        }
    }

    pub fn set_engine(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(e) = self.engines.get(ix).cloned() else {
            return;
        };
        if let Ok(p) =
            serde_json::from_value::<AudioTranscriptionProvider>(serde_json::json!(e.provider))
        {
            self.draft.transcription.provider = p;
            self.draft.transcription.model_id =
                crate::onboarding::engines::default_model(&e).and_then(|m| m.0.clone());
            self.refresh_facts();
            cx.notify();
        }
    }

    fn feature_row(&self, f: Feature, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let s = &self.features;
        let on = s.get(f);
        let src: Option<Hsla> = match f {
            Feature::Screen => Some(p.source_screen),
            Feature::Microphone => Some(p.source_mic),
            Feature::SystemAudio => Some(p.source_sysaudio),
            _ => None,
        };
        let disk = self.disk_cost(f).round();
        let dl: u64 = self
            .work_list()
            .iter()
            .filter(|w| w.feature == f)
            .map(|w| w.bytes)
            .sum();
        let cost = [
            (disk > 0.).then(|| format!("{disk} MB/day")),
            (dl > 0).then(|| format!("{} once", format_bytes(dl as f64))),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
        let fix = s.lock_fix(f);
        let locked = fix.is_some();
        let ctl: AnyElement = match fix {
            Some((kind, target, label)) => button(
                format!("ob-fix-{f:?}"),
                label,
                cx,
                move |this, _, cx| match kind {
                    LockFix::Grant => this.request(crate::onboarding::Moment::Microphone, cx),
                    LockFix::Toggle => this.toggle(target, cx),
                },
            )
            .sm()
            .into_any_element(),
            None => Switch::new(format!("ob-feat-{f:?}"), f.label(), on)
                .when_some(src, |sw, c| sw.on_color(c))
                .on_toggle({
                    let this = cx.entity().downgrade();
                    move |_, _, cx| {
                        let _ = this.update(cx, |o, cx| o.toggle(f, cx));
                    }
                })
                .into_any_element(),
        };
        let depth = f.depth() as f32;
        // `.ob-feat.cut`: the parent has switched this dependent row off.
        let cut = match f {
            Feature::Ocr => !s.screen,
            Feature::Transcription => !s.microphone && !s.system_audio,
            Feature::SpeakerSeparation => !s.transcription,
            _ => false,
        };
        // `.ob-feat.kid::before`: the tree elbow, hierarchy rather than a selection mark.
        let elbow = (depth > 0.).then(|| {
            div()
                .absolute()
                .left(px(depth * 26. - 16.))
                .top(px(12.))
                .w(px(9.))
                .h(px(10.))
                .border_l_1()
                .border_b_1()
                .rounded_bl(px(4.))
                .border_color(if cut { p.text_faint } else { p.border_strong })
                .when(cut, |d| d.border_dashed())
        });
        div()
            .relative()
            .flex()
            .items_center()
            .gap(space::S3)
            .min_h(px(46.))
            .py(px(6.))
            .pl(px(depth * 26.))
            .border_b_1()
            .border_color(p.hairline)
            .children(elbow)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S2)
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MD)
                            .line_height(relative(1.25))
                            .text_color(if on { p.text_strong } else { p.text_muted })
                            .children(src.map(|c| div().size(px(7.)).rounded(px(2.)).bg(c)))
                            .child(f.label()),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(text::BASE)
                            .line_height(relative(1.35))
                            .text_color(if locked { p.warn } else { p.text_subtle })
                            .child(s.note(f, self.ai_ready(), &self.engine_name())),
                    ),
            )
            .child(div().w(px(170.)).flex().justify_end().child(parts::mono(
                if cost.is_empty() {
                    "\u{2014}".into()
                } else {
                    cost
                },
                text::SM,
                p.text_subtle,
            )))
            .child(div().w(px(64.)).flex().justify_end().child(ctl))
            .into_any_element()
    }

    fn features_pane(&self, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        let mut v = vec![kicker_note(
            "What Mnema will do",
            fine("Indented rows need the row above them.", p),
            p,
            cx,
        )];
        v.push(
            parts::ruled(
                None,
                Feature::ORDER
                    .into_iter()
                    .map(|f| self.feature_row(f, cx, p))
                    .collect(),
                p,
            )
            .into_any_element(),
        );
        if self.features.ai_features && !self.ai_ready() {
            v.push(
                notice(
                    Tone::Warn,
                    "icons/triangle-alert.svg",
                    "AI features are on with nothing to run them.",
                    None,
                    vec![
                        button("ob-ai-connect", "Connect a provider", cx, |this, _, cx| {
                            this.tab = 3;
                            this.ai.mode = Some(true);
                            cx.notify();
                        })
                        .sm()
                        .into_any_element(),
                        button("ob-ai-off", "Turn it off", cx, |this, _, cx| {
                            this.toggle(Feature::AiFeatures, cx)
                        })
                        .ghost()
                        .sm()
                        .into_any_element(),
                    ],
                    p,
                )
                .into_any_element(),
            );
        }
        if let Some((_, text_)) = &self.undo {
            v.push(
                notice(
                    Tone::Info,
                    "icons/info.svg",
                    text_.clone(),
                    None,
                    vec![
                        button("ob-undo", "Undo", cx, |this, _, cx| this.undo_toggle(cx))
                            .ghost()
                            .sm()
                            .into_any_element(),
                    ],
                    p,
                )
                .into_any_element(),
            );
        }
        let total: f64 = Feature::ORDER.iter().map(|f| self.disk_cost(*f)).sum();
        let bar = [
            (Feature::Screen, p.source_screen),
            (Feature::Ocr, p.chart_grey_4),
            (Feature::Microphone, p.source_mic),
            (Feature::SystemAudio, p.source_sysaudio),
            (Feature::Transcription, p.chart_grey_3),
            (Feature::SemanticSearch, p.info),
        ];
        let work = self.work_list();
        let dl = if work.is_empty() {
            "nothing".to_string()
        } else {
            format!(
                "{}{}",
                if work.iter().any(|w| w.approx()) {
                    "about "
                } else {
                    ""
                },
                format_bytes(crate::onboarding::flow::work_bytes(&work) as f64)
            )
        };
        let kv = |k: &'static str, val: String| {
            baseline::row(
                vec![
                    (
                        Line::new(FONT_UI, text::BASE, leading::BODY),
                        div().text_color(p.text_subtle).child(k).into_any_element(),
                    ),
                    (
                        Line::new(FONT_MONO, text::BASE, leading::ONE),
                        parts::mono(val, text::BASE, p.text_strong)
                            .font_weight(theme::W_MEDIUM)
                            .into_any_element(),
                    ),
                ],
                cx,
            )
            .gap(px(6.))
        };
        v.push(
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .h(px(6.))
                        .rounded(px(3.))
                        .overflow_hidden()
                        .bg(p.surface_hover)
                        .children(bar.into_iter().map(|(f, c)| {
                            let frac = if total > 0. {
                                self.disk_cost(f) / total
                            } else {
                                0.
                            };
                            div().h_full().w(relative(frac as f32)).bg(c)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(space::S6)
                        .child(kv(
                            "disk, once it is running",
                            format!(
                                "{}{} MB/day",
                                if self.features.semantic_search {
                                    "about "
                                } else {
                                    ""
                                },
                                total.round()
                            ),
                        ))
                        .child(kv("download", dl)),
                )
                .into_any_element(),
        );
        v
    }

    fn engines_pane(&self, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        use crate::onboarding::engines::{self, RECOMMENDED};
        let id = provider_id(self.draft.transcription.provider);
        let cards = engines::cards(&self.engines);
        let max_bytes = cards.iter().filter_map(|c| c.bytes).max().unwrap_or(0);
        let max_ram = cards.iter().filter_map(|c| c.ram).max().unwrap_or(0);
        let opts: Vec<ChoiceOption> = cards
            .iter()
            .map(|c| {
                let o = ChoiceOption::new(c.name.clone()).desc(c.line.clone());
                let o = match c.icon {
                    Some(icon) => o.icon(icon),
                    None => o,
                };
                let o = if c.provider == RECOMMENDED {
                    o.tag("recommended")
                } else {
                    o
                };
                // `.foot` then the two `.ob-met` meters.
                let (foot, download, memory) =
                    (c.foot.clone(), c.download.clone(), c.memory.clone());
                let (dl, mem) = (
                    engines::track(c.bytes, max_bytes),
                    engines::track(c.ram, max_ram),
                );
                o.below(move |p| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .children(foot.clone().map(|f| {
                            div()
                                .text_size(text::SM)
                                .line_height(relative(parts::FINE_LH))
                                .text_color(p.text_subtle)
                                .child(f)
                        }))
                        .child(meter("download", download.clone(), dl, false, p))
                        .child(meter("memory", memory.clone(), mem, true, p))
                        .into_any_element()
                })
            })
            .collect();
        let selected = cards.iter().position(|c| c.provider == id).unwrap_or(0);
        let (delta, up) = engines::delta(&cards, id);
        let recommended_ix = cards.iter().position(|c| c.provider == RECOMMENDED);
        let acts = div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(
                div()
                    .ml(space::S2)
                    .text_size(text::BASE)
                    .text_color(if up { p.warn } else { p.text_subtle })
                    .child(delta),
            )
            .when_some(recommended_ix.filter(|_| id != RECOMMENDED), |d, ix| {
                d.child(
                    button(
                        "ob-engine-recommended",
                        "Use recommended",
                        cx,
                        move |this, _, cx| this.set_engine(ix, cx),
                    )
                    .sm(),
                )
            });
        let open = self.cloud_open;
        let cloud = div()
            .flex()
            .flex_col()
            .gap(space::S2)
            .text_size(text::SM)
            .line_height(relative(1.5))
            .text_color(p.text_subtle)
            .child(
                div()
                    .id("ob-engine-cloud")
                    .role(gpui_kit::Role::Button)
                    .aria_expanded(open)
                    .cursor_pointer()
                    .hover(|s| s.text_color(p.text))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(crate::ui::icon(
                        if open {
                            "icons/chevron-down.svg"
                        } else {
                            "icons/chevron-right.svg"
                        },
                        px(10.),
                    ))
                    .child("Where\u{2019}s the cloud option?")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cloud_open = !this.cloud_open;
                        cx.notify();
                    }))
                    .test_support(),
            )
            .when(open, |d| {
                d.child(div().max_w(px(380.)).child(
                    gpui_kit::StyledText::new(CLOUD_NOTE).with_highlights(vec![(
                        0..CLOUD_LEAD.len(),
                        gpui_kit::HighlightStyle {
                            color: Some(p.text),
                            font_weight: Some(theme::W_SEMIBOLD),
                            ..Default::default()
                        },
                    )]),
                ))
            });
        // `.ob-pane { gap: var(--s-4) }`: one block, so its parts sit 16 px apart.
        vec![div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .children([
            head(vec![
                parts::display("Your voice never leaves this Mac.", p).text_size(text::XL).into_any_element(),
                fine("Every engine below transcribes on this machine. Once an engine\u{2019}s model is on disk, it keeps working with the network off.", p).into_any_element(),
            ])
            .into_any_element(),
            // `.ob-pane__head` sits 8 px over the tiles (its −8 px margin on the pane's gap).
            div()
                .flex()
                .flex_col()
                .gap(space::S2)
                .child(kicker_note("Turning speech into text", fine("one is picked for you", p), p, cx))
                .child(
                    div()
                        .w_full()
                        .child(Choice::new("ob-engines", "Transcription engine", opts, selected).on_change(on_ix(cx, |this, ix, cx| this.set_engine(ix, cx)))),
                )
                .into_any_element(),
            acts.into_any_element(),
            fine("Memory figures are measured on a running app, not published in any manifest \u{2014} they always read \u{201c}about\u{201d}. The one ordering the manifest states is memory: Parakeet uses the most.", p)
                .text_size(text::SM)
                .max_w(px(380.))
                .into_any_element(),
            cloud.into_any_element(),
            ])
            .into_any_element()]
    }

    pub(in crate::onboarding) fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = theme::tokens(cx);
        let at = self.tab.min(SECTIONS.len() - 1);
        let tabs = div()
            .flex()
            .items_center()
            .gap(space::S3)
            .child(
                Segmented::new("ob-tabs", "Sections", SECTIONS.map(|s| s.0), at)
                    .md()
                    .on_change(on_ix(cx, |this, ix, cx| {
                        this.tab = ix;
                        cx.notify();
                    })),
            )
            .child(parts::mono(
                format!("{} / {}", at + 1, SECTIONS.len()),
                text::SM,
                p.text_subtle,
            ));
        let mut col = vec![tabs.into_any_element()];
        col.extend(match at {
            0 => self.features_pane(cx, &p),
            1 => self.engines_pane(cx, &p),
            2 => self.models_pane(window, cx, &p),
            _ => self.ai_pane(window, cx, &p),
        });
        let back_btn = if at == 0 {
            button("ob-back", "Back to your settings", cx, |this, _, cx| {
                this.back(cx)
            })
        } else {
            button("ob-back", SECTIONS[at - 1].0, cx, move |this, _, cx| {
                this.tab = at - 1;
                cx.notify();
            })
        };
        let next_btn = if at == SECTIONS.len() - 1 {
            primary("ob-next", "Back to your settings", cx, |this, _, cx| {
                this.next(cx)
            })
        } else {
            primary("ob-next", SECTIONS[at + 1].0, cx, move |this, _, cx| {
                this.tab = at + 1;
                cx.notify();
            })
        };
        let foot = parts::footer(
            Some(SECTIONS[at].1.into_any_element()),
            vec![
                back_btn.ghost().into_any_element(),
                next_btn.into_any_element(),
            ],
            self.inset,
            &p,
        );
        (
            parts::main_col("ob-change", true, true, self.inset, col),
            Some(foot.into_any_element()),
        )
    }
}
