//! The developer window's Capture, AI and Logs readouts, ported from
//! `lib/debug/{capture-view,ai-view,log-line,log-filter}.ts`.

use serde_json::Value;

use crate::settings::model::Json;
use crate::ui::Tone;

// ── Capture ─────────────────────────────────────────────────────────────────

/// Lane state → (word, tone).
pub fn lane_word(state: &str) -> (&'static str, Option<Tone>) {
    match state {
        "recording" => ("Recording", None),
        "paused" => ("Paused", Some(Tone::Info)),
        "waiting" => ("Waiting", Some(Tone::Info)),
        "reconnecting" => ("Reconnecting", Some(Tone::Warn)),
        "failed" => ("Failed", Some(Tone::Danger)),
        _ => ("Off", None),
    }
}

pub fn reason_phrase(code: &str) -> Option<&'static str> {
    Some(match code {
        "capture_display_unavailable" => "display asleep",
        "privacy_filter_apply_failed" => "privacy filter retrying",
        "privacy_recovery_restart_required" => "privacy filter needs a restart",
        "capture_low_disk" => "disk almost full",
        "not_requested" => "turned off in Settings",
        "user_paused" => "paused by you",
        "inactivity" => "nothing happening",
        _ => return None,
    })
}

/// A recording lane names its device; any other state leads with why.
pub fn lane_detail(src: &Value, rate_hz: Option<f64>) -> String {
    let state = src.str_at(&["state"]).unwrap_or("off");
    let why = src.str_at(&["reason"]).and_then(reason_phrase);
    let device = src.str_at(&["deviceName"]);
    let first = if state == "recording" {
        device.or(why)
    } else {
        why.or(device)
    };
    let mut parts: Vec<String> = first.into_iter().map(str::to_string).collect();
    if let Some(hz) = rate_hz.filter(|_| state != "off") {
        parts.push(format!("{} kHz", (hz / 1000.).round()));
    }
    parts.join(" \u{b7} ")
}

/// Lit bars on the 12-bar meter: clamp((dB + 60) / 60 · 12).
pub fn meter_bars(level_db: Option<f64>) -> usize {
    level_db.map_or(0, |db| {
        (((db + 60.) / 60. * 12.).round().clamp(0., 12.)) as usize
    })
}

/// The tap spec's rebuild reason in sentence case, plus its mono detail. A
/// sample-rate drop across 16 kHz reads as the headset switching to call quality.
pub fn rebuild_reason(reason: Option<&str>, detail: Option<&str>) -> String {
    let text = match reason {
        Some("deviceChanged") => "Output device changed",
        Some("deviceDied") => "Output device went away",
        Some("sampleRateChanged") => "Sample rate changed",
        Some("excludeListMoved") => "Kept-out apps moved",
        Some("silenceWatchdog") => "No sound for a while",
        _ => return "\u{2014}".into(),
    };
    let rates = detail
        .and_then(|d| d.strip_suffix(" Hz"))
        .and_then(|d| d.split_once(" \u{2192} "))
        .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)));
    if let (Some("sampleRateChanged"), Some((a, b))) = (reason, rates) {
        let k = |hz: f64| format!("{}", (hz / 100.).round() / 10.);
        let short = format!("{} \u{2192} {} kHz", k(a), k(b));
        let label = if b <= 16_000. && a > 16_000. {
            "Headset switched to call quality"
        } else if a <= 16_000. && b > 16_000. {
            "Headset back to full quality"
        } else {
            text
        };
        return format!("{label}, {short}");
    }
    detail.map_or(text.to_string(), |d| format!("{text}, {d}"))
}

/// "30 s" / "2 min".
pub fn format_seconds(ms: f64) -> String {
    let s = (ms / 1000.).round().max(0.);
    if s < 120. {
        format!("{s} s")
    } else {
        format!("{} min", (s / 60.).round())
    }
}

const GIB: f64 = 1024. * 1024. * 1024.;
pub const DISK_SCALE_BYTES: f64 = 25. * GIB;

/// Bare GB the way the mockup writes them: "182", "41.2", "5".
pub fn gb(bytes: f64) -> String {
    let v = bytes / GIB;
    if v >= 100. {
        format!("{}", v.round())
    } else {
        let r = (v * 10.).round() / 10.;
        if r.fract() == 0. {
            format!("{r:.0}")
        } else {
            format!("{r:.1}")
        }
    }
}

