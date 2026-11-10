//! The voice take shared by both enrollment doors (onboarding's Voice step and
//! Settings › Your voice; `voice-enrollment.ts` + `VoiceEnrollmentStore`): a
//! 15 s bounded clip, the level meter, then the embedder's verdict. The state
//! machine is pure over explicit `Instant`s (the test clock), so the 15 s
//! boundary is asserted on both sides. All judgment is the backend's: this
//! never inspects a clip.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{div, px, Div, Hsla, Pixels};

/// `ENROLLMENT_CLIP_MS`.
pub const CLIP: Duration = Duration::from_millis(15_000);
/// The level meter polls this often while recording.
pub const LEVEL_POLL: Duration = Duration::from_millis(150);
/// Bars in the level meter (mockup 07a draws eighteen).
pub const BARS: usize = 18;

/// `ENROLLMENT_SENTENCE`.
pub const SENTENCE: &str = "I keep my work on this machine, and I would like Mnema to know my voice. I work on a few things at once, and I would like the transcript to say who said what. Reading this out loud takes about fifteen seconds.";

/// `enroll_account_owner_voice`'s tagged outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum Enrollment {
    Enrolled,
    TooShort { duration_ms: u64 },
    NoSpeech,
    MultipleSpeakers { count: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Recording {
        started: Instant,
    },
    /// The clip is done; the embedder (or, in Settings, the recorder) answers.
    Judging,
    /// Settings only: the take is on disk, waiting for Use this take / Discard.
    Review(String),
    Rejected(Enrollment),
    Enrolled,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Take {
    pub phase: Phase,
    pub take: u32,
    pub take_ms: u64,
    pub levels: [f32; BARS],
    pub peak: f32,
    pub elapsed: Duration,
}

impl Default for Take {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            take: 0,
            take_ms: 0,
            levels: [0.; BARS],
            peak: 0.,
            elapsed: Duration::ZERO,
        }
    }
}

impl Take {
    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Recording { .. } | Phase::Judging)
    }

    /// Start a take; `false` while one is already running.
    pub fn start(&mut self, now: Instant) -> bool {
        if self.busy() {
            return false;
        }
        *self = Self {
            phase: Phase::Recording { started: now },
            take: self.take,
            ..Self::default()
        };
        true
    }

    /// Advance the clock. `true` when the clip has just reached its 15 s length
    /// (the take ends and goes to the embedder).
    pub fn tick(&mut self, now: Instant) -> bool {
        let Phase::Recording { started } = self.phase else {
            return false;
        };
        self.elapsed = now.saturating_duration_since(started);
        if self.elapsed < CLIP {
            return false;
        }
        self.take += 1;
        self.take_ms = self.elapsed.as_millis() as u64;
        self.phase = Phase::Judging;
        true
    }

    pub fn level(&mut self, v: f32) {
        let v = v.clamp(0., 1.);
        self.levels.rotate_left(1);
        self.levels[BARS - 1] = v;
        self.peak = self.peak.max(v);
    }

    /// The recorder handed back the clip: hold it for review.
    pub fn review(&mut self, clip: String) {
        self.phase = Phase::Review(clip);
    }

    /// Cancel a running take, or discard one under review: back to idle, meter flat.
    pub fn cancel(&mut self) {
        *self = Self {
            take: self.take,
            ..Self::default()
        };
    }

    pub fn judged(&mut self, outcome: Result<Enrollment, String>) {
        self.phase = match outcome {
            Ok(Enrollment::Enrolled) => Phase::Enrolled,
            Ok(other) => Phase::Rejected(other),
            Err(e) => Phase::Failed(e),
        };
    }

    /// Whole seconds left in the take (`secondsLeft`: rounded up, never below 0).
    pub fn seconds_left(&self) -> u64 {
        let left = CLIP.saturating_sub(self.elapsed).as_millis() as u64;
        left.div_ceil(1000)
    }

    /// The status line under the meter.
    pub fn line(&self) -> String {
        match self.phase {
            Phase::Recording { .. } => format!("{} of about {}", clock(self.elapsed), clock(CLIP)),
            Phase::Judging => "Checking the take\u{2026}".into(),
            _ if self.take == 0 => format!("Ready when you are \u{b7} about {}", clock(CLIP)),
            _ => format!(
                "Take {} \u{b7} {} \u{b7} peaked at {}%",
                self.take,
                clock(Duration::from_millis(self.take_ms)),
                (self.peak * 100.).round()
            ),
        }
    }
}

