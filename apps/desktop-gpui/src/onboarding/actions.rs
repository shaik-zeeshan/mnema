//! What the screens' buttons do. Every backend call goes through `run`
//! (background executor); timers use the executor clock, so tests drive them
//! with `advance_clock`.

use std::time::Duration;

use capture_types::{RetentionPolicy, StartNativeCaptureRequest};
use gpui_kit::{App, AppContext, Context, Window};

use super::data::Enrollment;
use super::downloads::{self, DlState};
use super::flow::{self, Feature, Step, WorkItem};
use super::storage::{self, ProbeState, Repair};
use super::voice;
use super::{FinalePhase, Moment, Onboarding};

/// The system-audio meter's poll (one bar per poll, ~2.5 s of sound at a glance).
pub const SYS_POLL: Duration = Duration::from_millis(120);
/// The Finale's evidence poll, bounded (90 ticks) so a missing OCR never polls forever.
pub const EVIDENCE_POLL: Duration = Duration::from_millis(1500);
const EVIDENCE_TICKS: u32 = 90;

impl Onboarding {
    // ── Permissions ────────────────────────────────────────────────────────
    pub fn request(&mut self, m: Moment, cx: &mut Context<Self>) {
        if self.requesting.is_some() {
            return;
        }
        if m == Moment::Accessibility {
            self.acc_asked = true;
            self.run(
                cx,
                |b| b.request_accessibility(),
                |s, g, _| s.gecko = g.or(s.gecko.take()),
            );
            return;
        }
        self.requesting = Some(m);
        self.error = None;
        let before = flow::is_granted(self.perms.screen);
        self.run(
            cx,
            move |b| b.request_permission(m.wire()),
            move |s, res, _| {
                s.requesting = None;
                match res {
                    Ok(p) => {
                        s.perms = p;
                        if m == Moment::SystemAudio {
                            s.sys_prompt_raised = true;
                        }
                        // A grant that appeared in this process IS the relaunch signal.
                        if m == Moment::Screen && !before && flow::is_granted(s.perms.screen) {
                            s.screen_needs_relaunch = true;
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "onboarding: the {} permission request failed: {e}",
                            m.wire()
                        );
                        s.error = Some(e);
                    }
                }
            },
        );
    }

