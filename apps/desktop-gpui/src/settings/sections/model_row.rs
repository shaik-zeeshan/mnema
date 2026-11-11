//! `ModelDownloadRow.svelte`: one "model status" row for OCR / Transcription /
//! Speakers / Semantic Search: the status word or Download (size) on the
//! right, a chip for failed/missing, error / warning lines and "Stored in".

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString};

use crate::settings::data::{self, Family};
use crate::settings::model::{format_bytes, ModelView};
use crate::settings::row::{line, row, state_word};
use crate::settings::{icons, Settings};
use crate::theme::{space, text, Palette, FONT_MONO};
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
    /// The "Model files" disclosure: `(term, value)` rows.
    pub files: Vec<(&'static str, String)>,
    /// What the row says while its download runs (`None` = no description).
    pub running_desc: Option<String>,
    /// The running download's Cancel (`None` = "Cancel download").
    pub cancel_label: Option<&'static str>,
    /// The model's name for "Downloading <name>".
    pub name: Option<String>,
    /// Extra lines under the copy (`ModelDownloadRow`'s `note`).
    pub notes: Vec<AnyElement>,
}

/// `downloadMeta`: the progress line's title and detail, and the bar's value.
pub fn download_meta(
    name: &str,
    status: &str,
    done: u64,
    total: Option<u64>,
) -> (String, String, Option<f32>) {
    let title = match status {
        "installing" => format!("Installing {name}"),
        "starting" => "Starting download".to_string(),
        _ => format!("Downloading {name}"),
    };
    match total.filter(|t| *t > 0) {
        None => (
            title,
            if done > 0 {
                format_bytes(done as f64)
            } else {
                String::new()
            },
            None,
        ),
        Some(t) => {
            let pct = ((done as f64 / t as f64) * 100.).round().min(100.);
            (
                title,
                format!(
                    "{} of {} \u{b7} {pct}%",
                    format_bytes(done as f64),
                    format_bytes(t as f64)
                ),
                Some((pct / 100.) as f32),
            )
        }
    }
}

/// The busy name while a family's download is being started.
fn starting_name(family: Family) -> &'static str {
    match family {
        Family::Ocr => "Starting OCR download",
        Family::Transcription => "Starting transcription download",
        Family::Speaker => "Starting speaker download",
        Family::Semantic => "Starting search model download",
    }
}

/// The words a failed download or model action leaves on its row.
pub fn failure_prefix(family: Family) -> &'static str {
    match family {
        Family::Ocr => "",
        Family::Speaker => "Speaker model action failed: ",
        Family::Transcription | Family::Semantic => "Download failed: ",
    }
}

/// `ModelDownloadRow`'s `info`: install path, licence, source, missing files.
pub fn model_files(m: &ModelView) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(v) = &m.install_path {
        out.push(("Install path", v.clone()));
    }
    if let Some(v) = &m.license {
        out.push(("Licence", v.clone()));
    }
    if let Some(v) = &m.source {
        out.push(("Source", v.clone()));
    }
    if !m.missing.is_empty() {
        out.push(("Missing", m.missing.join("\n")));
    }
    out
}

