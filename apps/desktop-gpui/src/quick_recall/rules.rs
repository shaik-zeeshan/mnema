//! The pure rules behind Quick Recall's states (`recall-states.ts`,
//! `result-sections.ts`, `filterSurfaces`' options, `friendlyParseError`,
//! `askNotice`): what a state says, never how it's drawn.

use app_infra::{RecentQuery, SearchFacet, SearchFacetCounts, SearchParseError};
use capture_types::RecordingSettings;
use time::{Duration, OffsetDateTime};

use super::model::IndexStatus;
use super::tokens::{
    iso_day, operator_query, pill_edit_text, pill_label, pill_operators, Kind, Part, Pill, Src,
};

pub const RECENT_MAX: usize = 8;
/// The best moment + 7 rows, collapsed.
pub const VISIBLE_CAP: usize = 8;
pub const FRAME_FETCH_LIMIT: u32 = 24;
pub const AUDIO_FETCH_LIMIT: u32 = 12;
pub const MIN_QUERY_LENGTH: usize = 2;
/// Unread frames count as "behind" once the oldest has waited this long.
pub const INDEX_LAG_MS: i64 = 60_000;
pub const SEMANTIC_BUILDING_BELOW: u8 = 95;

/// "50+" when a per-kind cap was hit.
pub fn count_label(n: usize, capped: bool) -> String {
    if capped {
        format!("{n}+")
    } else {
        n.to_string()
    }
}

