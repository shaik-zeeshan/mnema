//! 5 / 8 Change settings (`ChangeSettingsScreen.svelte`): the flow's ONE dense
//! screen, four sections behind a tab strip. `toggle` is the only way to flip
//! a feature (its cascades run both ways, with an undo line). AI features on
//! with nothing to run it is a soft warning beside the row, never a hold.
//! Deepgram is never offered here (Settings-only, behind its consent gate).

use capture_types::AudioTranscriptionProvider;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, Context, Hsla, Pixels, Window};

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
/// `feature-cost.ts` anchor shares (MB/day at the 3 s anchor).
const SHARE_OCR: f64 = 47.;
const SHARE_AUDIO: f64 = 23.;
const SHARE_TRANSCRIPT: f64 = 4.5;
const TRANSCRIPT_VECTOR_MB: f64 = 2.;
const ACTIVE_HOURS: f64 = 8.;

/// The big download total beside its fine print (`.ob-total b`, 22 px).
const TOTAL: Pixels = px(22.);

type SemanticModel = (String, String, u64, bool);

/// shortcut: the language comes from the display name's "(Multilingual…)" tag,
/// not the catalog's `multilingual` flag; read the catalog if a name ever drops it.
fn is_multilingual(m: &SemanticModel) -> bool {
    m.1.contains("(Multilingual")
}

/// Indices into `semantic` of one language group, in catalog order.
fn family_builds(semantic: &[SemanticModel], multilingual: bool) -> Vec<usize> {
    (0..semantic.len())
        .filter(|&i| is_multilingual(&semantic[i]) == multilingual)
        .collect()
}

/// `model-budget.ts` `SHORT_LABELS`: pill labels; others drop the "(…)" tag.
fn short_label(m: &SemanticModel) -> &str {
    match m.0.as_str() {
        "nomic-embed-text-v1.5" => "Nomic",
        "stella_en_400M_v5" => "Stella",
        "multilingual-e5-small" => "E5 Small",
        "bge-m3" => "BGE-M3",
        "snowflake-arctic-embed-l-v2.0" => "Snowflake",
        "gte-modernbert-base" => "GTE",
        "granite-embedding-english-r2" => "Granite R2",
        "granite-embedding-small-english-r2" => "Granite Small",
        _ => m.1.split(" (").next().unwrap_or(&m.1),
    }
}

