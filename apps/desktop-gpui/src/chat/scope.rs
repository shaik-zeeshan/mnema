//! Chat scope (CH3, port of `lib/chat/scope.ts`): the time range + "About you"
//! switch resent with every turn, never persisted. Days are local calendar days.

use capture_types::AskAiScope;
use time::{Date, Duration, OffsetDateTime, Time, UtcOffset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    /// Inclusive local days; both None = all time.
    pub start: Option<Date>,
    pub end: Option<Date>,
    pub about_you: bool,
}

impl Default for Scope {
    /// All time, About you on (founder ruling 2026-10-11).
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            about_you: true,
        }
    }
}

pub use crate::ui::calendar::{presets, span_label};

impl Scope {
    /// "all time", a preset's name lower-cased, else the span.
    pub fn label(&self, today: Date) -> String {
        let (Some(a), Some(b)) = (self.start, self.end) else {
            return "all time".into();
        };
        presets(today)
            .into_iter()
            .find(|(_, s, e)| *s == a && *e == b)
            .map(|(name, ..)| name.to_lowercase())
            .unwrap_or_else(|| span_label(a, b))
    }

    /// The composer's context button text ("all time · about you").
    pub fn summary(&self, today: Date) -> String {
        let about = if self.about_you { " · about you" } else { "" };
        format!("{}{about}", self.label(today))
    }

    /// Local midnight of `start` through the last ms of `end`; no bounds for all time.
    pub fn to_wire(&self, offset: UtcOffset) -> AskAiScope {
        let ms = |d: Date| {
            let t = d.with_time(Time::MIDNIGHT).assume_offset(offset);
            (t.unix_timestamp_nanos() / 1_000_000) as i64
        };
        match (self.start, self.end) {
            (Some(a), Some(b)) => AskAiScope {
                from_ms: Some(ms(a)),
                to_ms: Some(ms(b + Duration::days(1)) - 1),
                about_you: self.about_you,
            },
            _ => AskAiScope {
                from_ms: None,
                to_ms: None,
                about_you: self.about_you,
            },
        }
    }

    /// A "Try asking" suggestion's preset, keeping About you.
    pub fn preset(name: &str, about_you: bool, today: Date) -> Self {
        match presets(today).into_iter().find(|(l, ..)| *l == name) {
            Some((_, a, b)) => Self {
                start: Some(a),
                end: Some(b),
                about_you,
            },
            None => Self {
                about_you,
                ..Self::default()
            },
        }
    }
}

pub fn today(now: OffsetDateTime) -> Date {
    now.date()
}
