//! One-shot commands (buttons, not autosave): run a `live::*` call off the main
//! thread, show its outcome in the page notice, then re-read the loaders.
//! Without an `AppCore` (tests, goldens) nothing runs.

use std::sync::Arc;

use app_core::AppCore;
use capture_types::RecordingSettings;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Task, Window};

use super::autosave::Unit;
use super::live::{self, Outcome};
use super::pending::Pending;
use super::{icons, Settings, SettingsData};
use crate::theme::Palette;
use crate::ui::{notice, Button, Tone};

impl Settings {
    /// Run `job` in the background; `what` names it in the notice and the log.
    pub fn run_command(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Arc<AppCore>) -> Outcome + Send + 'static,
    ) {
        self.run_command_with(what, cx, job, |_, _| false)
    }

    /// `run_command` whose outcome a row shows itself: `inline` takes it and
    /// answers true when it did (then no page note); the re-read still runs.
    pub fn run_command_with(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Arc<AppCore>) -> Outcome + Send + 'static,
        inline: impl FnOnce(&mut Self, &Outcome) -> bool + 'static,
    ) {
        let Some(core) = self.command_core(what, cx) else {
            return;
        };
        let work = cx.background_spawn(async move { job(&core) });
        self.land_command(what, work, inline, cx).detach();
    }

    /// `run_command_with` for a slow network call: `job` answers at once with
    /// a [`Pending`] that runs on tokio, so no thread waits on it, and dropping
    /// Settings cancels it.
    pub fn run_slow_command_with(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Arc<AppCore>) -> Pending<Option<String>>,
        inline: impl FnOnce(&mut Self, &Outcome) -> bool + 'static,
    ) {
        let Some(core) = self.command_core(what, cx) else {
            return;
        };
        let work = job(&core).spawn(cx);
        let task = self.land_command(what, work, inline, cx);
        self.jobs.insert(what.into(), task);
    }

    /// The core a command runs on, once `what` is marked busy (`None`: no
    /// core, or `what` is already running).
    fn command_core(&mut self, what: &'static str, cx: &Context<Self>) -> Option<Arc<AppCore>> {
        let core = crate::chat::core(cx)?;
        self.ui.busy.insert(what).then_some(core)
    }

    /// Show a command's outcome when `work` lands, then re-read the loaders.
    fn land_command(
        &mut self,
        what: &'static str,
        work: Task<Outcome>,
        inline: impl FnOnce(&mut Self, &Outcome) -> bool + 'static,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = work.await;
            let _ = this.update(cx, |s, cx| {
                s.ui.busy.remove(what);
                if inline(s, &outcome) {
                    if let Err(e) = &outcome {
                        log::warn!("settings: {what} failed: {e}");
                    }
                    s.refresh(cx);
                    cx.notify();
                    return;
                }
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
        // A core with no recording settings (a bare test core) has nothing to read.
        if core
            .try_state::<app_core::recording_settings::RecordingSettingsState>()
            .is_none()
        {
            return;
        }
        let c = core.clone();
        self.ui.models_loading = true;
        cx.notify();
        // Blocking work (it walks the model folders): tokio's blocking pool, not a worker.
        let local = Tokio::spawn(cx, async move {
            let core = c.clone();
            c.spawn_blocking(move || super::data::load_local(&core))
                .await
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(local)) = local.await else { return };
            let Ok(rest) = this.update(cx, |s, cx| {
                s.ui.models_loading = false;
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
            // Unsaved edits stay; every field they didn't touch takes the re-read
            // value, so the next save never writes a stale one back.
            self.rebase(&data);
            self.data = data;
            self.tick(cx);
            cx.notify();
        } else {
            self.set_data(data, cx);
        }
    }

    /// A re-read under unsaved edits: each draft keeps the fields the user
    /// changed and takes the new value everywhere else; then saved moves on.
    fn rebase(&mut self, data: &SettingsData) {
        if let Some(new) = &data.settings {
            let merged = merge(&self.saved, &self.draft, new);
            self.needs_sync |= merged != self.draft;
            self.draft = merged;
            self.saved = Some(new.clone());
        }
        if let Some(new) = &data.keyboard {
            self.kb_draft = merge(&self.kb_saved, &self.kb_draft, new);
            self.kb_saved = Some(new.clone());
        }
        if let Some(new) = &data.mic {
            self.mic_draft = merge(&self.mic_saved, &self.mic_draft, new);
            self.mic_saved = Some(new.clone());
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
        // `setSemanticSearchEnabled`: a refusal flips the switch back and says why on the row.
        self.adopt(|d| d.semantic_search.enabled = on);
        self.ui.model_notes.remove(&super::data::Family::Semantic);
        self.run_command_with(
            "Semantic search",
            cx,
            move |core| live::set_semantic_enabled(core, on).map(|_| None),
            move |s, outcome| {
                let Err(e) = outcome else { return false };
                s.adopt(|d| d.semantic_search.enabled = !on);
                let words = format!("Model status failed: {e}");
                s.ui.model_notes
                    .insert(super::data::Family::Semantic, (false, words));
                true
            },
        );
    }

    /// Window focus (`+page.svelte`'s and the privacy picker's focus handlers):
    /// re-read what System Settings may have changed, and an open picker's apps.
    /// The drafts are never touched, so unsaved edits stay.
    pub fn on_window_focus(&mut self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        let task = cx.background_spawn(async move { backend.recheck_access() });
        cx.spawn(async move |this, cx| {
            let Some(d) = task.await else { return };
            let _ = this.update(cx, |s, cx| {
                s.data.permissions = d.permissions;
                s.data.system_audio_blocked = d.system_audio_blocked;
                s.data.gecko = d.gecko;
                if d.complete {
                    s.data.grants = d.grants;
                    s.data.history = d.history;
                }
                cx.notify();
            });
        })
        .detach();
        if self
            .ui
            .app_picker
            .as_ref()
            .is_some_and(|apps| !apps.is_empty())
        {
            self.list_apps(cx);
        }
    }

    /// "Add app…": list the installed apps, then show the picker.
    pub fn open_app_picker(&mut self, cx: &mut Context<Self>) {
        if crate::chat::core(cx).is_none() {
            return;
        }
        self.ui.app_picker = Some(Vec::new());
        self.ui.app_highlight = 0;
        self.ui.clear_app_search = true;
        self.list_apps(cx);
    }

    fn list_apps(&mut self, cx: &mut Context<Self>) {
        if crate::chat::core(cx).is_none() {
            return;
        }
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
                                .map(|a| (a.bundle_id, a.display_name, a.running))
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
    // These speak inline in their own rows (User Context).
    use super::sections::user_context::{DERIVE, WIPE};
    if what == DERIVE || (what == WIPE && !ok) {
        return None;
    }
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

/// Apps still offered: installed, not excluded yet, matching the search
/// (`filteredPrivacyAppCandidates`: the first 12).
pub fn candidates<'a>(s: &'a Settings, query: &str) -> Vec<&'a (String, String, bool)> {
    let have: Vec<String> = s
        .d()
        .privacy
        .excluded_apps
        .iter()
        .map(|a| a.bundle_id.clone())
        .collect();
    let q = query.trim().to_lowercase();
    s.ui.app_picker
        .iter()
        .flatten()
        .filter(|(bundle, _, _)| !have.contains(bundle))
        .filter(|(bundle, name, _)| {
            q.is_empty() || name.to_lowercase().contains(&q) || bundle.to_lowercase().contains(&q)
        })
        .take(12)
        .collect()
}

/// Enter picks the highlighted match (`onComboboxKeydown`).
pub fn highlighted_app(s: &Settings, query: &str) -> Option<(String, String)> {
    candidates(s, query)
        .get(s.ui.app_highlight)
        .map(|(b, n, _)| (b.clone(), n.clone()))
}

/// Pick an app: the picker closes and the exclusion is added.
pub fn exclude_app(s: &mut Settings, bundle: String, name: String, cx: &mut Context<Settings>) {
    s.ui.app_picker = None;
    s.privacy_command(
        move |d| {
            app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
                d, &bundle, &name, true,
            )
        },
        cx,
    );
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
                Ok((segments, frames, audio, cutoff)) => {
                    let message = format!(
                        "Retention will delete {frames} frame row(s), {audio} audio segment row(s), and {segments} capture segment(s) before {}. Continue?",
                        cutoff.as_deref().unwrap_or("the cutoff")
                    );
                    super::confirm(
                        cx,
                        "Confirm retention cleanup",
                        &message,
                        "Continue",
                        move |s, cx| {
                            // The tightened window's cleanup runs once the save lands.
                            s.ui.cleanup_after_save = true;
                            s.edit(|d| d.retention_policy = policy, cx)
                        },
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
        use super::data::Family;
        use app_core::CoreEvent as E;
        let (family, progress) = match event {
            E::AudioTranscriptionModelDownloadProgress(p) => {
                (Family::Transcription, serde_json::to_value(p).ok())
            }
            E::SpeakerAnalysisModelDownloadProgress(p) => {
                (Family::Speaker, serde_json::to_value(p).ok())
            }
            E::OcrModelDownloadProgress(p) => (Family::Ocr, serde_json::to_value(p).ok()),
            E::SemanticSearchModelDownloadProgress(p) => {
                (Family::Semantic, serde_json::to_value(p).ok())
            }
            E::ChatgptLoginUpdate(v) => {
                self.on_chatgpt_update(v, cx);
                return;
            }
            // Retention and Delete Recent Capture change what is on disk.
            E::TimelineDataChanged(p)
                if matches!(p.reason.as_str(), "retention" | "delete_recent_capture") =>
            {
                self.refresh(cx);
                return;
            }
            E::UserContextChanged
            | E::McpAuthorizationChanged(_)
            | E::MicrophoneControllerChanged(_)
            | E::RecordingSettingsChanged(_)
            | E::RecordingSettingsDomainChanged(_) => {
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
                    let why = p["message"].as_str().unwrap_or("Download failed.");
                    let words = format!("{}{why}", super::sections::model_row_failure(family));
                    self.ui.model_notes.insert(family, (false, words));
                }
                self.refresh(cx);
            }
            _ => {
                self.ui.model_notes.remove(&family);
                let done = p["downloadedBytes"].as_u64().unwrap_or(0);
                self.ui
                    .downloads
                    .insert(key, (done, p["totalBytes"].as_u64(), status));
            }
        }
        cx.notify();
    }
}

/// Three-way merge through JSON: where the draft still equals what was saved,
/// take the new value; objects merge per key; any other edit is the user's and stays.
fn merge<T: serde::Serialize + serde::de::DeserializeOwned + Clone>(
    saved: &Option<T>,
    draft: &Option<T>,
    new: &T,
) -> Option<T> {
    let (Some(saved), Some(draft)) = (saved, draft) else {
        return Some(new.clone());
    };
    let json = |v: &T| serde_json::to_value(v).ok();
    let merged = match (json(saved), json(draft), json(new)) {
        (Some(s), Some(d), Some(n)) => serde_json::from_value(merge_value(&s, &d, n)).ok(),
        _ => None,
    };
    Some(merged.unwrap_or_else(|| draft.clone()))
}

fn merge_value(
    saved: &serde_json::Value,
    draft: &serde_json::Value,
    new: serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;
    if draft == saved {
        return new;
    }
    match (saved, draft, new) {
        (Value::Object(s), Value::Object(d), Value::Object(mut n)) => {
            for (k, dv) in d {
                let sv = s.get(k).unwrap_or(&Value::Null);
                match n.remove(k) {
                    Some(nv) => {
                        n.insert(k.clone(), merge_value(sv, dv, nv));
                    }
                    None if dv != sv => {
                        n.insert(k.clone(), dv.clone());
                    }
                    None => {}
                }
            }
            Value::Object(n)
        }
        _ => draft.clone(),
    }
}

/// Route a core event to the open Settings page (main's shell owns it).
/// The native confirm (`Host::confirm`), then `then` on OK. Without an
/// `AppCore` (tests, goldens) nothing is asked and nothing runs.
pub fn confirm_as(
    cx: &mut Context<Settings>,
    title: &str,
    message: &str,
    ok_label: &str,
    cancel_label: &str,
    then: impl FnOnce(&mut Settings, &mut Context<Settings>) + 'static,
) {
    confirm_then(
        cx,
        title,
        message,
        ok_label,
        cancel_label,
        move |s, ok, cx| {
            if ok {
                then(s, cx)
            }
        },
    );
}

/// [`confirm_as`], with the answer either way (`false` also when the dialog
/// itself failed: a failed dialog never deletes anything).
pub fn confirm_then(
    cx: &mut Context<Settings>,
    title: &str,
    message: &str,
    ok_label: &str,
    cancel_label: &str,
    then: impl FnOnce(&mut Settings, bool, &mut Context<Settings>) + 'static,
) {
    let Some(core) = crate::chat::core(cx) else {
        return;
    };
    let answer = core.host().confirm(app_core::ConfirmDialog {
        title: Some(title.into()),
        message: message.into(),
        kind: app_core::DialogKind::Warning,
        ok_label: ok_label.into(),
        cancel_label: cancel_label.into(),
    });
    cx.spawn(async move |this, cx| {
        let ok = answer.await == Ok(true);
        let _ = this.update(cx, |s, cx| then(s, ok, cx));
    })
    .detach();
}

pub fn on_event(event: &app_core::CoreEvent, cx: &mut gpui_kit::App) {
    if let Some(shell) = crate::window::shell(cx) {
        let settings = shell.read(cx).settings.clone();
        settings.update(cx, |s, cx| s.on_event(event, cx));
    }
}
