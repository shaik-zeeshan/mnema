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

enum Saved {
    Settings(RecordingSettings),
    Keyboard(Value),
    Mic(MicrophoneControllerState),
}

impl Settings {
    pub fn gate(&self, unit: Unit) -> Gate {
        let saving = self.saving.contains(&unit);
        match unit {
            Unit::Keyboard => Gate {
                dirty: self.kb_saved.is_some() && self.kb_draft != self.kb_saved,
                blocked: false,
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
                self.saved = Some(s);
                self.failed = None;
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
            }
        }
        // An edit made while it saved schedules a follow-up save.
        if self.failed.is_none() {
            self.tick(cx);
        }
        cx.notify();
    }

    /// The bar's Retry: re-run the failed unit now.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some((unit, _)) = self.failed.take() {
            self.save(unit, cx);
        }
    }

    /// The notice's Dismiss: put the last saved value back.
    pub fn dismiss_failure(&mut self, cx: &mut Context<Self>) {
        if self.failed.take().is_some() {
            self.draft = self.saved.clone();
            self.kb_draft = self.kb_saved.clone();
            self.mic_draft = self.mic_saved.clone();
            self.tick(cx);
            cx.notify();
        }
    }

    pub fn save_status(&self) -> SaveStatus {
        let blocked = Unit::ALL.iter().any(|u| {
            let g = self.gate(*u);
            g.dirty && g.blocked
        });
        autosave::save_status(self.failed.is_some(), blocked, !self.saving.is_empty())
    }
}
