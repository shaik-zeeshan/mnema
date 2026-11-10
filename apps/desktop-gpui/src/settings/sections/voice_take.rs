//! Settings › Your voice's take (`VoiceEnrollmentStore`): record 15 s with the
//! level meter and countdown, review it (play it back, Use this take /
//! Discard), then enroll. The clock is the shared `crate::voice::Take`.
//!
//! It lives in keyed element state under the Voice section, so it exists only
//! while that section renders: leaving the group or closing Settings drops it,
//! and the dropped [`MicLease`] ends a running take (as Cancel does). The
//! microphone is opened only by `start`.

use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Entity, Task, WeakEntity, Window};

use crate::settings::data::Backend;
use crate::settings::Settings;
use crate::timeline::audio::transport::Player;
use crate::voice::{self, Enrollment, Phase, Take};

/// Held while a take records; dropping it early ends the take and frees the mic.
struct MicLease {
    backend: Arc<dyn Backend>,
    armed: bool,
}

impl Drop for MicLease {
    fn drop(&mut self) {
        if self.armed {
            self.backend.cancel_clip();
        }
    }
}

pub struct VoiceTake {
    pub take: Take,
    /// Set for the render after a successful enroll (`justEnrolled`).
    pub just_enrolled: bool,
    pub playing: bool,
    pub play_failed: bool,
    backend: Arc<dyn Backend>,
    settings: WeakEntity<Settings>,
    mic: Option<MicLease>,
    task: Option<Task<()>>,
    player: Option<Box<dyn Player + Send>>,
    play_task: Option<Task<()>>,
}

/// This visit's take (created on first render of the Voice section).
pub fn state(s: &Settings, window: &mut Window, cx: &mut Context<Settings>) -> Entity<VoiceTake> {
    let backend = s.backend.clone();
    let settings = cx.entity().downgrade();
    window.use_keyed_state("voice-take", cx, move |_, _| VoiceTake {
        take: Take::default(),
        just_enrolled: false,
        playing: false,
        play_failed: false,
        backend,
        settings,
        mic: None,
        task: None,
        player: None,
        play_task: None,
    })
}

impl VoiceTake {
    /// `record`: open the mic for one 15 s take; the meter polls while it runs.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if matches!(self.take.phase, Phase::Review(_)) || !self.take.start(now) {
            return;
        }
        self.stop_playback();
        self.just_enrolled = false;
        self.play_failed = false;
        self.mic = Some(MicLease {
            backend: self.backend.clone(),
            armed: true,
        });
        let b = self.backend.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let b2 = b.clone();
            let clip =
                cx.background_spawn(async move { b2.record_clip(voice::CLIP.as_millis() as u64) });
            loop {
                cx.background_executor().timer(voice::LEVEL_POLL).await;
                let b2 = b.clone();
                let level = cx.background_spawn(async move { b2.mic_level() }).await;
                let now = cx.background_executor().now();
                let done = this
                    .update(cx, |t, cx| {
                        t.take.level(level.unwrap_or(0.));
                        let done = t.take.tick(now);
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            let clip = clip.await;
            let _ = this.update(cx, |t, cx| {
                // The recorder ended the take itself: nothing left to release.
                if let Some(mut lease) = t.mic.take() {
                    lease.armed = false;
                }
                match clip {
                    Ok(path) => t.take.review(path),
                    Err(e) => t.take.judged(Err(e)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Cancel a running take: the mic is released now.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if matches!(self.take.phase, Phase::Recording { .. }) {
            self.task = None;
            self.mic = None;
            self.take.cancel();
            cx.notify();
        }
    }

    /// `discard`: throw the reviewed take away without enrolling it.
    pub fn discard(&mut self, cx: &mut Context<Self>) {
        if matches!(self.take.phase, Phase::Review(_)) {
            self.stop_playback();
            self.take.cancel();
            cx.notify();
        }
    }

    /// `enroll`: submit the reviewed take. A stored voiceprint turned
    /// `recognize_saved_people` on backend-side; the draft mirrors it so the
    /// next autosave can't write the stale `false` back.
    pub fn use_take(&mut self, cx: &mut Context<Self>) {
        let Phase::Review(clip) = self.take.phase.clone() else {
            return;
        };
        self.stop_playback();
        self.take.phase = Phase::Judging;
        let b = self.backend.clone();
        let settings = self.settings.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { b.enroll(&clip) }).await;
            let enrolled = matches!(outcome, Ok(Enrollment::Enrolled));
            let _ = this.update(cx, |t, cx| {
                t.just_enrolled = enrolled;
                t.take.judged(outcome);
                cx.notify();
            });
            if enrolled {
                let _ = settings.update(cx, |s, cx| {
                    s.edit(|d| d.speaker_analysis.recognize_saved_people = true, cx);
                    s.refresh(cx);
                });
            }
        }));
        cx.notify();
    }

    /// Play the take / Stop.
    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.stop_playback();
            cx.notify();
            return;
        }
        let Phase::Review(clip) = self.take.phase.clone() else {
            return;
        };
        let b = self.backend.clone();
        self.play_task = Some(cx.spawn(async move |this, cx| {
            let opened = cx.background_spawn(async move { b.open_clip(&clip) }).await;
            let started = this.update(cx, |t, cx| {
                match opened {
                    Ok(mut player) => {
                        t.playing = player.play();
                        t.play_failed = !t.playing;
                        t.player = Some(player);
                    }
                    Err(_) => t.play_failed = true,
                }
                cx.notify();
                t.playing
            });
            if !started.unwrap_or(false) {
                return;
            }
            // The take ends on its own: watch for it like `onended`.
            loop {
                cx.background_executor().timer(voice::LEVEL_POLL).await;
                let still = this
                    .update(cx, |t, cx| {
                        let on = t.player.as_ref().is_some_and(|p| p.playing());
                        if !on {
                            t.player = None;
                            t.playing = false;
                            cx.notify();
                        }
                        on
                    })
                    .unwrap_or(false);
                if !still {
                    break;
                }
            }
        }));
    }

    fn stop_playback(&mut self) {
        self.play_task = None;
        if let Some(mut p) = self.player.take() {
            p.pause();
        }
        self.playing = false;
    }
}
