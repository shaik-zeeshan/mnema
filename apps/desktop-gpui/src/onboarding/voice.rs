//! The Voice take (`VoiceScreen.svelte`): a 15 s bounded clip, then the
//! embedder's verdict. Pure over explicit `Instant`s (the test clock), so the
//! 15 s boundary is asserted on both sides. All judgment is the backend's: this
//! never inspects a clip.

use std::time::{Duration, Instant};

use super::data::Enrollment;

/// `ENROLLMENT_CLIP_MS`.
pub const CLIP: Duration = Duration::from_millis(15_000);
/// The level meter polls this often while recording.
pub const LEVEL_POLL: Duration = Duration::from_millis(150);
/// Bars in the level meter (mockup 07a draws eighteen).
pub const BARS: usize = 18;

pub const SENTENCE: &str = "I keep my work on this machine, and I would like Mnema to know my voice. I work on a few things at once, and I would like the transcript to say who said what. Reading this out loud takes about fifteen seconds.";

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Recording { started: Instant },
    Judging,
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

    pub fn judged(&mut self, outcome: Result<Enrollment, String>) {
        self.phase = match outcome {
            Ok(Enrollment::Enrolled) => Phase::Enrolled,
            Ok(other) => Phase::Rejected(other),
            Err(e) => Phase::Failed(e),
        };
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

/// `(title, body)` for a rejection; the words are shared with Settings' door.
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
