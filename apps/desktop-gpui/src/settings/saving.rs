//! Autosave plumbing on `Settings`: which fields each unit owns, the dirty /
//! blocked gate, the one timer driving `autosave::Engine`, the off-thread
//! persist and what a save hands back (adopted, failed, retried, dismissed).

use std::collections::HashMap;
use std::time::Instant;

use capture_types::{MicrophoneControllerState, RecordingSettings};
use gpui_kit::prelude::*;
use gpui_kit::Context;
use serde_json::{json, Value};

use super::autosave::{self, Gate, SaveStatus, Unit};
use super::{live, Settings};

/// The fields one autosave unit owns (`apply_domain_patch_to_settings`).
pub fn domain_value(unit: Unit, s: &RecordingSettings) -> Value {
    match unit {
        Unit::CaptureSources => json!([
            s.capture_screen,
            s.capture_microphone,
            s.capture_system_audio
        ]),
        Unit::CaptureTiming => json!([s.segment_duration_seconds, s.auto_start]),
        Unit::Video => json!([s.screen_frame_rate, s.screen_resolution, s.video_bitrate]),
        Unit::Storage => json!([s.save_directory, s.retention_policy]),
        Unit::Display => json!([s.appearance, s.time_format, s.follow_timeline_live]),
        Unit::Metadata => json!(s.metadata),
        Unit::Inactivity => json!([
            s.pause_capture_on_inactivity,
            s.idle_timeout_seconds,
            s.microphone_activity_sensitivity,
            s.system_audio_activity_sensitivity,
            s.audio_speech_detection
        ]),
        Unit::Processing => json!([
            s.ocr,
            s.transcription,
            s.speaker_analysis,
            s.preview_cache_ttl_seconds
        ]),
        Unit::Developer => json!(s.developer_options_enabled),
        Unit::Access => json!(s.access),
        Unit::AiRuntime => json!(s.ai_runtime),
        Unit::UserContext => json!(s.user_context),
        Unit::Keyboard | Unit::Microphone => Value::Null,
    }
}

/// `recDomainSaveBlocked`: no source at all, unparseable video fields, no folder.
pub(super) fn domain_blocked(unit: Unit, s: &RecordingSettings, video_invalid: bool) -> bool {
    match unit {
        Unit::CaptureSources => {
            !s.capture_screen && !s.capture_microphone && !s.capture_system_audio
        }
        Unit::Video => video_invalid,
        Unit::Storage => s.save_directory.trim().is_empty(),
        _ => false,
    }
}

/// `min(30 s, 500 ms × 2^(n-1))` for the n-th failure in a row.
pub(super) fn retry_delay(failures: u32) -> std::time::Duration {
    let ms = 500u64.saturating_mul(1 << failures.saturating_sub(1).min(16));
    std::time::Duration::from_millis(ms.min(30_000))
}

enum Saved {
    Settings(RecordingSettings),
    Keyboard(Value),
    Mic(MicrophoneControllerState),
}

impl Settings {
    pub fn gate(&self, unit: Unit) -> Gate {
        // A failed recording domain waits for its backoff retry (or Retry /
        // Dismiss), not the next debounce: Svelte's `recSaveBackoffUntil` gate.
        let backing_off = !matches!(unit, Unit::Keyboard | Unit::Microphone)
            && self.failed.as_ref().is_some_and(|(u, _)| *u == unit);
        let saving = self.saving.contains(&unit) || backing_off;
        match unit {
            // `keyboardShortcutSaveBlocked`: an issue, or a row still listening.
            Unit::Keyboard => Gate {
                dirty: self.kb_saved.is_some() && self.kb_draft != self.kb_saved,
                blocked: self.ui.kb_listening.is_some() || !self.shortcut_issues().is_empty(),
                saving,
            },
            Unit::Microphone => Gate {
                dirty: self.mic_saved.is_some() && self.mic_draft != self.mic_saved,
                blocked: self.mic_draft.as_ref().is_some_and(|m| {
                    m.preference.mode == capture_types::MicrophonePreferenceMode::SpecificDevice
                        && m.preference.device_id.is_none()
                }),
                saving,
            },
            _ => match (&self.draft, &self.saved) {
                (Some(d), Some(s)) => Gate {
                    dirty: domain_value(unit, d) != domain_value(unit, s),
                    blocked: domain_blocked(unit, d, self.video_invalid),
                    saving,
                },
                _ => Gate::default(),
            },
        }
    }

