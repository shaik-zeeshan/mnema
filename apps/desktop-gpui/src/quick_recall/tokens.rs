//! Natural language → search operators for the field (`natural-tokens.ts`,
//! `filter-chips.ts`' pill half, `query-tokens.ts`, `question-detect.ts`'s pure
//! rules, and `filterSurfaces`' ghost). Recognised phrases are tinted in place
//! and lift into pills after a pause; each pill serialises to the operators the
//! backend parser already knows. Offsets are byte offsets into the text.

use std::sync::LazyLock;

use regex::Regex;
use time::{Duration, OffsetDateTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Morning,
    Afternoon,
    Evening,
}

impl Part {
    pub const ALL: [Part; 3] = [Part::Morning, Part::Afternoon, Part::Evening];

    pub fn name(self) -> &'static str {
        match self {
            Part::Morning => "morning",
            Part::Afternoon => "afternoon",
            Part::Evening => "evening",
        }
    }

    /// `hours:H-H`, read in local time by the backend.
    pub fn hours(self) -> &'static str {
        match self {
            Part::Morning => "5-12",
            Part::Afternoon => "12-18",
            Part::Evening => "18-24",
        }
    }

    fn parse(s: &str) -> Option<Part> {
        Part::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Mic,
    System,
    Screen,
}