pub fn clock(d: Duration) -> String {
    let s = (d.as_millis() as f64 / 1000.).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `(title, body)` for a rejection; the body is `rejectionMessage`, shared with Settings.
pub fn rejection(e: &Enrollment) -> (&'static str, String) {
    match e {
        Enrollment::TooShort { duration_ms } => {
            let s = ((*duration_ms as f64 / 1000.).round() as u64).max(1);
            (
                "Too short to learn from",
                format!(
                    "That take had only {s} second{} of speech in it. Read the whole sentence out loud, then try again.",
                    if s == 1 { "" } else { "s" }
                ),
            )
        }
        Enrollment::NoSpeech => (
            "Too quiet to hear",
            "Mnema heard no speech in that take. Check the right microphone is selected and that you are not muted, then try again.".into(),
        ),
        Enrollment::MultipleSpeakers { count } => (
            "More than one voice",
            format!("Mnema heard {count} voices in that take. It can only learn one, so record somewhere quieter \u{2014} or ask the other person for fifteen seconds of silence."),
        ),
        Enrollment::Enrolled => ("Enrolled.", "Mnema will pick your voice out from now on.".into()),
    }
}

/// `voiceErrorMessage`: a failed take in words, from the recorder's error code.
pub fn error_words(code: &str, raw: String) -> String {
    match code {
        "microphone_clip_already_recording" => {
            "A take is already recording. Wait for it to finish, then try again.".into()
        }
        "microphone_input_unavailable" => "That didn\u{2019}t record \u{2014} the microphone is unavailable. Another app may be using it, or it was unplugged. Free it up, then record again.".into(),
        "unsupported_platform" => {
            "Voice enrollment isn\u{2019}t available in this build of Mnema.".into()
        }
        _ if raw.to_lowercase().contains("not compiled into this build") => {
            "Voice enrollment isn\u{2019}t available in this build of Mnema.".into()
        }
        _ => raw,
    }
}

/// The level meter (`.ob-meter`): one bar per sample, `h(level)` tall.
pub fn meter(levels: &[f32], bar_w: Pixels, max_h: f32, color: Hsla, sqrt: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(if bar_w > px(3.) { 3. } else { 2. }))
        .h(px(max_h + 8.))
        .children(levels.iter().map(|l| {
            let v = if sqrt { l.max(0.).sqrt() } else { *l };
            div()
                .flex_none()
                .w(bar_w)
                .h(px(4. + (v * max_h).round()))
                .rounded(px(2.))
                .bg(color)
        }))
}

/// The app-core doors both enrollment surfaces call (blocking: run them off-main).
pub mod live {
    use std::sync::Arc;

    use app_core::{AppCore, AppInfraState};

    use super::{error_words, Enrollment};

    /// `record_bounded_microphone_clip`: blocks for the take, answers the clip path.
    pub fn record(core: &Arc<AppCore>, ms: u64) -> Result<String, String> {
        use app_core::native_capture::microphone::bounded_clip::record_bounded_microphone_clip;
        core.block_on(record_bounded_microphone_clip(ms, core))
            .map_err(|e| error_words(&e.code, e.message))
    }

    /// `enroll_account_owner_voice` on a recorded clip.
    pub fn enroll(
        core: &Arc<AppCore>,
        infra: &AppInfraState,
        clip: &str,
    ) -> Result<Enrollment, String> {
        use app_core::voice_enrollment::{
            enroll_account_owner_voice, EnrollAccountOwnerVoiceRequest,
            VoiceEnrollmentOutcomeDto as O,
        };
        let request = EnrollAccountOwnerVoiceRequest {
            clip_path: clip.into(),
            display_name: None,
        };
        Ok(
            match core.block_on(enroll_account_owner_voice(request, core, infra))? {
                O::Enrolled { .. } => Enrollment::Enrolled,
                O::TooShort { duration_ms } => Enrollment::TooShort { duration_ms },
                O::NoSpeech => Enrollment::NoSpeech,
                O::MultipleSpeakers { speaker_count } => Enrollment::MultipleSpeakers {
                    count: speaker_count,
                },
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_voice_take_ends_at_exactly_fifteen_seconds() {
        let t0 = Instant::now();
        let mut take = Take::default();
        assert!(take.start(t0));
        assert!(!take.start(t0), "one take at a time");
        assert!(!take.tick(t0 + CLIP - MS), "still recording at 14 999 ms");
        assert!(matches!(take.phase, Phase::Recording { .. }));
        assert_eq!(take.line(), "0:15 of about 0:15");
        assert!(take.tick(t0 + CLIP), "the take ends at 15 000 ms");
        assert_eq!(take.phase, Phase::Judging);
        take.judged(Ok(Enrollment::TooShort { duration_ms: 6000 }));
        assert_eq!(
            rejection(&Enrollment::TooShort { duration_ms: 6000 }).0,
            "Too short to learn from"
        );
        assert!(take.start(t0), "a rejection can be retried");
    }

    #[test]
    fn the_countdown_rounds_up_and_a_cancel_flattens_the_meter() {
        let t0 = Instant::now();
        let mut take = Take::default();
        take.start(t0);
        assert_eq!(take.seconds_left(), 15);
        take.tick(t0 + Duration::from_millis(1_000) - MS);
        assert_eq!(take.seconds_left(), 15, "999 ms in: still 15 s left");
        take.tick(t0 + Duration::from_millis(1_000));
        assert_eq!(take.seconds_left(), 14, "1 000 ms in: 14 s left");
        take.tick(t0 + CLIP - MS);
        assert_eq!(take.seconds_left(), 1, "14 999 ms in: 1 s left");
        take.level(0.7);
        take.cancel();
        assert_eq!(take.phase, Phase::Idle);
        assert_eq!(take.levels, [0.; BARS]);
        assert!(take.start(t0), "a cancelled take can be retaken");
    }

    #[test]
    fn recorder_errors_read_as_what_to_do() {
        assert!(error_words("microphone_input_unavailable", "x".into()).contains("unavailable"));
        assert_eq!(error_words("other", "raw".into()), "raw");
    }
}
