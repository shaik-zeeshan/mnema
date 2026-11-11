//! The shell's capture controls (`capture-controls.svelte.ts`): Record / Stop /
//! Pause / Resume, the source toggles, the bell's actions, and the refreshes
//! that keep the bar live. Every command goes through app-core's capture
//! control (`native::toggle_recording` / `toggle_pause`), off the main thread.

use std::sync::Arc;
use std::time::Duration;

use app_core::AppCore;
use gpui_kit::{AppContext as _, Context};

use super::model::{RowAction, StatusFlag, Tone};
use super::{data, model, Overlay, Shell};
use crate::bridge::tokio::Tokio;
use crate::native::{self, CaptureResult};

/// `subscribeRuntimeSources` / `subscribeCaptureHealth` / the popover's poll.
pub const LIVE_POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Start,
    Stop,
    Pause,
}

impl Shell {
    /// Record (the status bar, the Timeline's empty state, Insights, "Retry").
    pub fn start_recording(&mut self, cx: &mut Context<Self>) {
        let s = &self.model.status;
        if s.running || s.loading_start || s.read_only {
            return;
        }
        self.run(Command::Start, cx);
    }

    pub fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.model.status.running {
            self.run(Command::Stop, cx);
        }
    }

    /// Pause, or resume a user pause. A low-disk suspension resumes on its own.
    pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let s = &self.model.status;
        if s.running && (s.user_paused || !s.low_disk) {
            self.run(Command::Pause, cx);
        }
    }

    fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        if self.capture_busy {
            return;
        }
        self.capture_busy = true;
        if command == Command::Start {
            self.start_error = None;
            self.model.status.start_error = None;
            self.model.status.loading_start = true;
            self.timeline.update(cx, |t, _| t.capture.starting = true);
        }
        self.model.status.loading_stop = command == Command::Stop;
        cx.notify();
        let (tx, rx) = async_channel::bounded::<CaptureResult>(1);
        let worker = core.clone();
        // Capture start/stop opens and flushes streams: never on the main thread.
        std::thread::spawn(move || {
            let result = match command {
                // Both guarded on the live session, so a stale bar can't flip it the wrong way.
                Command::Start | Command::Stop => {
                    let running =
                        app_core::native_capture::control::current_native_capture_session(&worker)
                            .is_running;
                    if running == (command == Command::Stop) {
                        native::toggle_recording(&worker, "status-bar-window")
                    } else {
                        Ok(())
                    }
                }
                Command::Pause => native::toggle_pause(&worker),
            };
            let _ = tx.send_blocking(result);
        });
        cx.spawn(async move |this, cx| {
            let result = rx.recv().await;
            let _ = this.update(cx, |this, cx| {
                this.capture_busy = false;
                this.model.status.loading_start = false;
                this.model.status.loading_stop = false;
                this.timeline.update(cx, |t, _| t.capture.starting = false);
                if let Ok(Err((title, e))) = &result {
                    if command == Command::Start {
                        this.start_error = Some(e.message.clone());
                        this.model.status.start_error = this.start_error.clone();
                    }
                    // Never silent: a bell row, and the "Recording error" dialog.
                    native::report_capture_error(&core, title, e, true);
                }
                cx.notify();
                // The session event reloads too; this covers a failure, which emits none.
                this.reload(cx);
            });
        })
        .detach();
    }

    /// `setSourceSelected`: save one source for the next recording.
    pub(crate) fn save_source(&mut self, i: usize, selected: bool, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        let mut request = capture_types::UpdateCaptureSourceSettingsRequest::default();
        *[
            &mut request.capture_screen,
            &mut request.capture_microphone,
            &mut request.capture_system_audio,
        ][i] = Some(selected);
        let task = Tokio::spawn(cx, {
            let core = core.clone();
            async move {
                app_core::native_capture::settings_update::update_capture_source_settings(
                    request, &core,
                )
                .await
            }
        });
        cx.spawn(async move |this, cx| {
            let Ok(Err(e)) = task.await else { return };
            // Locked (a start is reading them): the saved settings stand, so the
            // reload below reverts the toggle. Anything else is reported.
            if e.code != "capture_sources_locked" {
                native::report_capture_error(&core, "Recording error", &e, true);
            }
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
    }

    /// The status flag's click (`runFlag`).
    pub(super) fn run_flag(&mut self, flag: &StatusFlag, cx: &mut Context<Self>) {
        let m = &self.model.status;
        if m.running {
            // "disk almost full" (the only info flag) opens the popover; the
            // others ("retrying" / "resumes on its own") are status only.
            if flag.tone == Tone::Info {
                self.toggle_overlay(Overlay::Status, cx);
            }
            return;
        }
        if m.start_error.is_some() {
            self.start_recording(cx);
        } else if m.read_only {
            cx.defer(|cx| crate::window::open_settings(Some("license"), cx));
        } else if m.stopped_reason.as_deref() == Some("disk full") {
            cx.defer(|cx| crate::window::open_settings(Some("storage"), cx));
        } else if model::screen_permission_missing(m) {
            // macOS won't re-prompt once denied: open the pane + float the helper.
            let Some(core) = crate::chat::core(cx) else {
                return;
            };
            let _ = core
                .host()
                .open_window(app_core::WindowRequest::PermissionHelper {
                    kind: "screen".into(),
                });
            let _ = flag;
        }
    }

    /// A bell row's button: open its Settings tab, or Start from the reminder.
    pub(super) fn run_notification(&mut self, i: usize, snooze: bool, cx: &mut Context<Self>) {
        let Some(row) = self.model.notifications.get(i) else {
            return;
        };
        match &row.kind {
            RowAction::None => {}
            RowAction::OpenSettings(tab) => {
                let tab = tab.clone();
                self.overlay = None;
                cx.defer(move |cx| crate::window::open_settings(Some(&tab), cx));
            }
            // `snooze_not_recording_reminder` until the next local midnight.
            RowAction::Reminder if snooze => {
                let Some(core) = crate::chat::core(cx) else {
                    return;
                };
                app_core::native_capture::not_recording_reminder::snooze_not_recording_reminder(
                    native::sys::next_local_midnight_ms(),
                    &core,
                );
            }
            // `startFromReminder`: the shared start, which reports its own failure;
            // the lifecycle withdraws the reminder once recording runs.
            RowAction::Reminder => self.start_recording(cx),
        }
        cx.notify();
    }

    /// `clear_app_notification` (one row) / `clear_app_notifications`.
    pub(crate) fn clear_notifications(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        use app_core::native_capture::app_notifications as bell;
        // Both emit `AppNotificationsChanged`, which reloads the shell.
        match id {
            Some(id) => drop(bell::clear_app_notification(id, &core)),
            None => drop(bell::clear_app_notifications(&core)),
        }
        self.model.notifications = data::notifications(&core);
        cx.notify();
    }

    /// Window focus (and first show): re-read the bar and stamp the local UTC
    /// offset, so User Context labels Activity times in the user's clock.
    pub fn on_focus(&mut self, cx: &mut Context<Self>) {
        // Settings re-checks what System Settings may have changed meanwhile.
        if self.surface == super::Surface::Settings {
            self.settings.update(cx, |s, cx| s.on_window_focus(cx));
        }
        let Some(core) = crate::chat::core(cx) else {
            return;
        };
        stamp_local_offset(&core, cx);
        self.reload(cx);
    }

    /// One 60 s beat (`Overview.svelte`'s / `DayTimeline.svelte`'s interval): a
    /// visible Insights refreshes its tracked time, and every 5th beat re-reads
    /// the bar's "today" (`StatusBar.svelte`'s 5 min). Skipped while nothing of
    /// the main window can be seen.
    pub(super) fn start_beat(cx: &mut Context<Self>) -> gpui_kit::Task<()> {
        cx.spawn(async move |this, cx| {
            let mut beats = 0u64;
            loop {
                cx.background_executor().timer(INSIGHTS_BEAT).await;
                beats += 1;
                let today = beats % (TODAY_BEAT.as_secs() / INSIGHTS_BEAT.as_secs()) == 0;
                let alive = this.update(cx, |this, cx| {
                    if crate::window::render_idle(cx) {
                        return;
                    }
                    if this.surface == super::Surface::Insights
                        && this.insights.update(cx, |i, cx| i.tick(cx))
                    {
                        this.beat_reads.1 += 1;
                    }
                    if today {
                        this.beat_reads.0 += 1;
                        this.reload(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    /// Re-read the model once this update is over (`reload_shell` reads the shell).
    fn reload(&mut self, cx: &mut Context<Self>) {
        let shell = cx.entity();
        cx.defer(move |cx| crate::bridge::events::reload_shell(&shell, cx));
    }

    /// The live feed is wanted while recording (the bar) or while the status
    /// popover is open (it reads health and the backlog even when stopped).
    fn live_needed(&self) -> bool {
        self.model.status.running || self.overlay == Some(Overlay::Status)
    }

    /// While the feed is wanted, re-read the session, its sources and capture
    /// health now and then every 2 s, plus the backlog while the popover is
    /// open, off the main thread. Skipped while nothing can render; stops (and
    /// forgets the health reading) once neither the bar nor the popover needs it.
    pub(super) fn arm_live_poll(&mut self, cx: &mut Context<Self>) {
        if !self.live_needed() {
            if self.live.take().is_some() {
                self.model.status.health = None;
            }
            return;
        }
        if self.live.is_some() {
            return;
        }
        self.live = Some(cx.spawn(async move |this, cx| loop {
            let polled = this.update(cx, |this, cx| {
                if !this.live_needed() {
                    // At most one idle wake after the popover closes; it reads nothing.
                    this.live = None;
                    this.model.status.health = None;
                    return None;
                }
                if crate::window::render_idle(cx) {
                    return Some(None);
                }
                let Some(core) = crate::chat::core(cx) else {
                    return Some(None);
                };
                let mut status = this.model.status.clone();
                let open = this.overlay == Some(Overlay::Status);
                Some(Some(cx.background_spawn(async move {
                    data::refresh_live(&core, &mut status);
                    status.health = data::health(&core);
                    let backlog = open.then(|| data::backlog(&core, status.deepgram_transcription));
                    (status, backlog)
                })))
            });
            match polled {
                Ok(Some(Some(task))) => {
                    let (status, backlog) = task.await;
                    let applied = this.update(cx, |this, cx| {
                        // Released while in flight: don't resurrect a stale reading.
                        if !this.live_needed() {
                            return;
                        }
                        let mut model = this.model.clone();
                        model.status = status;
                        if this.overlay == Some(Overlay::Status) {
                            model.backlog = backlog.or(model.backlog);
                        }
                        // `set_model` keeps the last health when this read failed.
                        if model != this.model {
                            this.set_model(model, cx);
                        }
                    });
                    if applied.is_err() {
                        break;
                    }
                }
                Ok(Some(None)) => {}
                Ok(None) | Err(_) => break,
            }
            cx.background_executor().timer(LIVE_POLL).await;
        }));
    }
}

/// Insights' tracked-time refresh while recording.
pub(super) const INSIGHTS_BEAT: Duration = Duration::from_secs(60);
/// The status bar's "today" re-read.
pub(super) const TODAY_BEAT: Duration = Duration::from_secs(300);

/// `user_context_stamp_local_offset`, fire-and-forget: a failed stamp never
/// breaks anything.
pub fn stamp_local_offset(core: &Arc<AppCore>, cx: &gpui_kit::App) {
    let Some(infra) = core.try_state::<app_core::AppInfraState>() else {
        return;
    };
    let minutes = i64::from(native::sys::local_offset_seconds() / 60);
    Tokio::spawn(cx, async move {
        let _ = app_core::user_context::commands::user_context_stamp_local_offset(&infra, minutes)
            .await;
    })
    .detach();
}
