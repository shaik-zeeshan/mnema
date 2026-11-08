//! Pure display helpers for Chat (ports of `lib/chat/chat-format.ts`, the
//! `conversation.ts` model labels and `markdown.ts`'s `closeDanglingBold`).
//! Everything that formats a time takes the frozen-able "now" for its offset.

use std::borrow::Cow;

use capture_types::{AiProviderConfig, AiProviderKind, ConversationSummary};
use time::{Duration, OffsetDateTime, Time};

const DAY_MS: i64 = 86_400_000;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `ms` (unix) in `now`'s offset, the Mac's local time.
pub fn local(ms: i64, now: OffsetDateTime) -> OffsetDateTime {
    let utc = OffsetDateTime::UNIX_EPOCH + Duration::milliseconds(ms);
    utc.to_offset(now.offset())
}

fn start_of_day_ms(now: OffsetDateTime) -> i64 {
    let midnight = now.replace_time(Time::MIDNIGHT);
    (midnight.unix_timestamp_nanos() / 1_000_000) as i64
}

/// "2:32 PM" (en-US, the Svelte `formatClock` default).
pub fn clock(d: OffsetDateTime) -> String {
    let h = d.hour() % 12;
    let h = if h == 0 { 12 } else { h };
    let ampm = if d.hour() < 12 { "AM" } else { "PM" };
    format!("{h}:{:02} {ampm}", d.minute())
}

/// "Wed 2:32 PM" (`toLocaleString(weekday short, hour numeric, minute 2-digit)`).
pub fn weekday_clock(ms: i64, now: OffsetDateTime) -> String {
    let d = local(ms, now);
    format!(
        "{} {}",
        WEEKDAYS[d.weekday().number_days_from_monday() as usize],
        clock(d)
    )
}

/// A chat-list stamp: the clock today and yesterday, the weekday within the
/// week, else "Sep 30" (the year only when it differs).
pub fn chat_when(ms: i64, now: OffsetDateTime) -> String {
    if ms <= 0 {
        return String::new();
    }
    let d = local(ms, now);
    let start = start_of_day_ms(now);
    if ms >= start - DAY_MS {
        return clock(d);
    }
    if ms >= start - 6 * DAY_MS {
        return WEEKDAYS[d.weekday().number_days_from_monday() as usize].to_string();
    }
    let md = format!("{} {}", MONTHS[d.month() as usize - 1], d.day());
    if d.year() == now.year() {
        md
    } else {
        format!("{md}, {}", d.year())
    }
}

/// History groups: Pinned first, then Today / Yesterday / This week / "May 2026",
/// in first-seen order so the backend's sort holds within each group.
pub fn group_history(
    rows: &[ConversationSummary],
    now: OffsetDateTime,
) -> Vec<(String, Vec<ConversationSummary>)> {
    let start = start_of_day_ms(now);
    let label = |c: &ConversationSummary| -> String {
        let ms = c.updated_at_ms;
        if c.pinned {
            "Pinned".into()
        } else if ms <= 0 {
            "Earlier".into()
        } else if ms >= start {
            "Today".into()
        } else if ms >= start - DAY_MS {
            "Yesterday".into()
        } else if ms >= start - 6 * DAY_MS {
            "This week".into()
        } else {
            let d = local(ms, now);
            format!("{} {}", MONTHS_LONG[d.month() as usize - 1], d.year())
        }
    };
    let mut groups: Vec<(String, Vec<ConversationSummary>)> = Vec::new();
    for c in rows {
        let l = label(c);
        match groups.iter_mut().find(|(g, _)| *g == l) {
            Some((_, items)) => items.push(c.clone()),
            None if l == "Pinned" => groups.insert(0, (l, vec![c.clone()])),
            None => groups.push((l, vec![c.clone()])),
        }
    }
    groups
}