    /// Re-read the grants (Re-check, window focus): a grant made in System
    /// Settings needs the same relaunch as an in-app one.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        let before = flow::is_granted(self.perms.screen);
        self.run(
            cx,
            |b| (b.permissions(), b.accessibility()),
            move |s, (perms, gecko), _| {
                if let Ok(p) = perms {
                    s.perms = p;
                }
                if gecko.is_some() {
                    s.gecko = gecko;
                }
                if !before && flow::is_granted(s.perms.screen) && s.step == Step::Permissions {
                    s.screen_needs_relaunch = true;
                }
            },
        );
    }

    /// Every "Open System Settings": the pane plus the floating helper (S15).
    pub fn open_helper(&mut self, kind: &'static str, cx: &mut App) {
        crate::small::helper::open(kind, cx);
    }

    pub fn moments(&self) -> Vec<Moment> {
        let mut v = vec![Moment::Screen, Moment::Microphone, Moment::SystemAudio];
        if self
            .gecko
            .as_ref()
            .is_some_and(|(_, names)| !names.is_empty())
        {
            v.push(Moment::Accessibility);
        }
        v
    }

    /// Answered = granted (nothing more to ask) or continued past. System audio
    /// can never answer itself: there is no grant to read.
    pub fn answered(&self, m: Moment) -> bool {
        self.passed.contains(&m)
            || match m {
                Moment::Accessibility => self.gecko.as_ref().is_some_and(|(t, _)| *t),
                Moment::SystemAudio => false,
                Moment::Screen => flow::is_granted(self.perms.screen),
                Moment::Microphone => flow::is_granted(self.perms.microphone),
            }
    }

    pub fn current_moment(&self) -> Option<Moment> {
        self.moments().into_iter().find(|m| !self.answered(*m))
    }

    pub fn upcoming(&self) -> Vec<Moment> {
        let Some(cur) = self.current_moment() else {
            return vec![];
        };
        let all = self.moments();
        let at = all.iter().position(|m| *m == cur).unwrap_or(0);
        all.into_iter()
            .skip(at + 1)
            .filter(|m| !self.answered(*m))
            .collect()
    }

    /// The Permissions primary: relaunch (the one hold), the next moment, or on.
    pub fn advance_permissions(&mut self, cx: &mut Context<Self>) {
        let (ready, _) = flow::screen_gate(self.perms.screen, self.screen_needs_relaunch);
        if !ready {
            self.run(cx, |b| b.relaunch(), |_, _, _| {});
            return;
        }
        let cur = self.current_moment();
        if cur.is_none() || self.upcoming().is_empty() {
            self.next(cx);
            return;
        }
        if let Some(m) = cur {
            self.passed.push(m);
        }
        cx.notify();
    }

    /// The live system-audio meter: only once the prompt was raised (building
    /// the tap IS the prompt) or a past session heard sound.
    pub fn sync_sys_poll(&mut self, cx: &mut Context<Self>) {
        let want = self.step == Step::Permissions
            && self.current_moment() == Some(Moment::SystemAudio)
            && (self.sys_prompt_raised
                || self.perms.system_audio
                    == capture_types::CapturePermissionState::AssumedWorking);
        if !want {
            if self.sys_poll.take().is_some() {
                self.sys_levels = vec![0.; self.sys_levels.len()];
                self.probing_sys = false;
            }
            return;
        }
        if self.sys_poll.is_some() {
            return;
        }
        let b = self.backend.clone();
        self.sys_poll = Some(cx.spawn(async move |this, cx| {
            let b2 = b.clone();
            cx.background_spawn(async move { b2.start_sys_audio_probe() })
                .await;
            loop {
                cx.background_executor().timer(SYS_POLL).await;
                let b2 = b.clone();
                let level = cx
                    .background_spawn(async move { b2.sys_audio_level() })
                    .await;
                let ok = this.update(cx, |s, cx| {
                    s.probing_sys = level.is_some();
                    s.sys_levels.remove(0);
                    s.sys_levels.push(level.unwrap_or(0.));
                    cx.notify();
                });
                if ok.is_err() {
                    return;
                }
            }
        }));
    }

    // ── Capture & Storage ──────────────────────────────────────────────────
    /// Probe the chosen folder; `seq` drops a late reply for a folder already left.
    pub fn run_probe(&mut self, cx: &mut Context<Self>) {
        self.probe_seq += 1;
        let ticket = self.probe_seq;
        self.probe_state = ProbeState::Checking;
        let path = self.draft.save_directory.clone();
        self.run(
            cx,
            move |b| b.probe(&path),
            move |s, res, _| {
                if ticket != s.probe_seq {
                    return;
                }
                match res {
                    Ok(p) => {
                        s.probe = Some(p);
                        s.probe_state = ProbeState::Done;
                    }
                    // Unmeasurable is not a shortfall (ADR 0040): the gate stays open.
                    Err(_) => {
                        s.probe = None;
                        s.probe_state = ProbeState::Failed;
                    }
                }
            },
        );
    }

    /// First run only: the recommended exclusions become real, strikeable rows.
    /// Draft-only (committed at the Finale), so nothing else is resynced.
    pub fn seed_recommended_apps(&mut self) {
        if self.seeded_apps || self.ever_saved {
            return;
        }
        self.seeded_apps = true;
        for (bundle, name) in self.recommended.clone() {
            let _ = app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
                &mut self.draft,
                &bundle,
                &name,
                true,
            );
        }
    }

    pub fn toggle_app(&mut self, id: &str, cx: &mut Context<Self>) {
        let on = self
            .draft
            .privacy
            .excluded_apps
            .iter()
            .find(|a| a.id == id)
            .is_some_and(|a| a.enabled);
        let _ = app_core::privacy_redaction_sources::set_privacy_excluded_app_enabled(
            &mut self.draft,
            id,
            !on,
        );
        cx.notify();
    }

    pub fn set_rate(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.draft.screen_frame_rate = 1. / storage::LADDER[ix.min(storage::LADDER.len() - 1)];
        cx.notify();
    }

    pub fn set_retention(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.draft.retention_policy = storage::RETENTION[ix.min(3)].0;
        cx.notify();
    }

    pub fn set_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.draft.save_directory = path;
        self.run_probe(cx);
    }

    pub fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        crate::native::dialogs::pick_folder(
            "Choose where Mnema stores captures",
            cx,
            move |path, cx| {
                let _ = this.update(cx, |s, cx| {
                    s.set_folder(path.to_string_lossy().into_owned(), cx)
                });
            },
        );
    }

    pub fn repair(&mut self, act: Repair, cx: &mut Context<Self>) {
        let rate = storage::nearest_ladder_index(self.draft.screen_frame_rate);
        let keep = storage::retention_index(&self.draft.retention_policy);
        match act {
            Repair::Default => self.set_folder(String::new(), cx),
            Repair::Pick => self.pick_folder(cx),
            Repair::Recheck => self.run_probe(cx),
            Repair::NoSemantic => self.toggle(Feature::SemanticSearch, cx),
            Repair::Slower => self.set_rate(rate + 2, cx),
            Repair::Tighten => self.set_retention(keep + 1, cx),
            Repair::Keep30 => {
                self.draft.retention_policy = RetentionPolicy::Days30;
                cx.notify();
            }
        }
    }

    // ── Change settings ────────────────────────────────────────────────────
    /// The ONLY way to flip a feature: runs the cascades both ways and offers undo.
    pub fn toggle(&mut self, f: Feature, cx: &mut Context<Self>) {
        let before = self.features;
        let Some(after) = before.toggled(f) else {
            return;
        };
        let moved = before.cascade(&after, f);
        self.undo = (!moved.is_empty() && f != Feature::AiFeatures).then(|| {
            let names: Vec<String> = moved.iter().map(|m| m.label().to_lowercase()).collect();
            (
                before,
                format!(
                    "{} went {} \u{2014} and {} went with it.",
                    f.label(),
                    if after.get(f) { "on" } else { "off" },
                    names.join(" + ")
                ),
            )
        });
        self.features = after;
        if !self.touched.contains(&f) {
            self.touched.push(f);
        }
        cx.notify();
    }

    pub fn undo_toggle(&mut self, cx: &mut Context<Self>) {
        if let Some((before, _)) = self.undo.take() {
            self.features = before;
            cx.notify();
        }
    }

    // ── Setup ──────────────────────────────────────────────────────────────
    /// Free-disk preflight: only a MEASURED shortfall holds the downloads.
    pub fn download_shortfall(&self) -> Option<(f64, f64)> {
        let need: u64 = self
            .work_list()
            .iter()
            .filter(|i| self.features.get(i.feature))
            .map(|i| i.bytes)
            .sum();
        let free = self.probe.as_ref()?.free?;
        (free < need as f64).then_some((free, need as f64))
    }

    pub fn drive_downloads(&mut self, cx: &mut Context<Self>) {
        let may = self.download_shortfall().is_none() || self.dl.disk_override;
        let items = self.work_list();
        let Some(item) = self.dl.next_to_start(&items, &self.features, may) else {
            return;
        };
        let it = item.clone();
        self.run(
            cx,
            move |b| b.start_download(&it),
            move |s, res, cx| {
                if let Err(e) = res {
                    s.dl.failed_to_start(&item, e);
                    s.drive_downloads(cx);
                }
            },
        );
    }

    pub fn on_download_event(&mut self, event: &serde_json::Value, cx: &mut Context<Self>) {
        let items = self.work_list();
        if self.dl.apply(event, &items).is_some() {
            if self.dl.in_flight.is_none() {
                self.drive_downloads(cx);
                // The start is refused while a model it needs is still
                // downloading; once nothing is in flight, ask again.
                if self.step == Step::Finale && matches!(self.finale, FinalePhase::Failed(_)) {
                    self.commit(cx);
                }
            }
            cx.notify();
        }
    }

    /// Cancel turns the dependent PROCESSING feature off and leaves the capture
    /// source on (cancelling Whisper never turns the microphone off).
    pub fn cancel_item(&mut self, item: &WorkItem, cx: &mut Context<Self>) {
        if self.features.get(item.feature) {
            self.toggle(item.feature, cx);
        }
        let mut stop = vec![item.clone()];
        for other in self.work_list() {
            if other != *item
                && !self.features.get(other.feature)
                && self.dl.state(&other) == DlState::Downloading
            {
                stop.push(other);
            }
        }
        for i in &stop {
            if self.dl.in_flight.as_deref() == Some(downloads::id(i).as_str()) {
                self.dl.in_flight = None;
            }
        }
        self.run(
            cx,
            move |b| stop.iter().for_each(|i| b.cancel_download(i)),
            |s, _, cx| s.drive_downloads(cx),
        );
    }

    pub fn retry_item(&mut self, item: &WorkItem, cx: &mut Context<Self>) {
        self.dl.retry(item);
        self.drive_downloads(cx);
        cx.notify();
    }

    pub fn restore_item(&mut self, item: &WorkItem, cx: &mut Context<Self>) {
        if item.feature == Feature::SpeakerSeparation && !self.features.transcription {
            self.toggle(Feature::Transcription, cx);
        }
        if !self.features.get(item.feature) {
            self.toggle(item.feature, cx);
        }
        self.retry_item(item, cx);
    }

    // ── Voice ──────────────────────────────────────────────────────────────
    /// One 15 s take, then the embedder's verdict. The take ends on the
    /// executor clock at `CLIP`; the recorder's own bound ends the real clip.
    pub fn record(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if !self.take.start(now) {
            return;
        }
        let b = self.backend.clone();
        self.voice_task = Some(cx.spawn(async move |this, cx| {
            let b2 = b.clone();
            let clip =
                cx.background_spawn(async move { b2.record_clip(voice::CLIP.as_millis() as u64) });
            loop {
                cx.background_executor().timer(voice::LEVEL_POLL).await;
                let b2 = b.clone();
                let level = cx.background_spawn(async move { b2.mic_level() }).await;
                let now = cx.background_executor().now();
                let done = this
                    .update(cx, |s, cx| {
                        s.take.level(level.unwrap_or(0.));
                        let done = s.take.tick(now);
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            let outcome = match clip.await {
                Ok(path) => {
                    let b2 = b.clone();
                    cx.background_spawn(async move { b2.enroll(&path) }).await
                }
                Err(e) => Err(e),
            };
            let _ = this.update(cx, |s, cx| {
                if matches!(outcome, Ok(Enrollment::Enrolled)) {
                    s.owner_enrolled = true;
                }
                s.take.judged(outcome);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Voice can run only with speaker separation on and its model on disk.
    pub fn voice_ready(&self) -> bool {
        if !self.features.speaker_separation {
            return false;
        }
        let installed = self
            .models
            .iter()
            .any(|(s, m)| *s == flow::Subsystem::Speaker && m.installed);
        installed
            || self
                .work_list()
                .iter()
                .find(|i| i.subsystem == flow::Subsystem::Speaker)
                .is_some_and(|i| self.dl.state(i) == DlState::Ready)
    }

    // ── Finale ─────────────────────────────────────────────────────────────
    /// What will ACTUALLY record: each source ANDed with its grant (system
    /// audio's is a raised prompt or sound already heard, ADR 0052).
    pub fn start_request(&self) -> StartNativeCaptureRequest {
        let i = self.intents();
        StartNativeCaptureRequest {
            capture_screen: self.features.screen && i.screen,
            capture_microphone: self.features.microphone && i.microphone,
            capture_system_audio: self.features.system_audio && i.system_audio,
        }
    }

    /// The single atomic commit, on arrival: save the whole config, complete
    /// onboarding, then start capture. Idempotent per arrival.
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        if self.committed {
            return;
        }
        self.committed = true;
        self.finale = FinalePhase::Starting;
        self.fold_features_into_draft();
        let start = self.start_request();
        let will = start.capture_screen || start.capture_microphone || start.capture_system_audio;
        let frames = start.capture_screen;
        let settings = self.draft.clone();
        self.run(
            cx,
            move |b| {
                let baseline = if frames { b.newest_frame_id() } else { None };
                (baseline, b.finish(&settings, will.then_some(start)))
            },
            move |s, (baseline, res), cx| match res {
                Err((completed, e)) => {
                    s.completed |= completed;
                    s.finale = FinalePhase::Failed(e);
                    // A failed commit may run again (Try again, coming back to
                    // this step, a model that finished downloading).
                    s.committed = false;
                }
                Ok(()) => {
                    s.completed = true;
                    if !will {
                        s.finale = FinalePhase::Idle;
                    } else {
                        s.finale = FinalePhase::Running;
                        s.rec_started = Some(crate::ui::clock::now(cx));
                        if frames {
                            s.poll_evidence(baseline, cx);
                        }
                    }
                }
            },
        );
    }

    fn fold_features_into_draft(&mut self) {
        let f = self.features;
        let d = &mut self.draft;
        d.capture_screen = f.screen;
        d.capture_microphone = f.microphone;
        d.capture_system_audio = f.system_audio;
        d.ocr.enabled = f.ocr;
        d.transcription.enabled = f.transcription;
        d.transcription.microphone_enabled = f.transcription && f.microphone;
        d.transcription.system_audio_enabled = f.transcription && f.system_audio;
        d.speaker_analysis.separate_speakers = f.speaker_separation;
        // Enrolling switched recognition on backend-side; a stale draft must not undo it.
        d.speaker_analysis.recognize_saved_people = f.speaker_separation
            && (d.speaker_analysis.recognize_saved_people || self.owner_enrolled);
        d.semantic_search.enabled = f.semantic_search;
        d.ai_runtime.enabled = f.ai_features;
    }

    /// Wait for evidence: a new frame row, then the OCR read off it. Bounded.
    fn poll_evidence(&mut self, baseline: Option<i64>, cx: &mut Context<Self>) {
        let b = self.backend.clone();
        self.finale_task = Some(cx.spawn(async move |this, cx| {
            for _ in 0..EVIDENCE_TICKS {
                cx.background_executor().timer(EVIDENCE_POLL).await;
                let b2 = b.clone();
                let ev = cx
                    .background_spawn(async move { b2.evidence(baseline) })
                    .await;
                let done = this
                    .update(cx, |s, cx| {
                        if let Some(ev) = ev {
                            s.evidence = Some(ev);
                        }
                        cx.notify();
                        s.evidence
                            .as_ref()
                            .is_some_and(|e| e.ocr.is_some() && e.image.is_some())
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        }));
    }

    /// A grant since the commit means there IS something to record: the commit
    /// path is idempotent, so run it again rather than a second way to start.
    pub fn recheck_finale(&mut self, cx: &mut Context<Self>) {
        self.run(
            cx,
            |b| b.permissions(),
            |s, perms, cx| {
                if let Ok(p) = perms {
                    s.perms = p;
                }
                let r = s.start_request();
                if r.capture_screen || r.capture_microphone || r.capture_system_audio {
                    s.committed = false;
                    s.commit(cx);
                }
            },
        );
    }

    pub fn relaunch(&mut self, cx: &mut Context<Self>) {
        self.run(cx, |b| b.relaunch(), |_, _, _| {});
    }

    /// "Open Mnema": main appears, this window goes.
    pub fn open_mnema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::window::ensure_main(cx);
        window.remove_window();
    }
}