impl Src {
    pub fn op(self) -> &'static str {
        match self {
            Src::Mic => "mic",
            Src::System => "system",
            Src::Screen => "screen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    App,
    Date,
    Source,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pill {
    App(String),
    /// `value` = the `date:` value; None = a part of day on its own. `today`: a
    /// part said in words, which means today unless a day is named too.
    Date {
        value: Option<String>,
        label: String,
        part: Option<Part>,
        today: bool,
    },
    Source(Src),
}

impl Pill {
    pub fn kind(&self) -> Kind {
        match self {
            Pill::App(_) => Kind::App,
            Pill::Date { .. } => Kind::Date,
            Pill::Source(_) => Kind::Source,
        }
    }

    fn date(value: &str, label: &str, part: Option<Part>) -> Pill {
        Pill::Date {
            value: Some(value.into()),
            label: label.into(),
            part,
            today: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub pill: Pill,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parse {
    pub spans: Vec<Span>,
    pub residual: String,
}

const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
const SHORT_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn period_label(v: &str) -> Option<&'static str> {
    Some(match v {
        "this-week" => "This week",
        "last-week" => "Last week",
        "this-month" => "This month",
        "last-month" => "Last month",
        _ => return None,
    })
}

fn source_word(v: &str) -> Option<Src> {
    Some(match v.to_ascii_lowercase().as_str() {
        "mic" | "microphone" => Src::Mic,
        "system" | "system_audio" => Src::System,
        "screen" => Src::Screen,
        _ => return None,
    })
}

pub fn iso_day(d: time::Date) -> String {
    format!("{}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

/// "Tue Nov 3" (`toLocaleDateString(en-US, weekday/month short, day)` minus the comma).
pub fn day_label(d: time::Date) -> String {
    format!(
        "{} {} {}",
        SHORT_DAYS[d.weekday().number_days_from_sunday() as usize],
        MONTHS[d.month() as usize - 1],
        d.day()
    )
}

/// A one-day pill: today / yesterday keep their name; older days carry the ISO day.
fn one_day(now: OffsetDateTime, back: i64, part: Option<Part>) -> Pill {
    match back {
        0 => Pill::date("today", "Today", part),
        1 => Pill::date("yesterday", "Yesterday", part),
        _ => {
            let d = now.date() - Duration::days(back);
            Pill::date(&iso_day(d), &day_label(d), part)
        }
    }
}

/// A typed `date:` value, or None when the backend would reject it.
fn date_operator(now: OffsetDateTime, raw: &str) -> Option<Pill> {
    let v = raw.to_ascii_lowercase();
    match v.as_str() {
        "today" => return Some(one_day(now, 0, None)),
        "yesterday" => return Some(one_day(now, 1, None)),
        _ => {}
    }
    if let Some(label) = period_label(&v) {
        return Some(Pill::date(&v, label, None));
    }
    static ISO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").unwrap());
    let c = ISO.captures(&v)?;
    let month = time::Month::try_from(c[2].parse::<u8>().ok()?).ok()?;
    let d = time::Date::from_calendar_date(c[1].parse().ok()?, month, c[3].parse().ok()?).ok()?;
    Some(one_day(now, (now.date() - d).whole_days(), None))
}

fn re(s: &str) -> Regex {
    Regex::new(&format!("(?i){s}")).expect("static pattern")
}

static OPERATOR: LazyLock<Regex> =
    LazyLock::new(|| re(r#"\b(app|source|date|hours):("[^"]+"|\S+)"#));
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\b(?:(?:on|last)\s+)?(sunday|monday|tuesday|wednesday|thursday|friday|saturday)(?:\s+(morning|afternoon|evening))?\b",
    )
});
static THIS_PART: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:this\s+(morning|afternoon|evening)|(tonight))\b"));
static TODAY: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(today|yesterday)(?:\s+(morning|afternoon|evening))?\b"));
static WEEK: LazyLock<Regex> = LazyLock::new(|| re(r"\b(this|last)\s+week\b"));
static AGO: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(\d+|one|two|three|four|five|six)\s+days?\s+ago\b"));
static CALL: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:on|in|during)\s+(?:a\s+|the\s+)?(?:call|meeting)s?\b"));
static SCREEN: LazyLock<Regex> = LazyLock::new(|| re(r"\bon\s+(?:my\s+|the\s+)?screen\b"));
static VIDEO: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:in|on)\s+a\s+(?:podcast|video)\b"));
static PART: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:in\s+the\s+)?(morning|afternoon|evening)\b"));

fn find_app(apps: &[String], v: &str) -> Option<String> {
    let low = v.to_lowercase();
    apps.iter()
        .find(|a| a.to_lowercase() == low)
        .or_else(|| apps.iter().find(|a| a.to_lowercase().starts_with(&low)))
        .cloned()
}

type Make<'a> = Box<dyn Fn(&regex::Captures) -> Option<Pill> + 'a>;

/// The recognised phrases in `text` (earlier rules win an overlap) and the free
/// text left once they are lifted out.
pub fn parse_natural(text: &str, now: OffsetDateTime, apps: &[String]) -> Parse {
    let weekday_now = now.weekday().number_days_from_sunday() as i64;
    let mut rules: Vec<(Regex, Make)> = vec![
        (
            OPERATOR.clone(),
            Box::new(|m| {
                let v = m[2].replace('"', "");
                match m[1].to_ascii_lowercase().as_str() {
                    "app" => find_app(apps, &v).map(Pill::App),
                    "source" => source_word(&v).map(Pill::Source),
                    "hours" => Part::ALL
                        .into_iter()
                        .find(|p| p.hours() == v)
                        .map(|p| Pill::Date {
                            value: None,
                            label: String::new(),
                            part: Some(p),
                            today: false,
                        }),
                    _ => date_operator(now, &v),
                }
            }),
        ),
        (
            WEEKDAY.clone(),
            Box::new(move |m| {
                let ix = WEEKDAYS
                    .iter()
                    .position(|d| d.eq_ignore_ascii_case(&m[1]))? as i64;
                let back = (weekday_now - ix + 7) % 7;
                let back = if back == 0 { 7 } else { back };
                Some(one_day(
                    now,
                    back,
                    m.get(2).and_then(|p| Part::parse(p.as_str())),
                ))
            }),
        ),
        (
            THIS_PART.clone(),
            Box::new(move |m| {
                let part = m
                    .get(1)
                    .and_then(|p| Part::parse(p.as_str()))
                    .unwrap_or(Part::Evening);
                Some(one_day(now, 0, Some(part)))
            }),
        ),
        (
            TODAY.clone(),
            Box::new(move |m| {
                let back = if m[1].eq_ignore_ascii_case("today") {
                    0
                } else {
                    1
                };
                Some(one_day(
                    now,
                    back,
                    m.get(2).and_then(|p| Part::parse(p.as_str())),
                ))
            }),
        ),
        (
            WEEK.clone(),
            Box::new(|m| {
                let value = format!("{}-week", m[1].to_ascii_lowercase());
                let label = period_label(&value)?;
                Some(Pill::date(&value, label, None))
            }),
        ),
        (
            AGO.clone(),
            Box::new(move |m| {
                let n = match m[1].to_ascii_lowercase().as_str() {
                    "one" => 1,
                    "two" => 2,
                    "three" => 3,
                    "four" => 4,
                    "five" => 5,
                    "six" => 6,
                    d => d.parse().ok()?,
                };
                Some(one_day(now, n, None))
            }),
        ),
    ];
    // A bare app name collides with ordinary words ("Notes", "Mail"): a captured
    // app only counts after in / on / from / using.
    let mut names: Vec<&String> = apps.iter().filter(|a| !a.trim().is_empty()).collect();
    names.sort_by_key(|a| std::cmp::Reverse(a.len()));
    let app_re = (!names.is_empty()).then(|| {
        let alt = names
            .iter()
            .map(|n| regex::escape(n))
            .collect::<Vec<_>>()
            .join("|");
        re(&format!(r"\b(?:in|on|from|using)\s+({alt})"))
    });
    if let Some(app_re) = app_re {
        rules.push((
            app_re,
            // shortcut: JS's `(?![\w-])` lookahead is checked after the match, so a
            // name followed by a word char is skipped rather than retried shorter.
            Box::new(move |m| {
                let end = m.get(0)?.end();
                let next = text[end..].chars().next();
                if next.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                    return None;
                }
                Some(Pill::App(
                    find_app(apps, &m[1]).unwrap_or_else(|| m[1].to_string()),
                ))
            }),
        ));
    }
    rules.push((CALL.clone(), Box::new(|_| Some(Pill::Source(Src::Mic)))));
    rules.push((
        SCREEN.clone(),
        Box::new(|_| Some(Pill::Source(Src::Screen))),
    ));
    rules.push((VIDEO.clone(), Box::new(|_| Some(Pill::Source(Src::System)))));
    rules.push((
        PART.clone(),
        Box::new(|m| {
            Some(Pill::Date {
                value: None,
                label: String::new(),
                part: Part::parse(&m[1]),
                today: true,
            })
        }),
    ));