/// The text a query is remembered as: the words plus each pill back as its phrase.
pub fn recent_text(residual: &str, pills: &[Pill]) -> String {
    std::iter::once(residual.trim().to_string())
        .chain(pills.iter().map(pill_edit_text))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Newest first, one per text (case-insensitive), at most 8.
pub fn with_recent(list: &[RecentQuery], entry: RecentQuery) -> Vec<RecentQuery> {
    let key = entry.text.trim().to_lowercase();
    if key.is_empty() {
        return list.to_vec();
    }
    std::iter::once(entry)
        .chain(
            list.iter()
                .filter(|q| q.text.trim().to_lowercase() != key)
                .cloned(),
        )
        .take(RECENT_MAX)
        .collect()
}

fn local(ms: i64, now: OffsetDateTime) -> OffsetDateTime {
    crate::chat::format::local(ms, now)
}

/// "just now" · "12m ago" · "3h ago" · "yesterday" · "Wed" · "Oct 3".
pub fn ago_label(at: i64, now: OffsetDateTime) -> String {
    let now_ms = (now.unix_timestamp_nanos() / 1_000_000) as i64;
    let mins = (now_ms - at).div_euclid(60_000);
    if mins < 1 {
        return "just now".into();
    }
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let then = local(at, now);
    let days = (now.date() - then.date()).whole_days();
    match days {
        0 => format!("{}h ago", mins / 60),
        1 => "yesterday".into(),
        2..=6 => super::tokens::day_label(then.date())[..3].to_string(),
        _ => {
            let l = super::tokens::day_label(then.date());
            l[4..].to_string()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeIcon {
    Info,
    Off,
    Alert,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub warn: bool,
    pub info: bool,
    pub icon: NoticeIcon,
    pub text: String,
    pub settings: bool,
}

fn plural(n: i64, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// "Still reading your last 4 min — 38 frames queued."
pub fn indexing_line(s: &IndexStatus, now: i64) -> String {
    let mins = ((now - s.oldest_queued_ms.unwrap_or(now)) as f64 / 60_000.)
        .round()
        .max(1.) as i64;
    let span = if mins < 90 {
        format!("{mins} min")
    } else {
        plural((mins as f64 / 60.).round() as i64, "hour")
    };
    format!(
        "Still reading your last {span} — {} queued.",
        plural(s.queued_frames, "frame")
    )
}

/// At most one line over the results, most actionable first.
pub fn search_notice(
    status: Option<&IndexStatus>,
    degraded: bool,
    untranscribed: u32,
    recording: Option<bool>,
    now: i64,
) -> Option<Notice> {
    let n = |warn, info, icon, text: String, settings| Notice {
        warn,
        info,
        icon,
        text,
        settings,
    };
    if degraded {
        return Some(n(
            true,
            false,
            NoticeIcon::Alert,
            "Search by meaning hit an error — these are keyword matches only.".into(),
            false,
        ));
    }
    if let Some(s) = status {
        let lag = s.oldest_queued_ms.map_or(0, |o| now - o);
        if s.queued_frames > 0 && lag >= INDEX_LAG_MS {
            let text = format!("{} Newer moments may be missing.", indexing_line(s, now));
            return Some(n(false, true, NoticeIcon::Info, text, false));
        }
    }
    if untranscribed > 0 {
        let u = untranscribed as i64;
        let (verb, tail) = if u == 1 {
            ("is", "it’ll show up once it is")
        } else {
            ("are", "they’ll show up once they are")
        };
        let text = format!(
            "{} {verb} still being transcribed — {tail}.",
            plural(u, "recording")
        );
        return Some(n(false, true, NoticeIcon::Info, text, false));
    }
    let pct = status.and_then(|s| s.semantic_indexed_percent);
    if let (Some(s), Some(pct)) = (status, pct) {
        if s.semantic_building && pct < SEMANTIC_BUILDING_BELOW {
            let text = format!(
                "Search by meaning is still building — {pct}% of your history so far. Keyword results are complete."
            );
            return Some(n(false, true, NoticeIcon::Info, text, false));
        }
    }
    if recording == Some(false) {
        let text = "Mnema isn’t recording right now — this searches everything captured before it stopped.";
        return Some(n(false, false, NoticeIcon::Off, text.into(), false));
    }
    if status.is_some() && pct.is_none() {
        let text = "Searching keywords only — search by meaning is off.";
        return Some(n(false, false, NoticeIcon::Off, text.into(), true));
    }
    None
}

/// One row of a pill's popover: the pill it would become (None drops it).
#[derive(Clone, Debug, PartialEq)]
pub struct PillOption {
    pub label: String,
    pub pill: Option<Pill>,
    /// Keys the facet counts ("" = the "Any …" row).
    pub op: String,
}

fn option(pill: Option<Pill>, label: Option<&str>) -> PillOption {
    PillOption {
        label: label.map_or_else(
            || pill.as_ref().map(pill_label).unwrap_or_default(),
            String::from,
        ),
        op: pill.as_ref().map(pill_operators).unwrap_or_default(),
        pill,
    }
}

/// The alternatives offered for one pill.
pub fn pill_options(pill: &Pill, apps: &[String]) -> Vec<PillOption> {
    match pill {
        Pill::App(_) => apps
            .iter()
            .map(|a| option(Some(Pill::App(a.clone())), None))
            .chain([option(None, Some("Any app"))])
            .collect(),
        Pill::Source(_) => vec![
            option(Some(Pill::Source(Src::Screen)), None),
            option(Some(Pill::Source(Src::Mic)), Some("On a call (mic)")),
            option(Some(Pill::Source(Src::System)), None),
            option(None, Some("Any source")),
        ],
        Pill::Date { value, label, .. } => {
            let one_day = value
                .as_ref()
                .is_none_or(|v| !v.ends_with("-week") && !v.ends_with("-month"));
            let mut out = Vec::new();
            if one_day {
                for part in Part::ALL {
                    let mut p = pill.clone();
                    if let Pill::Date { part: slot, .. } = &mut p {
                        *slot = Some(part);
                    }
                    out.push(option(Some(p), None));
                }
                if value.is_some() {
                    let mut p = pill.clone();
                    if let Pill::Date { part: slot, .. } = &mut p {
                        *slot = None;
                    }
                    out.push(option(Some(p), Some(&format!("{label} · all day"))));
                }
            }
            let week = |v: &str, l: &str| Pill::Date {
                value: Some(v.into()),
                label: l.into(),
                part: None,
                today: false,
            };
            out.push(option(Some(week("this-week", "This week")), None));
            out.push(option(Some(week("last-week", "Last week")), None));
            out.push(option(None, Some("Any time")));
            out
        }
    }
}

/// Options with no results drop out once counts are in (not the current one or "Any").
pub fn shown_options(
    options: Vec<PillOption>,
    counts: Option<&[(String, String)]>,
    current: &str,
) -> Vec<PillOption> {
    options
        .into_iter()
        .filter(|o| {
            o.pill.is_none()
                || o.op == current
                || counts.is_none_or(|c| {
                    c.iter()
                        .find(|(k, _)| *k == o.op)
                        .is_none_or(|(_, n)| n != "0")
                })
        })
        .collect()
}

/// The count calls one pill's popover needs (a date pill takes two).
pub fn popover_calls(residual: &str, pills: &[Pill], pill: &Pill) -> Vec<(SearchFacet, String)> {
    let query = operator_query(residual, pills);
    match pill.kind() {
        Kind::App => vec![(SearchFacet::App, query)],
        Kind::Source => vec![(SearchFacet::Source, query)],
        Kind::Date => {
            let rest: Vec<Pill> = pills
                .iter()
                .filter(|p| p.kind() != Kind::Date)
                .cloned()
                .collect();
            vec![
                (SearchFacet::PartOfDay, query),
                (SearchFacet::Day, operator_query(residual, &rest)),
            ]
        }
    }
}

/// Monday..Sunday of this week (offset 0) or an earlier one.
pub fn week_days(now: OffsetDateTime, offset: i64) -> Vec<String> {
    let monday = now.date() - Duration::days(now.weekday().number_days_from_monday() as i64)
        + Duration::weeks(offset);
    (0..7)
        .map(|i| iso_day(monday + Duration::days(i)))
        .collect()
}

/// Counts keyed by `PillOption::op`.
pub fn popover_counts(
    pill: &Pill,
    options: &[PillOption],
    results: &[(SearchFacet, SearchFacetCounts)],
    now: OffsetDateTime,
) -> Vec<(String, String)> {
    let of = |f: SearchFacet| results.iter().find(|r| r.0 == f).map(|r| &r.1);
    let bucket = |c: &SearchFacetCounts, m: &dyn Fn(&str, &str) -> bool| -> usize {
        c.buckets
            .iter()
            .filter(|b| m(&b.key, &b.label))
            .map(|b| b.count as usize)
            .sum()
    };
    let mut out = Vec::new();
    for o in options {
        let mut c = None;
        let mut n = None;
        match (pill.kind(), &o.pill) {
            (Kind::App | Kind::Source, next) => {
                c = of(if pill.kind() == Kind::App {
                    SearchFacet::App
                } else {
                    SearchFacet::Source
                });
                if let Some(cc) = c {
                    n = Some(match next {
                        Some(Pill::App(a)) => {
                            let name = a.to_lowercase();
                            bucket(cc, &|k, l| {
                                l.to_lowercase() == name || k.to_lowercase() == name
                            })
                        }
                        Some(Pill::Source(s)) => {
                            let key = match s {
                                Src::Screen => "screen",
                                Src::Mic => "microphone",
                                Src::System => "system_audio",
                            };
                            bucket(cc, &|k, _| k == key)
                        }
                        _ => cc.total as usize,
                    });
                }
            }
            (Kind::Date, None) => {
                c = of(SearchFacet::Day);
                n = c.map(|c| c.total as usize);
            }
            (Kind::Date, Some(Pill::Date { value, part, .. })) => {
                let Pill::Date { value: pv, .. } = pill else {
                    continue;
                };
                let week =
                    |v: &Option<String>| matches!(v.as_deref(), Some("this-week" | "last-week"));
                if value == pv && !week(pv) {
                    c = of(SearchFacet::PartOfDay);
                    n = c.map(|cc| match part {
                        Some(p) => bucket(cc, &|k, _| k == p.name()),
                        None => cc.total as usize,
                    });
                } else if week(value) {
                    c = of(SearchFacet::Day);
                    let days = week_days(
                        now,
                        if value.as_deref() == Some("this-week") {
                            0
                        } else {
                            -1
                        },
                    );
                    n = c.map(|cc| bucket(cc, &|k, _| days.iter().any(|d| d == k)));
                }
            }
            _ => {}
        }
        if let (Some(c), Some(n)) = (c, n) {
            // A capped zero says nothing; leave it unnumbered.
            if !(c.capped && n == 0) {
                out.push((o.op.clone(), count_label(n, c.capped)));
            }
        }
    }
    out
}

/// "on Tue Oct 6 afternoon", "yesterday", "this afternoon".
pub fn date_phrase(pill: &Pill) -> String {
    let Pill::Date {
        value, label, part, ..
    } = pill
    else {
        return String::new();
    };
    let Some(v) = value else {
        return format!("in the {}", part.map_or("", Part::name));
    };
    if v == "today" {
        if let Some(p) = part {
            return format!("this {}", p.name());
        }
    }
    let named = matches!(
        v.as_str(),
        "today" | "yesterday" | "this-week" | "last-week" | "this-month" | "last-month"
    );
    let day = if named {
        label.to_lowercase()
    } else {
        format!("on {label}")
    };
    match part {
        Some(p) => format!("{day} {}", p.name()),
        None => day,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WayOut {
    Pills {
        pills: Vec<Pill>,
        label: String,
        facet: SearchFacet,
        query: String,
    },
    Ask {
        label: String,
    },
}

impl WayOut {
    pub fn label(&self) -> &str {
        match self {
            WayOut::Pills { label, .. } | WayOut::Ask { label } => label,
        }
    }
}

/// The same search with one filter loosened: all of the day, then dropping each
/// pill, then Ask.
pub fn ways_out(residual: &str, pills: &[Pill], can_ask: bool) -> Vec<WayOut> {
    let mut out = Vec::new();
    if let Some(
        date @ Pill::Date {
            value: Some(_),
            part: Some(part),
            ..
        },
    ) = pills.iter().find(|p| p.kind() == Kind::Date)
    {
        let mut whole = date.clone();
        if let Pill::Date { part: slot, .. } = &mut whole {
            *slot = None;
        }
        let wider: Vec<Pill> = pills
            .iter()
            .map(|p| if p == date { whole.clone() } else { p.clone() })
            .collect();
        let phrase = date_phrase(&whole);
        out.push(WayOut::Pills {
            label: format!(
                "All of {}, not just the {}",
                phrase.strip_prefix("on ").unwrap_or(&phrase),
                part.name()
            ),
            facet: SearchFacet::PartOfDay,
            query: operator_query(residual, &wider),
            pills: wider,
        });
    }
    for pill in pills {
        let rest: Vec<Pill> = pills
            .iter()
            .filter(|p| p.kind() != pill.kind())
            .cloned()
            .collect();
        let (label, facet) = match pill.kind() {
            Kind::Date => (
                format!("Any time — not just {}", date_phrase(pill)),
                SearchFacet::Day,
            ),
            Kind::App => (format!("Drop “{}”", pill_label(pill)), SearchFacet::App),
            Kind::Source => (format!("Drop “{}”", pill_label(pill)), SearchFacet::Source),
        };
        out.push(WayOut::Pills {
            query: operator_query(residual, &rest),
            pills: rest,
            label,
            facet,
        });
    }
    if can_ask && !residual.trim().is_empty() {
        out.push(WayOut::Ask {
            label: "Ask instead — I’ll look harder".into(),
        });
    }
    out
}

/// Plain words for the first parse error (unknown kinds keep the backend's message).
pub fn friendly_parse_error(e: &SearchParseError) -> String {
    let t = e.token.trim();
    match e.kind.as_str() {
        "bad_date" if !t.is_empty() => {
            format!("“{t}” isn't a date I understand. Try a day like 2024-05-01, or today / yesterday.")
        }
        "bad_date" => "That date filter isn't one I understand. Try a day like 2024-05-01, or today / yesterday.".into(),
        "unknown_source" if !t.is_empty() => {
            format!("“{t}” isn't a source I know. Use source:mic, source:system, or source:screen.")
        }
        "unknown_source" => "Use source:mic, source:system, or source:screen.".into(),
        "unbalanced_quote" => "There's an unclosed quote in your search — add the matching closing quote.".into(),
        "empty_value" => "That filter is missing a value — add a name after the colon.".into(),
        "app_source_conflict" => {
            "app: and source: can't be combined — app: narrows the screen, source: narrows audio.".into()
        }
        "screen_audio_source_conflict" => "source:screen can't be combined with source:mic or source:system.".into(),
        "dangling_or" => "OR needs a search term on both sides.".into(),
        "pure_negation" => "An exclusion like -term needs at least one positive term to match.".into(),
        _ => e.message.clone(),
    }
}

/// The show-more toggle's label, or None when the list fits.
pub fn more_row_label(total: usize, expanded: bool) -> Option<String> {
    (total > VISIBLE_CAP).then(|| {
        if expanded {
            "↑ show less".into()
        } else {
            format!("↓ show {} more", total - VISIBLE_CAP)
        }
    })
}

/// "24+ screen · 12 audio · 4 days".
pub fn moment_breakdown(frames: usize, audio: usize, days: usize, more: (bool, bool)) -> String {
    [
        (frames > 0).then(|| format!("{} screen", count_label(frames, more.0))),
        (audio > 0).then(|| format!("{} audio", count_label(audio, more.1))),
        Some(format!("{days} day{}", if days == 1 { "" } else { "s" })),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// A row's time: the clock today, "Wed 2:12 PM" within the week, "Oct 6" before.
pub fn moment_time(at_ms: i64, now: OffsetDateTime) -> String {
    let at = local(at_ms, now);
    let days = (now.date() - at.date()).whole_days();
    let clock = crate::chat::format::clock(at);
    match days {
        0 => clock,
        1..=6 => format!("{} {clock}", &super::tokens::day_label(at.date())[..3]),
        _ => super::tokens::day_label(at.date())[4..].to_string(),
    }
}

/// The model Quick Recall would ask: short label, and whether it runs on this Mac.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AskEngine {
    pub model: Option<String>,
    pub local: bool,
}

pub fn ask_engine(s: Option<&RecordingSettings>) -> AskEngine {
    let Some(s) = s else {
        return AskEngine::default();
    };
    let e = crate::chat::data::engine(s);
    let Some((provider, model)) = e.default else {
        return AskEngine::default();
    };
    let p = s.ai_runtime.providers.iter().find(|p| p.id == provider);
    AskEngine {
        model: Some(crate::chat::format::model_display_name(&model)),
        local: crate::chat::format::provider_is_local(p),
    }
}

/// `Answer questions automatically`: always, or local-only with a local model.
pub fn auto_answer_allowed(s: Option<&RecordingSettings>, engine: &AskEngine) -> bool {
    use capture_types::QuickRecallAutoAnswer as A;
    match s.map(|s| &s.access.quick_recall_auto_answer) {
        Some(A::Always) => true,
        Some(A::Never) => false,
        _ => engine.local,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Plain,
    Off,
    Warn,
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AskNotice {
    pub tone: NoticeTone,
    pub title: String,
    pub text: String,
    /// `(label, primary)`.
    pub action: Option<(&'static str, bool)>,
}

/// Why Ask AI can't answer right now, in words. None = it can.
pub fn ask_notice(
    reason: Option<&str>,
    s: Option<&RecordingSettings>,
    online: bool,
    engine: &AskEngine,
) -> Option<AskNotice> {
    let providers = s.map(|s| s.ai_runtime.providers.as_slice()).unwrap_or(&[]);
    let name = |id: &str| crate::chat::format::provider_label_by_id(providers, id);
    let n = |tone, title: String, text: String, action| {
        Some(AskNotice {
            tone,
            title,
            text,
            action,
        })
    };
    let Some(reason) = reason else {
        if online || engine.local {
            return None;
        }
        let model = engine.model.clone().unwrap_or_else(|| "Your model".into());
        return n(
            NoticeTone::Warn,
            "You’re offline".into(),
            format!(
                "{model} runs in the cloud. Search still works — answers resume when you’re back."
            ),
            None,
        );
    };
    if reason == "ask_ai_disabled" || reason == "ai_runtime_disabled" {
        return n(
            NoticeTone::Off,
            "Ask AI is off".into(),
            "Search still works — these are the moments that match.".into(),
            Some(("Turn on in Settings →", false)),
        );
    }
    if reason == "no_providers" {
        return n(
            NoticeTone::Plain,
            "Ask AI needs a provider".into(),
            "Add a provider API key in Settings, or run a model on this Mac. The moments below don’t need one.".into(),
            Some(("Set up AI", true)),
        );
    }
    if let Some(id) = reason.strip_prefix("needs_reconnect:") {
        return n(
            NoticeTone::Danger,
            format!("{} needs you to sign in again", name(id)),
            "Your sign-in ended. Reconnect, then ask again.".into(),
            Some(("Reconnect", true)),
        );
    }
    if let Some(id) = reason.strip_prefix("provider_unreachable:") {
        return n(
            NoticeTone::Warn,
            format!("Can’t reach {} right now", name(id)),
            "Answers resume on their own. The moments below are all on this Mac.".into(),
            None,
        );
    }
    match crate::insights::model::reason_copy(reason, s) {
        crate::insights::model::Engine::Unreachable(text) => n(
            NoticeTone::Warn,
            "Can’t reach the model right now".into(),
            text,
            None,
        ),
        crate::insights::model::Engine::Fix(text, _) => n(
            NoticeTone::Plain,
            "Ask AI isn’t set up yet".into(),
            text,
            Some(("Set up AI", true)),
        ),
        _ => None,
    }
}

/// "just now" · "12m ago" · "3h ago" · "2d ago" · "3w ago" · "4mo ago" · "1y ago".
pub fn relative_time(at_ms: i64, now_ms: i64) -> String {
    let min = (now_ms - at_ms).div_euclid(60_000);
    if min < 1 {
        return "just now".into();
    }
    if min < 60 {
        return format!("{min}m ago");
    }
    let hr = min / 60;
    if hr < 24 {
        return format!("{hr}h ago");
    }
    let day = hr / 24;
    if day < 7 {
        return format!("{day}d ago");
    }
    if day / 7 < 5 {
        return format!("{}w ago", day / 7);
    }
    if day / 30 < 12 {
        return format!("{}mo ago", day / 30);
    }
    format!("{}y ago", day / 365)
}

/// "Wed, Nov 4" (`toLocaleDateString(weekday/month short, day)`).
pub fn day_comma(d: time::Date) -> String {
    let l = super::tokens::day_label(d);
    format!("{},{}", &l[..3], &l[3..])
}

/// `formatClockRange`: "2:12–2:24 PM" (one meridiem when both share it).
pub fn clock_range(a: &str, b: &str) -> String {
    if a == b {
        return a.to_string();
    }
    let m = |s: &str| [" AM", " PM"].into_iter().find(|m| s.ends_with(m));
    match (m(a), m(b)) {
        (Some(x), Some(y)) if x == y => format!("{}–{b}", &a[..a.len() - x.len()]),
        _ => format!("{a}–{b}"),
    }
}

/// "2:12:15 PM" (`formatClock(.., { seconds: true })`).
pub fn clock_secs(d: time::OffsetDateTime) -> String {
    let c = crate::chat::format::clock(d);
    let (hm, ampm) = c.split_once(' ').unwrap_or((&c, ""));
    format!("{hm}:{:02} {ampm}", d.second())
}

#[cfg(test)]
mod tests {
    #[test]
    fn ranges_share_a_meridiem_and_recent_caps_at_8() {
        assert_eq!(super::clock_range("2:12 PM", "2:24 PM"), "2:12–2:24 PM");
        assert_eq!(
            super::clock_range("11:50 AM", "12:10 PM"),
            "11:50 AM–12:10 PM"
        );
        let q = |t: &str| app_infra::RecentQuery {
            text: t.into(),
            ask: false,
            at: 0,
        };
        let list: Vec<_> = (0..8).map(|i| q(&format!("q{i}"))).collect();
        let next = super::with_recent(&list, q("Q3"));
        assert_eq!(next.len(), 8);
        assert_eq!(next[0].text, "Q3");
        assert_eq!(
            next.iter()
                .filter(|r| r.text.eq_ignore_ascii_case("q3"))
                .count(),
            1
        );
        assert_eq!(super::with_recent(&next, q("new"))[7].text, "q6");
    }
}