pub fn disk_at(bytes: f64) -> f32 {
    (bytes / DISK_SCALE_BYTES).clamp(0., 1.) as f32
}

// ── AI ──────────────────────────────────────────────────────────────────────

/// A stat value split into number and unit: 18240 → ("18.2", "k").
pub fn token_stat(n: f64) -> (String, &'static str) {
    let trim = |s: String| s.strip_suffix(".0").map(str::to_string).unwrap_or(s);
    if n < 1000. {
        (format!("{n}"), "")
    } else if n < 999_950. {
        (trim(format!("{:.1}", n / 1000.)), "k")
    } else {
        (trim(format!("{:.1}", n / 1_000_000.)), "M")
    }
}

pub fn duration_stat(ms: f64) -> (String, &'static str) {
    if ms < 999.5 {
        (format!("{}", ms.round()), "ms")
    } else if ms < 59_950. {
        (format!("{:.1}", ms / 1000.), "s")
    } else {
        (format!("{:.1}", ms / 60_000.), "min")
    }
}

/// `anthropic-2` → "Anthropic".
pub fn provider_label(id: &str) -> String {
    let kind = id
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('-');
    match kind {
        "anthropic" => "Anthropic".into(),
        "openai" => "OpenAI".into(),
        "openai_compatible" | "openaiCompatible" => "OpenAI-compatible".into(),
        "ollama" => "Ollama".into(),
        "llamafile" => "Llamafile".into(),
        "chatgpt" => "ChatGPT".into(),
        other => other.to_string(),
    }
}

/// Engine status copy + tone (`engineView`), or `None` while checking.
pub fn engine_view(
    status: Option<&Value>,
) -> Option<(Tone, String, Option<String>, Option<String>)> {
    let s = status?;
    let model = s.at(&["defaultModel"]);
    let provider = s
        .str_at(&["defaultProviderKind"])
        .map(provider_label)
        .or_else(|| model.str_at(&["provider"]).map(provider_label));
    let model_id = model.str_at(&["model"]).map(str::to_string);
    let reason = s.str_at(&["reason"]).unwrap_or("");
    let (tone, title) = if !s.bool_at(&["enabled"]) || reason == "ai_runtime_disabled" {
        (Tone::Off, "AI is turned off".to_string())
    } else if !s.bool_at(&["hasProviders"]) {
        (Tone::Off, "AI isn\u{2019}t set up".into())
    } else if s.bool_at(&["available"]) || reason.is_empty() {
        (Tone::Ok, "AI engine is available".into())
    } else if let Some(id) = reason.strip_prefix("provider_unreachable:") {
        (
            Tone::Warn,
            format!(
                "{} can\u{2019}t be reached, resumes on its own",
                provider_label(id)
            ),
        )
    } else if let Some(id) = reason.strip_prefix("needs_reconnect:") {
        (
            Tone::Danger,
            format!(
                "{} signed you out, sign in again in Settings",
                provider_label(id)
            ),
        )
    } else {
        (Tone::Info, "AI isn\u{2019}t ready".into())
    };
    Some((tone, title, provider, model_id))
}

// ── Logs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub time: String,
    pub level: String,
    pub prefix: String,
    pub message: String,
}

