//! `ModelDownloadRow.svelte`: one "model status" row for OCR / Transcription /
//! Speakers / Semantic Search: the status word or Download (size) on the
//! right, a chip for failed/missing, error / warning lines and "Stored in".

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use crate::settings::data::{self, Family};
use crate::settings::model::{format_bytes, ModelView};
use crate::settings::row::{line, row, state_word};
use crate::settings::{icons, Settings};
use crate::theme::{text, Palette, FONT_MONO};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::{Button, Tone};

#[derive(Default)]
pub struct ModelRow {
    pub label: &'static str,
    pub desc: Option<String>,
    pub chip: Option<(String, ChipKind)>,
    pub status: Option<&'static str>,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub controls: Vec<AnyElement>,
    /// `(bytes, failed)`: show Download (or Retry after a failure).
    pub download: Option<(Option<u64>, bool)>,
    /// What Download starts.
    pub start: Option<(Family, String, String)>,
    pub folder: Option<String>,
}

impl ModelRow {
    pub fn build(self, id: &'static str, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
        let mut r = row(self.label).desc_opt(self.desc);
        if let Some((text_, kind)) = self.chip {
            r = r.extra(chip(text_, kind, p));
        }
        if let Some(st) = self.status {
            r = r.control(state_word(st, Some(Tone::Ok), p));
        }
        for c in self.controls {
            r = r.control(c);
        }
        if let Some((bytes, failed)) = self.download {
            let label = match bytes {
                Some(b) => format!("Download ({})", format_bytes(b as f64)),
                None => "Download".into(),
            };
            let start = self.start.clone();
            r = r.control(
                Button::new(SharedString::from(format!("{id}-download")), label)
                    .icon(if failed {
                        icons::RETRY
                    } else {
                        icons::DOWNLOAD
                    })
                    .on_click(crate::settings::click(cx, move |_, cx| {
                        if let (Some((family, provider, model)), Some(core)) =
                            (start.clone(), crate::chat::core(cx))
                        {
                            let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
                                data::start_download(&core, family, provider, model)
                            });
                            cx.spawn(async move |this, cx| {
                                let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
                                if let Err(e) = result {
                                    log::warn!("settings: model download didn't start: {e}");
                                }
                                let _ = this.update(cx, |s, cx| s.reload_all(cx));
                            })
                            .detach();
                        }
                    })),
            );
        }
        if let Some(e) = self.error {
            r = r.below(line(Tone::Danger, icons::ALERT, e, p));
        }
        if let Some(w) = self.warning {
            r = r.below(line(Tone::Warn, icons::INFO, w, p));
        }
        if let Some(path) = self.folder {
            let copy = path.clone();
            r = r.below(
                div()
                    .flex()
                    .w_full()
                    .items_start()
                    .gap(px(6.))
                    .text_size(text::BASE)
                    .text_color(p.text_subtle)
                    .child(
                        div()
                            .mt(px(3.))
                            .child(crate::ui::icon(icons::FOLDER, px(13.))),
                    )
                    .child(div().flex_none().child("Stored in"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .mt(px(1.))
                            .font_family(FONT_MONO)
                            .text_size(text::SM)
                            .child(path),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id}-copy-folder")), "Copy")
                            .ghost()
                            .sm()
                            .on_click(crate::settings::click(cx, move |_, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    copy.clone(),
                                ))
                            })),
                    ),
            );
        }
        r.build(p).into_any_element()
    }
}

/// The OCR / transcription / speaker rows' common description + chip rules.
pub fn status_row(
    label: &'static str,
    m: &ModelView,
    failed: bool,
    installed_word: &'static str,
) -> ModelRow {
    ModelRow {
        label,
        status: m.available.then_some(installed_word),
        chip: (!m.available).then(|| {
            if failed {
                ("failed".into(), ChipKind::Danger)
            } else if m.status == "incomplete" {
                ("incomplete".into(), ChipKind::Warn)
            } else {
                ("missing".into(), ChipKind::Warn)
            }
        }),
        error: m.failure.clone(),
        download: (!m.available && m.download_bytes.is_some())
            .then_some((m.download_bytes, failed)),
        ..Default::default()
    }
}

/// "Delete unused models" (OCR / transcription): asks first, then runs on tokio
/// and re-reads the model status.
pub fn delete_unused_row(
    id: &'static str,
    family: Family,
    desc: &'static str,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> AnyElement {
    let (title, message) = match family {
        Family::Ocr => ("Delete unused OCR models", "Delete unused OCR model files?"),
        _ => (
            "Delete unused transcription models",
            "Delete unused transcription model files?",
        ),
    };
    let on_click = crate::settings::click(cx, move |_, cx| {
        crate::settings::confirm(cx, title, message, "Delete", move |_, cx| {
            let Some(core) = crate::chat::core(cx) else {
                return;
            };
            let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
                let infra = core
                    .try_state::<app_core::AppInfraState>()
                    .ok_or_else(|| "app infrastructure isn't ready".to_string())?;
                match family {
                    Family::Ocr => app_core::ocr_models::delete_unused_ocr_models(&core, &infra)
                        .await
                        .map(|_| ()),
                    _ => {
                        app_core::audio_transcription_models::delete_unused_audio_transcription_models(
                            &core, &infra,
                        )
                        .await
                        .map(|_| ())
                    }
                }
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Err(e)) = job.await {
                    log::warn!("settings: delete unused models failed: {e}");
                }
                let _ = this.update(cx, |s, cx| s.reload_all(cx));
            })
            .detach();
        });
    });
    row("Delete unused models")
        .desc(desc)
        .control(Button::new(id, "Delete unused").ghost().on_click(on_click))
        .build(p)
        .into_any_element()
}