/// "812" / "6.2k" / "14k" / "1.2M".
pub fn format_token_count(tokens: u64) -> String {
    let t = tokens as f64;
    if tokens >= 1_000_000 {
        format!("{:.1}M", t / 1e6)
    } else if tokens >= 10_000 {
        format!("{:.0}k", t / 1e3)
    } else if tokens >= 1_000 {
        format!("{:.1}k", t / 1e3)
    } else {
        tokens.to_string()
    }
}

pub fn greeting(hour: u8) -> &'static str {
    match hour {
        5..=11 => "Good morning.",
        12..=17 => "Good afternoon.",
        _ => "Good evening.",
    }
}

/// "4 steps · 3.4s" (duration only for an answer timed this session); None with no steps.
pub fn steps_line(steps: usize, elapsed_ms: Option<i64>) -> Option<String> {
    if steps == 0 {
        return None;
    }
    let head = format!("{steps} step{}", if steps == 1 { "" } else { "s" });
    Some(match elapsed_ms {
        Some(ms) if ms > 0 => format!("{head} · {:.1}s", ms as f64 / 1000.),
        _ => head,
    })
}

/// Close a `**` the stream hasn't closed yet (last paragraph, outside inline
/// code; an open code fence is left alone). A bare trailing opener is dropped,
/// as is one lone trailing `*`. Streaming display only.
pub fn close_dangling_bold(text: &str) -> Cow<'_, str> {
    let fences = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("```") || t.starts_with("~~~")
        })
        .count();
    if fences % 2 == 1 {
        return Cow::Borrowed(text);
    }
    let para = last_paragraph(text);
    if strip_inline_code(para).matches("**").count() % 2 == 0 {
        return Cow::Borrowed(text);
    }
    // `(?<!\*)\*$`: one lone trailing `*` (half of a closer still arriving).
    let out = if text.ends_with('*') && !text.ends_with("**") {
        &text[..text.len() - 1]
    } else {
        text
    };
    let trimmed = out.trim_end();
    if trimmed.ends_with("**") {
        Cow::Owned(trimmed[..trimmed.len() - 2].to_string())
    } else {
        Cow::Owned(format!("{trimmed}**"))
    }
}

/// `text.split(/\n\s*\n/).pop()`.
fn last_paragraph(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() && bytes[j] != b'\n' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'\n' {
                // A blank line: skip all following whitespace too.
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                start = k;
                i = k;
                continue;
            }
        }
        i += 1;
    }
    &text[start.min(text.len())..]
}

/// `.replace(/`[^`]*(`|$)/g, "")`.
fn strip_inline_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_code = false;
    for ch in s.chars() {
        if ch == '`' {
            in_code = !in_code;
        } else if !in_code {
            out.push(ch);
        }
    }
    out
}

/// The tail after the last "/" (long routing paths read cleanly).
pub fn short_model_label(model: &str) -> &str {
    let t = model.trim();
    match t.rsplit('/').next().map(str::trim) {
        Some(tail) if !tail.is_empty() => tail,
        _ => t,
    }
}

fn title_case(w: &str) -> String {
    let mut c = w.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
        .unwrap_or_default()
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_alpha(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic())
}