impl ModelRow {
    pub fn build(
        self,
        id: &'static str,
        s: &Settings,
        p: &Palette,
        cx: &mut Context<Settings>,
    ) -> AnyElement {
        let key = self
            .start
            .as_ref()
            .map(|(_, provider, model)| format!("{provider}:{model}"));
        let running = key.as_ref().and_then(|k| s.ui.downloads.get(k)).cloned();
        if let (Some((done, total, status)), Some((family, _, _))) = (&running, self.start.clone())
        {
            // The running layout (`ModelDownloadRow`'s `progress`): no chip, no
            // controls, just the progress and its Cancel.
            let r = row(self.label).desc_opt(self.running_desc).full();
            let name = self.name.unwrap_or_else(|| self.label.to_lowercase());
            let (title, detail, value) = download_meta(&name, status, *done, *total);
            let cancelling = s.ui.busy.contains("Cancel download");
            return r
                .control(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::S2)
                        .w_full()
                        .child(crate::ui::progress::progress_meta(title, detail, p))
                        .child(crate::ui::progress::progress(value, p))
                        .child(
                            div().flex().child(
                                Button::new(
                                    SharedString::from(format!("{id}-cancel")),
                                    if cancelling {
                                        "Cancelling"
                                    } else {
                                        self.cancel_label.unwrap_or("Cancel download")
                                    },
                                )
                                .sm()
                                .spinner(cancelling)
                                .disabled(cancelling)
                                .on_click(crate::settings::click(cx, move |s, cx| {
                                    s.run_command("Cancel download", cx, move |core| {
                                        data::cancel_download(core, family).map(|_| None)
                                    })
                                })),
                            ),
                        ),
                )
                .build(p)
                .into_any_element();
        }
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
        let family = self.start.as_ref().map(|(f, _, _)| *f);
        if let Some((bytes, failed)) = self.download {
            let label = match bytes {
                Some(b) => format!("Download ({})", format_bytes(b as f64)),
                None => "Download".into(),
            };
            let start = self.start.clone();
            let starting = family.is_some_and(|f| s.ui.busy.contains(starting_name(f)));
            r = r.control(
                Button::new(
                    SharedString::from(format!("{id}-download")),
                    if starting {
                        "Starting".to_string()
                    } else {
                        label
                    },
                )
                .when(!starting, |b| {
                    b.icon(if failed {
                        icons::RETRY
                    } else {
                        icons::DOWNLOAD
                    })
                })
                .spinner(starting)
                .disabled(starting || start.is_none())
                .on_click(crate::settings::click(cx, move |s, cx| {
                    if let (Some((family, provider, model)), Some(core)) =
                        (start.clone(), crate::chat::core(cx))
                    {
                        if !s.ui.busy.insert(starting_name(family)) {
                            return;
                        }
                        cx.notify();
                        let job = crate::bridge::tokio::Tokio::spawn(cx, async move {
                            data::start_download(&core, family, provider, model)
                        });
                        cx.spawn(async move |this, cx| {
                            let result = job.await.unwrap_or_else(|e| Err(e.to_string()));
                            let _ = this.update(cx, |s, cx| {
                                s.ui.busy.remove(starting_name(family));
                                s.ui.model_notes.remove(&family);
                                if let Err(e) = result {
                                    log::warn!("settings: model download didn't start: {e}");
                                    let words = format!("{}{e}", failure_prefix(family));
                                    s.ui.model_notes.insert(family, (false, words));
                                }
                                s.refresh(cx)
                            });
                        })
                        .detach();
                    }
                })),
            );
        }
        // A failed download / model action on this row speaks over the status's own.
        let note = family.and_then(|f| s.ui.model_notes.get(&f)).cloned();
        let error = match &note {
            Some((false, words)) => Some(words.clone()),
            _ => self.error,
        };
        if let Some(e) = error {
            r = r.below(line(Tone::Danger, icons::ALERT, e, p));
        }
        if let Some((true, words)) = note {
            r = r.below(crate::settings::row::hint(words, p));
        }
        if let Some(w) = self.warning {
            r = r.below(line(Tone::Warn, icons::INFO, w, p));
        }
        for n in self.notes {
            r = r.below(n);
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
                        // `ModelDownloadRow.svelte`: "Copied" for 1.5 s.
                        crate::ui::CopyButton::label(
                            SharedString::from(format!("{id}-copy-folder")),
                            "Copy",
                            copy,
                        )
                        .ghost()
                        .revert(std::time::Duration::from_millis(1500)),
                    ),
            );
        }
        if !self.files.is_empty() {
            let open = s.ui.files_open.contains(id);
            r = r.below(files(id, open, self.files, p, cx));
        }
        r.build(p).into_any_element()
    }
}

/// `<details class="mdr-info"><summary>Model files</summary><dl>…`.
fn files(
    id: &'static str,
    open: bool,
    rows: Vec<(&'static str, String)>,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> gpui_kit::Div {
    let hover = p.text;
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(space::S2)
        .text_size(text::SM)
        .child(
            div()
                .id(SharedString::from(format!("{id}-files")))
                .role(gpui_kit::Role::Button)
                .aria_expanded(open)
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .text_color(p.text_subtle)
                .hover(move |s| s.text_color(hover))
                .child(crate::ui::icon(
                    if open {
                        IconName::ChevronDown.path()
                    } else {
                        IconName::ChevronRight.path()
                    },
                    px(12.),
                ))
                .child("Model files")
                .on_click(crate::settings::click(cx, move |s, cx| {
                    if !s.ui.files_open.remove(id) {
                        s.ui.files_open.insert(id);
                    }
                    cx.notify();
                })),
        )
        .when(open, |d| {
            d.children(rows.into_iter().map(|(term, value)| {
                div()
                    .flex()
                    .gap(space::S3)
                    .child(
                        div()
                            .w(px(84.))
                            .flex_none()
                            .text_color(p.text_subtle)
                            .child(term),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(p.text_muted)
                            .when(term != "Licence", |d| d.font_family(FONT_MONO))
                            .children(value.split('\n').map(|l| div().child(l.to_string()))),
                    )
            }))
        })
}

/// A family's row while it has no status: the read's error (`prefix` + the
/// backend's words), "Checking installed … models…" while the reads run, else
/// the "no status" warning (`ModelDownloadRow`'s three early branches).
pub fn no_status_row(
    s: &Settings,
    family: Family,
    label: &'static str,
    prefix: &str,
    checking: &str,
    none: &str,
) -> ModelRow {
    if let Some(e) = s.data.model_errors.get(&family) {
        return ModelRow {
            label,
            error: Some(format!("{prefix}{e}")),
            ..Default::default()
        };
    }
    let loading = s.ui.models_loading;
    ModelRow {
        label,
        desc: loading.then(|| checking.to_string()),
        warning: (!loading).then(|| none.to_string()),
        ..Default::default()
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
        files: model_files(m),
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
                let failed = job.await.unwrap_or_else(|e| Err(e.to_string())).err();
                let _ = this.update(cx, |s, cx| {
                    if let Some(e) = failed {
                        log::warn!("settings: delete unused models failed: {e}");
                        s.ui.note = Some((false, "Delete unused models", e));
                    }
                    s.refresh(cx)
                });
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
