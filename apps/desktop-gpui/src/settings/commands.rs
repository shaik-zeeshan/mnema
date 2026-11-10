//! One-shot commands (buttons, not autosave): run a `live::*` call off the main
//! thread, show its outcome in the page notice, then re-read the loaders.
//! Without an `AppCore` (tests, goldens) nothing runs.

use std::sync::Arc;

use app_core::AppCore;
use capture_types::RecordingSettings;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, SharedString, Window};

use super::autosave::Unit;
use super::live::{self, Outcome};
use super::{icons, Settings, SettingsData};
use crate::theme::{self, radius, space, text, Palette};
use crate::ui::dialog::Modal;
use crate::ui::{notice, Button, Tone};

impl Settings {
    /// Run `job` in the background; `what` names it in the notice and the log.
    pub fn run_command(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Arc<AppCore>) -> Outcome + Send + 'static,
    ) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        if !self.ui.busy.insert(what) {
            return;
        }
        let work = cx.background_spawn(async move { job(&core) });
        cx.spawn(async move |this, cx| {
            let outcome = work.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.busy.remove(what);
                match outcome {
                    Ok(Some(text)) => s.ui.note = Some((true, what, text)),
                    Ok(None) => {}
                    Err(e) => {
                        // Backend error text: codes, paths and counts, never content.
                        log::warn!("settings: {what} failed: {e}");
                        s.ui.note = Some((false, what, e));
                    }
                }
                s.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Re-read every loader. Unsaved edits survive: only a clean page adopts
    /// the re-read settings as its drafts.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(false, cx);
    }

    /// Two landings: the local reads (mic, capture support, permissions…) at
    /// once, then the vault/keychain/network ones, so a slow or stuck key store
    /// never holds the rest back. Only the first answer may touch the drafts.
    pub(crate) fn load(&mut self, reset: bool, cx: &mut Context<Self>) {
        use crate::bridge::tokio::Tokio;
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        let c = core.clone();
        let local = Tokio::spawn(cx, async move { super::data::load_local(&c) });
        cx.spawn(async move |this, cx| {
            let Ok(local) = local.await else { return };
            let Ok(rest) = this.update(cx, |s, cx| {
                let first = local.clone();
                if reset {
                    s.set_data(first, cx);
                } else {
                    s.land(first, cx);
                }
                Tokio::spawn(
                    cx,
                    async move { super::data::load_slow(&core, local).await },
                )
            }) else {
                return;
            };
            if let Ok(full) = rest.await {
                // Not `land`: an edit or save since the first answer keeps its drafts.
                let _ = this.update(cx, |s, cx| {
                    s.data = full;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Adopt a re-read: the drafts follow it only on a clean page.
    pub(crate) fn land(&mut self, data: SettingsData, cx: &mut Context<Self>) {
        let dirty = !self.saving.is_empty() || Unit::ALL.iter().any(|u| self.gate(*u).dirty);
        if dirty {
            self.data = data;
            cx.notify();
        } else {
            self.set_data(data, cx);
        }
    }

    /// A settings answer from a one-shot command (the semantic switch, a wipe):
    /// both the saved copy and the draft take the fields it owns.
    pub fn adopt(&mut self, f: impl Fn(&mut RecordingSettings)) {
        if let Some(d) = self.draft.as_mut() {
            f(d);
        }
        if let Some(s) = self.saved.as_mut() {
            f(s);
        }
    }

    /// `update_semantic_search_settings { enabled }`: saved at once, reverted
    /// with the backend's reason when it fails.
    pub fn set_semantic_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.adopt(|d| d.semantic_search.enabled = on);
        self.run_command("Semantic search", cx, move |core| {
            live::set_semantic_enabled(core, on).map(|_| None)
        });
    }

    /// "Add app…": list the installed apps, then show the picker.
    pub fn open_app_picker(&mut self, cx: &mut Context<Self>) {
        if crate::chat::core(cx).is_none() {
            return;
        }
        self.ui.app_picker = Some(Vec::new());
        let task = crate::bridge::tokio::Tokio::spawn(cx, async move {
            app_core::native_capture::app_candidates::list_privacy_app_candidates().await
        });
        cx.spawn(async move |this, cx| {
            let listed = task.await;
            let _ = this.update(cx, |s, cx| {
                match listed {
                    Ok(Ok(apps)) if s.ui.app_picker.is_some() => {
                        s.ui.app_picker = Some(
                            apps.into_iter()
                                .map(|a| (a.bundle_id, a.display_name))
                                .collect(),
                        )
                    }
                    Ok(Err(e)) => {
                        log::warn!("settings: couldn't list installed apps: {e}");
                        s.ui.app_picker = None;
                        s.privacy_error = Some(e);
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

/// The last command's outcome, above the sections, until dismissed.
pub fn note(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Option<AnyElement> {
    let (ok, what, body) = s.ui.note.clone()?;
    let (tone, icon) = if ok {
        (Tone::Ok, icons::CHECK)
    } else {
        (Tone::Danger, icons::ALERT)
    };
    let title = if ok {
        what.to_string()
    } else {
        format!("{what} failed")
    };
    Some(
        notice::notice(
            tone,
            icon,
            title,
            Some(body.into()),
            vec![Button::new("settings-note-dismiss", "Dismiss")
                .ghost()
                .sm()
                .on_click(super::click(cx, |s, cx| {
                    s.ui.note = None;
                    cx.notify();
                }))
                .into_any_element()],
            p,
        )
        .into_any_element(),
    )
}

/// The "Add app…" picker: every installed app not excluded yet; a pick adds it.
pub fn app_picker(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> Option<AnyElement> {
    let apps = s.ui.app_picker.as_ref()?;
    let have: Vec<String> = s
        .d()
        .privacy
        .excluded_apps
        .iter()
        .map(|a| a.bundle_id.clone())
        .collect();
    let rows = apps
        .iter()
        .filter(|(bundle, _)| !have.contains(bundle))
        .map(|(bundle, name)| {
            let (bundle, name) = (bundle.clone(), name.clone());
            div()
                .id(SharedString::from(format!("excl-pick-{bundle}")))
                .role(gpui_kit::Role::Button)
                .flex()
                .items_center()
                .justify_between()
                .gap(space::S3)
                .px(space::S3)
                .py(px(7.))
                .rounded(radius::MD)
                .cursor_pointer()
                .hover(|st| st.bg(p.wash_strong))
                .child(
                    div()
                        .text_size(text::MD)
                        .text_color(p.text)
                        .child(name.clone()),
                )
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(text::SM)
                        .text_color(p.text_subtle)
                        .child(bundle.clone()),
                )
                .on_click(super::click(cx, move |s, cx| {
                    s.ui.app_picker = None;
                    let (bundle, name) = (bundle.clone(), name.clone());
                    s.privacy_command(
                        move |d| {
                            app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
                                d, &bundle, &name, true,
                            )
                        },
                        cx,
                    );
                }))
        })
        .collect::<Vec<_>>();
    let this = cx.entity().downgrade();
    let body = if apps.is_empty() {
        div()
            .text_color(p.text_muted)
            .child("Listing installed apps\u{2026}")
            .into_any_element()
    } else {
        div()
            .id("excl-pick-list")
            .flex()
            .flex_col()
            .max_h(px(360.))
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    };
    Some(
        Modal::new("excl-picker", "Never record an app")
            .lead(
                div()
                    .text_color(p.text_muted)
                    .child("Pick an app. Its windows and, if filtering is on, its audio stay out of recordings."),
            )
            .child(body)
            .on_close(move |_, cx| {
                let _ = this.update(cx, |s, cx| {
                    s.ui.app_picker = None;
                    cx.notify();
                });
            })
            .into_any_element(),
    )
}

/// Reads a text field once and clears it (keys and secrets never linger).
pub fn take_input(
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    window: &mut Window,
    cx: &mut Context<Settings>,
) -> String {
    let value = input.read(cx).value().trim().to_string();
    input.update(cx, |s, cx| s.set_value("", window, cx));
    value
}

/// `click` with the window (reading a text field needs it).
pub fn click_w(
    cx: &Context<Settings>,
    f: impl Fn(&mut Settings, &mut Window, &mut Context<Settings>) + 'static,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static {
    let this = cx.entity().downgrade();
    move |_, window, cx| {
        let _ = this.update(cx, |s, cx| f(s, window, cx));
    }
}

fn keep_days(p: &capture_types::RetentionPolicy) -> Option<u32> {
    use capture_types::RetentionPolicy as R;
    match p {
        R::Days7 => Some(7),
        R::Days14 => Some(14),
        R::Days30 => Some(30),
        R::Never => None,
    }
}

impl Settings {
    /// Retention: a shorter window first previews what it would delete and asks
    /// (the Svelte save's `preview_retention_cleanup` + confirm); Cancel keeps
    /// the saved window.
    pub fn set_retention(
        &mut self,
        policy: capture_types::RetentionPolicy,
        cx: &mut Context<Self>,
    ) {
        let saved = self.saved.as_ref().map(|s| s.retention_policy.clone());
        let shorter = match (saved.as_ref().map(keep_days), keep_days(&policy)) {
            (Some(None), Some(_)) => true,
            (Some(Some(was)), Some(now)) => now < was,
            _ => false,
        };
        let core = crate::chat::core(cx).filter(|_| shorter);
        let Some(core) = core else {
            self.edit(|d| d.retention_policy = policy, cx);
            return;
        };
        let asked = policy.clone();
        let work = cx.background_spawn(async move { live::retention_preview(&core, asked) });
        cx.spawn(async move |this, cx| {
            let preview = work.await;
            let _ = this.update(cx, |s, cx| match preview {
                Ok((segments, frames, audio)) => {
                    let message = format!(
                        "Retention will delete {segments} capture segment(s), {frames} frame row(s) and {audio} audio segment row(s) that are older than the new window. This cannot be undone."
                    );
                    super::confirm(
                        cx,
                        "Confirm retention cleanup",
                        &message,
                        "Continue",
                        move |s, cx| s.edit(|d| d.retention_policy = policy, cx),
                    );
                }
                Err(e) => {
                    log::warn!("settings: retention preview failed: {e}");
                    s.ui.note = Some((false, "Retention preview", e));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Events the page follows live: model download progress, and the changes
    /// that make its loaded answers stale.
    pub fn on_event(&mut self, event: &app_core::CoreEvent, cx: &mut Context<Self>) {
        use app_core::CoreEvent as E;
        let progress = match event {
            E::AudioTranscriptionModelDownloadProgress(p) => serde_json::to_value(p).ok(),
            E::SpeakerAnalysisModelDownloadProgress(p) => serde_json::to_value(p).ok(),
            E::OcrModelDownloadProgress(p) => serde_json::to_value(p).ok(),
            E::SemanticSearchModelDownloadProgress(p) => serde_json::to_value(p).ok(),
            E::UserContextChanged
            | E::McpAuthorizationChanged(_)
            | E::MicrophoneControllerChanged(_)
            | E::RecordingSettingsChanged(_) => {
                self.refresh(cx);
                return;
            }
            _ => return,
        };
        let Some(p) = progress else { return };
        let key = format!(
            "{}:{}",
            p["provider"].as_str().unwrap_or("local"),
            p["modelId"].as_str().unwrap_or_default()
        );
        let status = p["status"].as_str().unwrap_or_default().to_string();
        match status.as_str() {
            "completed" | "failed" | "cancelled" => {
                self.ui.downloads.remove(&key);
                if status == "failed" {
                    let why = p["message"]
                        .as_str()
                        .unwrap_or("the download failed")
                        .to_string();
                    self.ui.note = Some((false, "Model download", why));
                }
                self.refresh(cx);
            }
            _ => {
                let done = p["downloadedBytes"].as_u64().unwrap_or(0);
                self.ui
                    .downloads
                    .insert(key, (done, p["totalBytes"].as_u64(), status));
            }
        }
        cx.notify();
    }
}

/// Route a core event to the open Settings page (main's shell owns it).
pub fn on_event(event: &app_core::CoreEvent, cx: &mut gpui_kit::App) {
    if let Some(shell) = crate::window::shell(cx) {
        let settings = shell.read(cx).settings.clone();
        settings.update(cx, |s, cx| s.on_event(event, cx));
    }
}