/// A kicker with its note on one baseline (`.ob-pane__head`).
fn kicker_note(label: &str, note: gpui_kit::Div, p: &Palette, cx: &gpui_kit::App) -> AnyElement {
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

fn provider_id(p: AudioTranscriptionProvider) -> &'static str {
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
    fn refresh_facts(&mut self) {
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
            // Whisper's recommended build is Base; other engines start at their first.
            self.draft.transcription.model_id = e
                .models
                .iter()
                .find(|m| m.0.as_deref() == Some("base"))
                .or(e.models.first())
                .and_then(|m| m.0.clone());
            self.refresh_facts();
            cx.notify();
        }
    }

    pub fn set_transcription_model(&mut self, ix: usize, cx: &mut Context<Self>) {
        let id = provider_id(self.draft.transcription.provider);
        if let Some(m) = self
            .engines
            .iter()
            .find(|e| e.provider == id)
            .and_then(|e| e.models.get(ix))
        {
            self.draft.transcription.model_id = m.0.clone();
            self.refresh_facts();
            cx.notify();
        }
    }

    /// Picks the first build in the English or Multilingual group.
    pub fn set_semantic_family(&mut self, multilingual: bool, cx: &mut Context<Self>) {
        if let Some(&ix) = family_builds(&self.semantic, multilingual).first() {
            self.set_semantic_model(ix, cx);
        }
    }

    pub fn set_semantic_model(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(m) = self.semantic.get(ix) {
            self.draft.semantic_search.model_id = Some(m.0.clone());
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
        let id = provider_id(self.draft.transcription.provider);
        let opts: Vec<ChoiceOption> = self
            .engines
            .iter()
            .map(|e| {
                let bytes = e.models.iter().find(|m| m.0.as_deref() == Some("base")).or(e.models.first()).map_or(0, |m| m.2);
                let o = ChoiceOption::new(e.name.replace(" (on-device)", ""))
                    .tag(if bytes > 0 { format!("download {}", format_bytes(bytes as f64)) } else { "nothing to download".into() })
                    ;
                if e.provider == "local_whisper" { o.desc("Balanced speed, size and quality \u{2014} the manifest\u{2019}s reason for Base.") } else { o }
            })
            .collect();
        let selected = self
            .engines
            .iter()
            .position(|e| e.provider == id)
            .unwrap_or(0);
        vec![
            head(vec![
                parts::display("Your voice never leaves this Mac.", p).text_size(text::XL).into_any_element(),
                fine("Every engine below transcribes on this machine. Once its model is on disk, it keeps working with the network off.", p).into_any_element(),
            ])
            .into_any_element(),
            div()
                .w_full()
                .child(Choice::new("ob-engines", "Transcription engine", opts, selected).on_change(on_ix(cx, |this, ix, cx| this.set_engine(ix, cx))))
                .into_any_element(),
        ]
    }

    fn models_pane(&self, cx: &mut Context<Self>, p: &Palette) -> Vec<AnyElement> {
        let id = provider_id(self.draft.transcription.provider);
        let engine = self.engines.iter().find(|e| e.provider == id);
        let t_models: Vec<String> =
            engine.map_or(vec![], |e| e.models.iter().map(|m| m.1.clone()).collect());
        let t_sel = engine
            .and_then(|e| {
                e.models
                    .iter()
                    .position(|m| m.0 == self.draft.transcription.model_id)
            })
            .unwrap_or(0);
        let s_sel = self
            .semantic
            .iter()
            .position(|m| Some(&m.0) == self.draft.semantic_search.model_id.as_ref())
            .unwrap_or(0);
        let work = self.work_list();
        let total = if work.is_empty() {
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
        let mut v = vec![kicker_note(
            "Transcription",
            fine(
                format!(
                    "Engine: {} \u{2014} chosen on Engines. This screen picks its build.",
                    engine.map_or("Whisper", |e| e.name.as_str())
                ),
                p,
            ),
            p,
            cx,
        )];
        if t_models.len() > 1 {
            v.push(
                Segmented::new("ob-t-model", "Transcription build", t_models, t_sel)
                    .on_change(on_ix(cx, |this, ix, cx| {
                        this.set_transcription_model(ix, cx)
                    }))
                    .into_any_element(),
            );
        }
        v.push(kicker("Semantic Search", p).into_any_element());
        if !self.semantic.is_empty() {
            // `ModelPickers.svelte`: a language group, then that group's builds
            // under short labels. One segment per catalog model ran off the window.
            let multi = self.semantic.get(s_sel).is_some_and(is_multilingual);
            let builds = family_builds(&self.semantic, multi);
            let at = builds.iter().position(|&i| i == s_sel).unwrap_or(0);
            let labels: Vec<&str> = builds
                .iter()
                .map(|&i| short_label(&self.semantic[i]))
                .collect();
            v.push(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space::S3)
                    .child(
                        Segmented::new(
                            "ob-s-family",
                            "Semantic Search languages",
                            ["English", "Multilingual"],
                            multi as usize,
                        )
                        .on_change(on_ix(cx, |this, ix, cx| {
                            this.set_semantic_family(ix == 1, cx)
                        })),
                    )
                    .when(builds.len() > 1, |row| {
                        row.child(
                            Segmented::new("ob-s-model", "Semantic Search model", labels, at)
                                .on_change(on_ix(cx, move |this, ix, cx| {
                                    if let Some(&i) = builds.get(ix) {
                                        this.set_semantic_model(i, cx)
                                    }
                                })),
                        )
                    })
                    .into_any_element(),
            );
        }
        v.push(
            baseline::row(
                vec![
                    (
                        Line::new(FONT_MONO, TOTAL, leading::ONE),
                        parts::mono(total, TOTAL, p.text_strong).into_any_element(),
                    ),
                    (
                        Line::new(FONT_UI, text::BASE, parts::FINE_LH),
                        fine("to download before any of this works", p).into_any_element(),
                    ),
                ],
                cx,
            )
            .gap(space::S3)
            .into_any_element(),
        );
        v
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
            2 => self.models_pane(cx, &p),
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
            &p,
        );
        (
            parts::main_col("ob-change", true, true, col),
            Some(foot.into_any_element()),
        )
    }
}