    /// The single driver: re-arm every ready unit, then one timer for the earliest.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let gates: HashMap<Unit, Gate> = Unit::ALL.iter().map(|u| (*u, self.gate(*u))).collect();
        self.engine.tick(now, |u| gates[&u], self.privacy_in_flight);
        self.arm(cx);
    }

    pub(super) fn arm(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.engine.next_deadline() else {
            self.timer = None;
            return;
        };
        let wait = at.saturating_duration_since(cx.background_executor().now());
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| this.fire(cx));
        }));
    }

    fn fire(&mut self, cx: &mut Context<Self>) {
        let now: Instant = cx.background_executor().now();
        let gates: HashMap<Unit, Gate> = Unit::ALL.iter().map(|u| (*u, self.gate(*u))).collect();
        for unit in self.engine.due(now, |u| gates[&u], self.privacy_in_flight) {
            self.save(unit, cx);
        }
        self.arm(cx);
    }

    /// Run one unit's persist off the main thread.
    pub fn save(&mut self, unit: Unit, cx: &mut Context<Self>) {
        self.engine.take(unit);
        self.saving.insert(unit);
        let backend = self.backend.clone();
        let draft = self.draft.clone();
        let kb = self.kb_draft.clone();
        let mic = self.mic_draft.clone();
        let job = cx.background_spawn(async move {
            match unit {
                Unit::Keyboard => backend
                    .save_keyboard(&kb.unwrap_or_default())
                    .map(Saved::Keyboard),
                Unit::Microphone => match mic {
                    Some(m) => backend.save_mic(&m).map(Saved::Mic),
                    None => Err("no microphone state".into()),
                },
                _ => match draft {
                    Some(d) => backend.save_domain(unit, &d).map(Saved::Settings),
                    None => Err("settings not loaded".into()),
                },
            }
        });
        let sent = self.draft.as_ref().map(|d| domain_value(unit, d));
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| this.saved(unit, sent, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn saved(
        &mut self,
        unit: Unit,
        sent: Option<Value>,
        result: Result<Saved, String>,
        cx: &mut Context<Self>,
    ) {
        self.saving.remove(&unit);
        match result {
            Ok(Saved::Settings(s)) => {
                if self.saved.as_ref().map(|p| &p.appearance) != Some(&s.appearance) {
                    crate::theme::set_setting(s.appearance.clone(), cx);
                }
                // The backend may normalize what it stored (trimmed lists, a
                // cleared model pair). With no edit since the dispatch, the draft
                // adopts that, or it would read dirty and save again forever.
                if let Some(d) = self.draft.as_mut() {
                    let untouched = sent.as_ref() == Some(&domain_value(unit, d));
                    if untouched && domain_value(unit, d) != domain_value(unit, &s) {
                        if let Some(patch) = live::patch(unit, &s) {
                            let _ = app_core::recording_settings::apply_domain_patch_to_settings(
                                d, patch,
                            );
                            self.needs_sync = true;
                        }
                    }
                }
                let retention_landed = unit == Unit::Storage
                    && self.saved.as_ref().map(|p| &p.retention_policy)
                        != Some(&s.retention_policy);
                self.saved = Some(s);
                self.failed = None;
                if retention_landed && std::mem::take(&mut self.ui.cleanup_after_save) {
                    self.run_retention_cleanup(cx);
                }
            }
            Ok(Saved::Keyboard(k)) => {
                self.kb_saved = Some(k);
                self.failed = None;
            }
            Ok(Saved::Mic(m)) => {
                self.mic_saved = Some(m);
                self.failed = None;
            }
            Err(e) => {
                log::warn!("settings: {} failed: {e}", unit.command());
                self.failed = Some((unit, e));
                self.arm_retry(unit, cx);
            }
        }
        if self.failed.is_none() {
            self.fail_count = 0;
            self.retry_task = None;
        }
        // An edit made while it saved schedules a follow-up save.
        if self.failed.is_none() {
            self.tick(cx);
        }
        cx.notify();
    }

    /// `noteSaveFailure`: a failed recording domain retries on its own after
    /// a capped exponential backoff (500 ms doubling to 30 s), once per failure.
    fn arm_retry(&mut self, unit: Unit, cx: &mut Context<Self>) {
        if matches!(unit, Unit::Keyboard | Unit::Microphone) {
            return;
        }
        self.fail_count += 1;
        let delay = retry_delay(self.fail_count);
        self.retry_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |s, cx| {
                if s.failed.as_ref().is_some_and(|(u, _)| *u == unit) {
                    s.failed = None;
                    s.save(unit, cx);
                }
            });
        }));
    }

    /// The bar's Retry: re-run the failed unit now.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some((unit, _)) = self.failed.take() {
            self.fail_count = 0;
            self.retry_task = None;
            self.save(unit, cx);
        }
    }

    /// The notice's Dismiss. A recording domain puts its last saved value back
    /// (which also ends the retrying); shortcuts and the microphone only clear
    /// the message and try again on the next change.
    pub fn dismiss_failure(&mut self, cx: &mut Context<Self>) {
        let Some((unit, _)) = self.failed.take() else {
            return;
        };
        self.fail_count = 0;
        self.retry_task = None;
        if let (Some(d), Some(saved)) = (self.draft.as_mut(), self.saved.as_ref()) {
            if let Some(patch) = live::patch(unit, saved) {
                let _ = app_core::recording_settings::apply_domain_patch_to_settings(d, patch);
                self.needs_sync = true;
            }
        }
        self.tick(cx);
        cx.notify();
    }

    /// The draft shortcuts' conflicts (`shortcutIssues`).
    pub fn shortcut_issues(&self) -> HashMap<&'static str, String> {
        self.kb_draft
            .as_ref()
            .map(super::sections::shortcut_issues::issues)
            .unwrap_or_default()
    }

    pub fn save_status(&self) -> SaveStatus {
        // A shortcut issue blocks on its own; an in-progress key capture does not.
        let blocked = !self.shortcut_issues().is_empty()
            || Unit::ALL.iter().any(|u| {
                let g = self.gate(*u);
                *u != Unit::Keyboard && g.dirty && g.blocked
            });
        autosave::save_status(self.failed.is_some(), blocked, !self.saving.is_empty())
    }
}