    let mut spans: Vec<Span> = Vec::new();
    for (re, make) in &rules {
        for m in re.captures_iter(text) {
            let all = m.get(0).expect("match");
            let (start, end) = (all.start(), all.end());
            if spans.iter().any(|s| start < s.end && end > s.start) {
                continue;
            }
            if let Some(pill) = make(&m) {
                spans.push(Span { start, end, pill });
            }
        }
    }
    spans.sort_by_key(|s| s.start);
    let mut residual = String::new();
    let mut i = 0;
    for s in &spans {
        residual.push_str(&text[i..s.start]);
        i = s.end;
    }
    residual.push_str(&text[i..]);
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());
    static PUNCT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+([?.!,])").unwrap());
    let residual = SPACES.replace_all(&residual, " ");
    let residual = PUNCT.replace_all(&residual, "$1").trim().to_string();
    Parse { spans, residual }
}

/// One pill per kind, in kind order. A part of day on its own narrows the date
/// pill already there; a new date keeps a part-only pill's part. Only then does
/// a worded part with no day left become today's.
pub fn merge_pills(base: &[Pill], add: &[Pill]) -> Vec<Pill> {
    let mut by: Vec<Pill> = base.to_vec();
    for p in add {
        let prev = by.iter().position(|b| b.kind() == p.kind());
        if let (
            Pill::Date { value, part, .. },
            Some(Pill::Date {
                value: pv,
                part: pp,
                today: pt,
                ..
            }),
        ) = (p, prev.map(|i| by[i].clone()))
        {
            let i = prev.expect("prev");
            if value.is_none() {
                if let Pill::Date { part: slot, .. } = &mut by[i] {
                    *slot = *part;
                }
                continue;
            }
            if (pv.is_none() || pt) && part.is_none() {
                let mut next = p.clone();
                if let Pill::Date { part: slot, .. } = &mut next {
                    *slot = pp;
                }
                by[i] = next;
                continue;
            }
        }
        match prev {
            Some(i) => by[i] = p.clone(),
            None => by.push(p.clone()),
        }
    }
    for p in &mut by {
        if let Pill::Date {
            value: value @ None,
            label,
            today: true,
            ..
        } = p
        {
            *value = Some("today".into());
            *label = "Today".into();
        }
    }
    by.sort_by_key(Pill::kind);
    by
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seg {
    Plain,
    Live,
    Bad,
}

/// The field's mirror: plain text, recognised phrases (tinted) and the token the
/// backend rejected (wavy underline).
pub fn mirror_segments(text: &str, spans: &[Span], bad: Option<&str>) -> Vec<(String, Seg)> {
    let mut marks: Vec<(usize, usize, Seg)> =
        spans.iter().map(|s| (s.start, s.end, Seg::Live)).collect();
    if let Some(bad) = bad.filter(|b| !b.is_empty()) {
        if let Some(at) = text.find(bad) {
            if !marks.iter().any(|m| at < m.1 && at + bad.len() > m.0) {
                marks.push((at, at + bad.len(), Seg::Bad));
                marks.sort_by_key(|m| m.0);
            }
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    for (s, e, k) in marks {
        if s > i {
            out.push((text[i..s].to_string(), Seg::Plain));
        }
        out.push((text[s..e].to_string(), k));
        i = e;
    }
    if i < text.len() {
        out.push((text[i..].to_string(), Seg::Plain));
    }
    out
}

/// Quote a name with spaces so `app:"Google Chrome"` parses as one token.
pub fn app_operator_token(name: &str) -> String {
    if name.contains(' ') {
        format!("app:\"{name}\"")
    } else {
        format!("app:{name}")
    }
}

/// The operators one pill stands for: `app:Arc`, `date:yesterday hours:12-18`.
pub fn pill_operators(p: &Pill) -> String {
    match p {
        Pill::App(a) => app_operator_token(a),
        Pill::Source(s) => format!("source:{}", s.op()),
        Pill::Date { value, part, .. } => [
            value.as_ref().map(|v| format!("date:{v}")),
            part.map(|p| format!("hours:{}", p.hours())),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" "),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// What the pill reads as: `Arc`, `Yesterday · afternoon`, `Afternoon`, `On a call`.
pub fn pill_label(p: &Pill) -> String {
    match p {
        Pill::App(a) => a.clone(),
        Pill::Source(Src::Mic) => "On a call".into(),
        Pill::Source(Src::System) => "System audio".into(),
        Pill::Source(Src::Screen) => "Screen".into(),
        Pill::Date {
            value: None, part, ..
        } => capitalize(part.map_or("", Part::name)),
        Pill::Date {
            label,
            part: Some(part),
            ..
        } => format!("{label} · {}", part.name()),
        Pill::Date { label, .. } => label.clone(),
    }
}

/// "Edit as text": the phrase the pill came from.
pub fn pill_edit_text(p: &Pill) -> String {
    match p {
        Pill::App(a) => format!("in {a}"),
        Pill::Source(Src::Mic) => "on a call".into(),
        Pill::Source(Src::System) => "in a video".into(),
        Pill::Source(Src::Screen) => "on screen".into(),
        Pill::Date { value, part, .. } => {
            let day = value.as_ref().map(|v| {
                if v.starts_with(|c: char| c.is_ascii_digit()) {
                    format!("date:{v}")
                } else {
                    v.replacen('-', " ", 1)
                }
            });
            [day, part.map(|p| p.name().to_string())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// The query the backend runs and the footer shows.
pub fn operator_query(text: &str, pills: &[Pill]) -> String {
    std::iter::once(text.trim().to_string())
        .chain(pills.iter().map(pill_operators))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The trailing whitespace-delimited token (empty when the text ends in a space).
pub fn trailing_token(v: &str) -> &str {
    if v.ends_with(char::is_whitespace) {
        return "";
    }
    v.rsplit(char::is_whitespace).next().unwrap_or("")
}

fn token_op(t: &str) -> Option<String> {
    let colon = t.find(':')?;
    Some(t[..=colon].to_ascii_lowercase())
}

/// A half-typed field-operator value (`app:Saf`): it waits for the commit pause.
pub fn is_trailing_operator_partial(raw: &str) -> bool {
    token_op(trailing_token(raw))
        .is_some_and(|op| ["app:", "source:", "date:", "after:", "before:"].contains(&op.as_str()))
}

/// A bare operator name (`app:`): nothing to search yet.
pub fn is_bare_operator(raw: &str) -> bool {
    let t = trailing_token(raw).to_ascii_lowercase();
    ["app:", "source:", "date:", "after:", "before:", "hours:"].contains(&t.as_str())
}

const GHOST_OPERATORS: [&str; 5] = ["app:", "source:", "date:", "after:", "before:"];

fn ghost_values(op: &str) -> &'static [&'static str] {
    match op {
        "source:" => &["mic", "system", "screen"],
        "date:" => &["today", "yesterday", "this-week", "last-week"],
        _ => &[],
    }
}

fn complete_app<'a>(apps: &'a [String], typed: &str) -> Option<&'a String> {
    if typed.is_empty() || typed.starts_with('"') {
        return None;
    }
    let lower = typed.to_lowercase();
    let n = typed.chars().count();
    apps.iter()
        .find(|a| a.to_lowercase().starts_with(&lower) && a.chars().count() > n)
}

fn tail(s: &str, skip: usize) -> String {
    s.chars().skip(skip).collect()
}

/// The inline completion: what to draw after the text, and the text once → accepts it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub shown: String,
    pub accepted: String,
}

/// Operator names, the enumerable `source:`/`date:` values, `app:` values, and an
/// app name after in/on/from/using.
pub fn ghost(query: &str, apps: &[String]) -> Option<Ghost> {
    let token = trailing_token(query);
    if token.is_empty() {
        return None;
    }
    let suffix = |s: String| -> Option<Ghost> {
        (!s.is_empty()).then(|| Ghost {
            accepted: format!("{query}{s}{}", if s.ends_with(':') { "" } else { " " }),
            shown: s,
        })
    };
    let Some(colon) = token.find(':') else {
        if !token.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        static AFTER: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:in|on|from|using)\s+\S+$"));
        if AFTER.is_match(query) {
            if let Some(app) = complete_app(apps, token) {
                return suffix(tail(app, token.chars().count()));
            }
        }
        if token.len() < 2 {
            return None;
        }
        let lower = token.to_ascii_lowercase();
        let op = GHOST_OPERATORS
            .iter()
            .find(|o| o.starts_with(&lower) && o.len() > lower.len())?;
        return suffix(op[token.len()..].to_string());
    };
    let operator = token[..=colon].to_ascii_lowercase();
    let typed = &token[colon + 1..];
    if operator == "app:" {
        let app = complete_app(apps, typed)?;
        if app.contains(' ') {
            let head = &query[..query.len() - token.len()];
            return Some(Ghost {
                shown: format!("{}\"", tail(app, typed.chars().count())),
                accepted: format!("{head}{}\"{app}\" ", &token[..=colon]),
            });
        }
        return suffix(tail(app, typed.chars().count()));
    }
    let lower = typed.to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let v = ghost_values(&operator)
        .iter()
        .find(|v| v.starts_with(&lower) && v.len() > lower.len())?;
    suffix(v[typed.len()..].to_string())
}

static LEAD: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(who|what|when|where|why|how|did|do|does|was|were|is|are|can|could|should|which|summari[sz]e|remind me|tell me)\b",
    )
});

/// A trailing `?`, or a leading who/what/when… (a bare wh-word alone isn't one yet).
pub fn is_question(text: &str) -> bool {
    let t = text.trim();
    if t.ends_with('?') {
        return true;
    }
    if !t.contains(char::is_whitespace)
        && !t.eq_ignore_ascii_case("summarize")
        && !t.eq_ignore_ascii_case("summarise")
    {
        return false;
    }
    LEAD.is_match(t)
}

/// The question sent to Ask AI: the free text with each pill's phrase put back.
pub fn scoped_question(residual: &str, pills: &[Pill]) -> String {
    let text = residual.trim();
    let body = std::iter::once(text.trim_end_matches('?').trim().to_string())
        .chain(pills.iter().map(pill_edit_text))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !body.is_empty() && text.ends_with('?') {
        format!("{body}?")
    } else {
        body
    }
}

/// Case, spacing and trailing punctuation don't count: typing the final "?" never re-asks.
pub fn same_question(a: &str, b: &str) -> bool {
    let norm = |t: &str| {
        let t = t.to_lowercase();
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        t.trim_end_matches(|c: char| c.is_whitespace() || "?.!".contains(c))
            .trim()
            .to_string()
    };
    norm(a) == norm(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forced {
    Ask,
    Search,
}

/// Tab flips what the bar is doing; landing back on what detection would do on
/// its own clears the force.
pub fn next_forced(forced: Option<Forced>, auto_ask: bool) -> Option<Forced> {
    let asking = forced.map_or(auto_ask, |f| f == Forced::Ask);
    let next = if asking { Forced::Search } else { Forced::Ask };
    ((next == Forced::Ask) != auto_ask).then_some(next)
}

#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;
