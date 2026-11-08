//! Chat scope (CH3, port of `lib/chat/scope.ts`): the time range + "About you"
//! switch resent with every turn, never persisted. Days are local calendar days.

use capture_types::AskAiScope;
use time::{Date, Duration, OffsetDateTime, Time, UtcOffset};

use crate::ui::calendar::{monday, month_of};

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

/// The DateInput range presets (`presets(today)`), label + inclusive days.
pub fn presets(today: Date) -> Vec<(&'static str, Date, Date)> {
    let m0 = monday(today);
    let d = Duration::days;
    let (ms, me) = month_of(today, 0);
    let (ls, le) = month_of(today, -1);
    vec![
        ("Today", today, today),
        ("Yesterday", today - d(1), today - d(1)),
        ("This week", m0, m0 + d(6)),
        ("Last week", m0 - d(7), m0 - d(1)),
        ("Last 7 days", today - d(6), today),
        ("Last 30 days", today - d(29), today),
        ("This month", ms, me),
        ("Last month", ls, le),
    ]
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Oct 3" / "Oct 3 – 9" / "Sep 28 – Oct 4" (`spanLabel`).
pub fn span_label(a: Date, b: Date) -> String {
    let md = |d: Date| format!("{} {}", MONTHS[d.month() as usize - 1], d.day());
    if a == b {
        md(a)
    } else if a.month() == b.month() {
        format!("{} – {}", md(a), b.day())
    } else {
        format!("{} – {}", md(a), md(b))
    }
}

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