/// One head line in either writer's shape; `None` for a continuation line:
/// ours (`logging.rs`) `[2026-11-10T13:48:12.527282Z][WARN][target] message`,
/// Tauri's `[2026-10-13][15:01:26][target][INFO] message`.
/// rust.log stamps UTC; `offset_s` turns it into the local wall clock.
pub fn parse_log_line(raw: &str, offset_s: i64) -> Option<LogLine> {
    let rest = raw.strip_prefix('[')?;
    let (date, clock, target, level, text) = match rest.split_once(']')? {
        (stamp, rest) if stamp.contains('T') => {
            let (date, clock) = stamp.strip_suffix('Z').unwrap_or(stamp).split_once('T')?;
            let (level, rest) = rest.strip_prefix('[')?.split_once("][")?;
            let (target, text) = rest.split_once(']')?;
            (date, clock, target, level, text)
        }
        _ => {
            let (date, rest) = rest.split_once("][")?;
            let (clock, rest) = rest.split_once("][")?;
            let (target, rest) = rest.split_once("][")?;
            let (level, text) = rest.split_once(']')?;
            (date, clock, target, level, text)
        }
    };
    if date.len() != 10 || level.is_empty() || !level.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let (hms, frac) = clock
        .split_once('.')
        .map_or((clock, None), |(a, b)| (a, Some(b)));
    let mut it = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, s) = (it.next()??, it.next()??, it.next()??);
    let secs = (h * 3600 + m * 60 + s + offset_s).rem_euclid(86_400);
    let mut time = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if let Some(f) = frac {
        time.push_str(&format!(".{:0<3}", &f[..f.len().min(3)]));
    }
    let text = text.strip_prefix(' ').unwrap_or(text);
    let last = target.rsplit("::").next().unwrap_or(target);
    let (prefix, message) = match text.split_once(char::is_whitespace) {
        Some((tag, msg))
            if (tag.starts_with('[') && tag.ends_with(']'))
                || (tag.ends_with(':') && tag.len() > 1) =>
        {
            (tag.to_string(), msg.trim_start().to_string())
        }
        _ => (last.to_string(), text.to_string()),
    };
    Some(LogLine {
        time,
        level: level.to_string(),
        prefix,
        message,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub head: Option<LogLine>,
    pub more: Vec<String>,
    pub raw: String,
}

/// Group raw lines (oldest first) so a backtrace stays under its line.
pub fn group_log_lines(lines: &[String], offset_s: i64) -> Vec<LogEntry> {
    let mut out: Vec<LogEntry> = Vec::new();
    for raw in lines {
        match (parse_log_line(raw, offset_s), out.last_mut()) {
            (None, Some(last)) => {
                last.more.push(raw.clone());
                last.raw.push('\n');
                last.raw.push_str(raw);
            }
            (head, _) => out.push(LogEntry {
                more: if head.is_none() {
                    vec![raw.clone()]
                } else {
                    vec![]
                },
                head,
                raw: raw.clone(),
            }),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    All,
    Warn,
    Error,
}

pub fn level_matches(level: Option<&str>, f: Level) -> bool {
    match f {
        Level::All => true,
        Level::Error => level == Some("ERROR"),
        Level::Warn => matches!(level, Some("WARN" | "ERROR")),
    }
}

/// The feature chips: `(label, keywords)`; keyword hints over the raw text.
pub const LOG_FEATURES: [(&str, &[&str]); 10] = [
    ("Everything", &[]),
    (
        "Capture",
        &[
            "capture",
            "segment",
            "session",
            "frame",
            "display",
            "screen",
            "recording",
            "microphone",
            "audio",
        ],
    ),
    (
        "Privacy",
        &[
            "privacy",
            "exclusion",
            "excluded",
            "redact",
            "inactivity",
            "idle",
        ],
    ),
    (
        "Reading",
        &["ocr", "recognition", "recognizer", "vision", "tesseract"],
    ),
    ("Transcription", &["transcri", "whisper", "deepgram"]),
    ("Speakers", &["diariz", "speaker", "speakrs"]),
    (
        "Meaning",
        &[
            "embed",
            "semantic",
            "vector",
            "index",
            "backfill",
            "quarantin",
        ],
    ),
    (
        "AI",
        &[
            "ai runtime",
            "ask ai",
            "ai_runtime",
            "ask_ai",
            "anthropic",
            "openai",
            "ollama",
            "llamafile",
        ],
    ),
    (
        "User Context",
        &[
            "user context",
            "user_context",
            "distill",
            "derivation",
            "subject",
            "belief",
            "digest",
        ],
    ),
    (
        "Jobs",
        &[
            "job",
            "worker",
            "migration",
            "sqlite",
            "database",
            "storage",
            "cleanup",
            "retention",
        ],
    ),
];

pub fn filter_entries<'a>(
    entries: &'a [LogEntry],
    level: Level,
    feature: usize,
    needle: &str,
) -> Vec<&'a LogEntry> {
    let words = LOG_FEATURES.get(feature).map_or(&[][..], |f| f.1);
    let q = needle.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| {
            let raw = e.raw.to_lowercase();
            level_matches(e.head.as_ref().map(|h| h.level.as_str()), level)
                && (words.is_empty() || words.iter().any(|w| raw.contains(w)))
                && (q.is_empty() || raw.contains(&q))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capture_and_ai_helpers() {
        assert_eq!(meter_bars(Some(-30.)), 6);
        assert_eq!(meter_bars(None), 0);
        assert_eq!(gb(182. * GIB), "182");
        assert_eq!(gb(5. * GIB), "5");
        assert_eq!(gb(41.24 * GIB), "41.2");
        let src = json!({"state": "waiting", "reason": "capture_display_unavailable", "deviceName": "Built-in"});
        assert_eq!(lane_detail(&src, None), "display asleep");
        assert_eq!(token_stat(18240.), ("18.2".into(), "k"));
        assert_eq!(duration_stat(4200.), ("4.2".into(), "s"));
        assert_eq!(provider_label("anthropic-2"), "Anthropic");
        assert_eq!(
            rebuild_reason(Some("sampleRateChanged"), Some("48000 \u{2192} 16000 Hz")),
            "Headset switched to call quality, 48 \u{2192} 16 kHz"
        );
        assert_eq!(format_seconds(30_000.), "30 s");
    }

    #[test]
    fn log_lines_parse_group_and_filter() {
        let l = parse_log_line(
            "[2026-11-04][09:01:26][capture_system_audio::rebuild][WARN] system-audio-tap: rebuilt",
            19_800,
        )
        .unwrap();
        assert_eq!(l.time, "14:31:26");
        assert_eq!(l.prefix, "system-audio-tap:");
        assert_eq!(l.message, "rebuilt");
        let plain = parse_log_line(
            "[2026-11-04][09:01:26.5][mnema::jobs][INFO] worker started",
            0,
        )
        .unwrap();
        assert_eq!(
            (plain.time.as_str(), plain.prefix.as_str()),
            ("09:01:26.500", "jobs")
        );
        let lines: Vec<String> = [
            "[2026-11-04][09:00:00][a::b][ERROR] boom",
            "  at frame 1",
            "[2026-11-04][09:00:01][a::ocr][INFO] ocr done",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let g = group_log_lines(&lines, 0);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].more, vec!["  at frame 1".to_string()]);
        assert_eq!(filter_entries(&g, Level::Error, 0, "").len(), 1);
        assert_eq!(filter_entries(&g, Level::All, 3, "").len(), 1);
        assert_eq!(filter_entries(&g, Level::All, 0, "FRAME 1").len(), 1);
    }

    /// The shape `logging.rs` writes: one row per line, not one merged entry.
    #[test]
    fn our_own_log_lines_are_one_row_each() {
        let lines: Vec<String> = [
            "[2026-11-10T13:48:12.157462Z][INFO][mnema_gpui] mnema-gpui starting",
            "[2026-11-10T13:48:12.527282Z][WARN][gpui_apple::text_system] skipping duplicate font",
            "[2026-11-10T13:48:13.000001Z][ERROR][app_core::startup] worker failed: io error",
            "stack backtrace:",
            "   0: std::backtrace::Backtrace::create",
            "[2026-11-10T23:59:59Z][INFO][app_infra::jobs] [app-infra] worker started",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let g = group_log_lines(&lines, 19_800);
        assert_eq!(g.len(), 4);
        let warn = g[1].head.as_ref().unwrap();
        assert_eq!(
            (
                warn.time.as_str(),
                warn.level.as_str(),
                warn.prefix.as_str()
            ),
            ("19:18:12.527", "WARN", "text_system")
        );
        assert_eq!(warn.message, "skipping duplicate font");
        // The multi-line message keeps its continuation lines under its head.
        assert_eq!(g[2].head.as_ref().unwrap().level, "ERROR");
        assert_eq!(g[2].more.len(), 2);
        let last = g[3].head.as_ref().unwrap();
        assert_eq!(
            (last.time.as_str(), last.prefix.as_str()),
            ("05:29:59", "[app-infra]")
        );
        assert_eq!(filter_entries(&g, Level::Warn, 0, "").len(), 2);
        assert!(parse_log_line("[2026-11-10T13:48:12Z][][x] no level", 0).is_none());
    }
}