/// "claude-sonnet-4-5-20250929" → "Claude Sonnet 4.5", "gpt-5-mini" → "GPT-5 mini";
/// unknown ids come back as their short label.
pub fn model_display_name(model: &str) -> String {
    let short = short_model_label(model);
    let mut parts: Vec<&str> = short.split('-').collect();
    // Strip a date / "latest" suffix: -YYYYMMDD, -YYYY-MM-DD, -latest.
    let n = parts.len();
    if n > 1
        && (parts[n - 1].eq_ignore_ascii_case("latest")
            || (is_digits(parts[n - 1]) && parts[n - 1].len() == 8))
    {
        parts.pop();
    } else if n > 3
        && [4, 2, 2] == [parts[n - 3].len(), parts[n - 2].len(), parts[n - 1].len()]
        && parts[n - 3..].iter().all(|p| is_digits(p))
    {
        parts.truncate(n - 3);
    }
    let claude = |family: &str, major: &str, minor: Option<&str>| {
        let minor = minor.map(|m| format!(".{m}")).unwrap_or_default();
        format!("Claude {} {major}{minor}", title_case(family))
    };
    if parts
        .first()
        .is_some_and(|p| p.eq_ignore_ascii_case("claude"))
    {
        match parts.as_slice() {
            [_, f, a] if is_alpha(f) && is_digits(a) => return claude(f, a, None),
            [_, f, a, b] if is_alpha(f) && is_digits(a) && is_digits(b) => {
                return claude(f, a, Some(b))
            }
            [_, a, f] if is_digits(a) && is_alpha(f) => return claude(f, a, None),
            [_, a, b, f] if is_digits(a) && is_digits(b) && is_alpha(f) => {
                return claude(f, a, Some(b))
            }
            _ => {}
        }
    }
    if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("gpt") {
        let v = parts[1];
        let core = v.strip_suffix('o').unwrap_or(v);
        if !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            let rest = parts[2..].join(" ");
            return if rest.is_empty() {
                format!("GPT-{v}")
            } else {
                format!("GPT-{v} {rest}")
            };
        }
    }
    short.to_string()
}

/// Known-family context windows (Anthropic/OpenAI never advertise one).
pub fn context_window_for_model(model: &str) -> Option<u64> {
    let t = short_model_label(model).to_lowercase();
    let table: [(&str, u64); 6] = [
        ("claude", 200_000),
        ("gpt-5", 400_000),
        ("gpt-4.1", 1_000_000),
        ("gpt-4o", 128_000),
        ("gemini", 1_048_576),
        ("deepseek", 128_000),
    ];
    table.iter().find(|(k, _)| t.contains(k)).map(|(_, v)| *v)
}

pub fn kind_label(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Anthropic => "Anthropic",
        AiProviderKind::Openai => "OpenAI",
        AiProviderKind::OpenaiCompatible => "OpenAI-compatible",
        AiProviderKind::Chatgpt => "ChatGPT",
        AiProviderKind::Ollama => "Ollama",
        AiProviderKind::Llamafile => "Llamafile",
    }
}

/// Host of a base URL ("localhost:11434"), else the trimmed text.
pub fn url_host(url: &str) -> String {
    let t = url.trim();
    let rest = t.split_once("://").map_or(t, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// A provider instance's display name: its label, else kind + host, else kind (n).
pub fn provider_instance_label(p: &AiProviderConfig) -> String {
    if !p.label.trim().is_empty() {
        return p.label.trim().to_string();
    }
    let kind = kind_label(p.kind);
    let host = url_host(&p.base_url);
    if !host.is_empty() {
        return format!("{kind} · {host}");
    }
    match p.id.strip_prefix(&format!("{}-", p.kind.id())) {
        Some(n) if !n.is_empty() => format!("{kind} ({n})"),
        _ => kind.to_string(),
    }
}

pub fn provider_label_by_id(providers: &[AiProviderConfig], id: &str) -> String {
    match providers.iter().find(|p| p.id == id) {
        Some(p) => provider_instance_label(p),
        None => {
            AiProviderKind::from_id(id).map_or_else(|| id.to_string(), |k| kind_label(k).into())
        }
    }
}

/// Where a provider runs: local runtimes or a loopback endpoint, else cloud.
pub fn provider_is_local(p: Option<&AiProviderConfig>) -> bool {
    let Some(p) = p else { return false };
    if p.kind.is_local() {
        return true;
    }
    let host = url_host(&p.base_url).to_lowercase();
    ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|h| host == *h || host.starts_with(&format!("{h}:")))
}

/// How a provider is reached ("your API key", "localhost:11434", …).
pub fn provider_via(p: Option<&AiProviderConfig>) -> String {
    let Some(p) = p else { return String::new() };
    if p.kind == AiProviderKind::Chatgpt {
        return "your ChatGPT sign-in".into();
    }
    if !p.base_url.trim().is_empty() {
        return url_host(&p.base_url);
    }
    if p.kind.is_local() {
        "on this Mac".into()
    } else {
        "your API key".into()
    }
}
