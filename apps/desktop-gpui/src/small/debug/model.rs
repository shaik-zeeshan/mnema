//! Pure mapping for the developer window, ported from `lib/debug/{verdict,
//! health-view,pipeline-view,capture-view,ai-view,log-line,log-filter}.ts`.
//! Inputs are the commands' camelCase JSON, as the Svelte stub serves them.

use serde_json::Value;

use crate::settings::model::Json;
use crate::ui::Tone;

/// Severity → tone. `error` is the only rename; anything unknown reads as ok.
pub fn tone_of(severity: &str) -> Tone {
    match severity {
        "error" => Tone::Danger,
        "warn" => Tone::Warn,
        "info" => Tone::Info,
        "off" => Tone::Off,
        _ => Tone::Ok,
    }
}

fn rank(t: Tone) -> u8 {
    match t {
        Tone::Danger => 0,
        Tone::Warn => 1,
        Tone::Info => 2,
        Tone::Off => 3,
        _ => 4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Pipeline,
    Capture,
    Ai,
    Logs,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Pipeline,
        Tab::Capture,
        Tab::Ai,
        Tab::Logs,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Pipeline => "Pipeline",
            Tab::Capture => "Capture",
            Tab::Ai => "AI",
            Tab::Logs => "Logs",
        }
    }
}

/// The plain name and home tab of each of the 12 health rows.
pub fn feature_row(feature: &str) -> (&str, Tab) {
    match feature {
        "screen" => ("Screen", Tab::Capture),
        "microphone" => ("Microphone", Tab::Capture),
        "systemAudio" => ("System audio", Tab::Capture),
        "reading" => ("Reading", Tab::Pipeline),
        "transcription" => ("Transcription", Tab::Pipeline),
        "speakers" => ("Speakers", Tab::Pipeline),
        "meaningIndex" => ("Meaning index", Tab::Pipeline),
        "aiEngine" => ("AI engine", Tab::Ai),
        "userContext" => ("User Context", Tab::Ai),
        "disk" => ("Disk", Tab::Capture),
        "cliAccess" => ("CLI access", Tab::Ai),
        "startup" => ("Startup", Tab::Logs),
        other => (other, Tab::Overview),
    }
}

const CAPTURE: [&str; 3] = ["screen", "microphone", "systemAudio"];

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub tone: Tone,
    pub text: String,
}

fn sev(r: &Value) -> &str {
    r.str_at(&["severity"]).unwrap_or("ok")
}

fn feat(r: &Value) -> &str {
    r.str_at(&["feature"]).unwrap_or("")
}

pub fn verdict(rows: &[Value]) -> Verdict {
    let bad: Vec<_> = rows
        .iter()
        .filter(|r| matches!(sev(r), "warn" | "error"))
        .collect();
    if !bad.is_empty() {
        let n = bad.len();
        return Verdict {
            tone: if bad.iter().any(|r| sev(r) == "error") {
                Tone::Danger
            } else {
                Tone::Warn
            },
            text: format!(
                "{n} {} a look",
                if n == 1 { "thing needs" } else { "things need" }
            ),
        };
    }
    let info = rows.iter().find(|r| sev(r) == "info");
    let capture: Vec<_> = rows.iter().filter(|r| CAPTURE.contains(&feat(r))).collect();
    if !capture.is_empty() && capture.iter().all(|r| sev(r) == "off") {
        let text = if info.is_some() {
            "Not recording \u{b7} finishing earlier work"
        } else {
            "Not recording"
        };
        return Verdict {
            tone: Tone::Off,
            text: text.into(),
        };
    }
    if let Some(i) = info {
        return Verdict {
            tone: Tone::Info,
            text: format!(
                "{}: {}",
                feature_row(feat(i)).0,
                i.str_at(&["reason"]).unwrap_or("")
            ),
        };
    }
    Verdict {
        tone: Tone::Ok,
        text: "Everything is running".into(),
    }
}

/// A failing read wins over the rows: a stale "Everything is running" lies.
pub fn health_verdict(rows: &[Value], error: Option<&str>) -> Verdict {
    if error.is_some() {
        return Verdict {
            tone: Tone::Danger,
            text: "Can\u{2019}t read health".into(),
        };
    }
    if rows.is_empty() {
        Verdict {
            tone: Tone::Off,
            text: "Checking\u{2026}".into(),
        }
    } else {
        verdict(rows)
    }
}

/// "2h 14m" / "14m".
pub fn format_running(ms: i64) -> String {
    let m = (ms / 60_000).max(0);
    if m >= 60 {
        format!("{}h {}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub feature: String,
    pub tone: Tone,
    pub name: String,
    pub why: String,
    pub metric: String,
    /// Where a click goes.
    pub tab: Tab,
}

/// `metricAtMs` wins as a local clock ("used 14:28" for the AI engine).
pub fn row_view(r: &Value, clock: &dyn Fn(i64) -> String) -> RowView {
    let feature = feat(r).to_string();
    let (name, tab) = feature_row(&feature);
    let metric = match r.num_at(&["metricAtMs"]) {
        Some(at) => format!(
            "{}{}",
            if feature == "aiEngine" { "used " } else { "" },
            clock(at as i64)
        ),
        None => r.str_at(&["metric"]).unwrap_or("").to_string(),
    };
    RowView {
        tone: tone_of(sev(r)),
        name: name.to_string(),
        why: r.str_at(&["reason"]).unwrap_or("").to_string(),
        metric,
        tab,
        feature,
    }
}

/// "Needs a look" · "Right now" · "Running fine", each ranked; empty groups dropped.
pub fn group(rows: &[Value], clock: &dyn Fn(i64) -> String) -> Vec<(&'static str, Vec<RowView>)> {
    let mut all: Vec<_> = rows.iter().map(|r| row_view(r, clock)).collect();
    all.sort_by_key(|r| rank(r.tone));
    let pick = |tones: &[Tone]| -> Vec<RowView> {
        all.iter()
            .filter(|r| tones.contains(&r.tone))
            .cloned()
            .collect()
    };
    [
        ("Needs a look", pick(&[Tone::Danger, Tone::Warn])),
        ("Right now", pick(&[Tone::Info, Tone::Off])),
        ("Running fine", pick(&[Tone::Ok])),
    ]
    .into_iter()
    .filter(|(_, r)| !r.is_empty())
    .collect()
}

/// The mini view's eight dots.
pub const MINI_DOTS: [(&str, &str); 8] = [
    ("screen", "Screen"),
    ("microphone", "Mic"),
    ("systemAudio", "Sys audio"),
    ("reading", "Reading"),
    ("transcription", "Transcription"),
    ("speakers", "Speakers"),
    ("meaningIndex", "Meaning"),
    ("aiEngine", "AI"),
];

pub fn severity_for<'a>(rows: &'a [Value], feature: &str) -> Option<&'a str> {
    rows.iter().find(|r| feat(r) == feature).map(sev)
}

/// The mini footer: the worst row as "Name: why", else the verdict sentence.
pub fn mini_footer(rows: &[Value], error: Option<&str>) -> Verdict {
    if error.is_some() || rows.is_empty() {
        return health_verdict(rows, error);
    }
    let v = verdict(rows);
    if matches!(v.tone, Tone::Danger | Tone::Warn | Tone::Info) {
        let mut all: Vec<_> = rows
            .iter()
            .map(|r| row_view(r, &|_| String::new()))
            .collect();
        all.sort_by_key(|r| rank(r.tone));
        if let Some(w) = all.first() {
            return Verdict {
                tone: v.tone,
                text: format!("{}: {}", w.name, w.why),
            };
        }
    }
    v
}

// ── Pipeline ────────────────────────────────────────────────────────────────

pub const BAR_FULL_JOBS: f32 = 50.;
pub const BAR_FULL_PASSAGES: f32 = 500.;

/// `toLocaleString()` for counts: 12,480.
pub fn count(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeLine {
    pub text: String,
    pub tone: Option<Tone>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowNode {
    pub name: &'static str,
    pub big: String,
    pub unit: &'static str,
    pub lines: Vec<NodeLine>,
    pub bar: Option<f32>,
    pub tone: Option<Tone>,
}

fn nl(text: impl Into<String>, tone: Option<Tone>) -> NodeLine {
    NodeLine {
        text: text.into(),
        tone,
    }
}

pub fn is_recording(rows: &[Value]) -> bool {
    CAPTURE
        .iter()
        .any(|f| severity_for(rows, f).is_some_and(|s| s != "off"))
}

/// A processor's lane, zero-defaulted (a lane with no jobs is absent).
pub fn lane<'a>(lanes: &'a [Value], processor: &str) -> Option<&'a Value> {
    lanes
        .iter()
        .find(|l| l.str_at(&["processor"]) == Some(processor))
}

fn int(v: Option<&Value>, k: &str) -> i64 {
    v.and_then(|v| v.num_at(&[k])).unwrap_or(0.) as i64
}

fn capture_node(rows: &[Value]) -> FlowNode {
    let tones: Vec<(Tone, &str)> = [
        ("screen", "screen"),
        ("microphone", "mic"),
        ("systemAudio", "sys audio"),
    ]
    .iter()
    .map(|(f, short)| (tone_of(severity_for(rows, f).unwrap_or("off")), *short))
    .collect();
    let live = tones
        .iter()
        .filter(|(t, _)| matches!(t, Tone::Ok | Tone::Warn))
        .count();
    let mut node = FlowNode {
        name: "Capture",
        big: live.to_string(),
        unit: if live == 1 {
            "source live"
        } else {
            "sources live"
        },
        lines: vec![],
        bar: None,
        tone: None,
    };
    if !is_recording(rows) {
        node.lines.push(nl("not recording", None));
        return node;
    }
    let word = |t: Tone| match t {
        Tone::Danger => Some("failed"),
        Tone::Warn => Some("recovering"),
        Tone::Info => Some("waits"),
        _ => None,
    };
    if let Some((t, short)) = tones
        .iter()
        .filter(|(t, _)| word(*t).is_some())
        .min_by_key(|(t, _)| rank(*t))
    {
        node.lines.push(nl(*short, Some(*t)));
        node.lines.push(nl(word(*t).unwrap_or(""), Some(*t)));
    }
    node
}

fn lane_node(name: &'static str, l: Option<&Value>, recording: bool) -> FlowNode {
    let (waiting, retrying, running, failed) = (
        int(l, "queued"),
        int(l, "retrying"),
        int(l, "running"),
        int(l, "failedLast24h"),
    );
    let first = if retrying > 0 {
        nl(format!("{} retrying", count(retrying)), Some(Tone::Warn))
    } else if running > 0 || waiting > 0 {
        nl(format!("{} running", count(running)), None)
    } else {
        nl("idle", None)
    };
    let second = if failed > 0 {
        nl(format!("{} failed", count(failed)), Some(Tone::Danger))
    } else {
        nl("0 failed", None)
    };
    let tone = if retrying > 0 {
        Some(Tone::Warn)
    } else if failed > 0 {
        Some(Tone::Danger)
    } else if !recording && waiting > 0 {
        Some(Tone::Info)
    } else {
        None
    };
    FlowNode {
        name,
        big: count(waiting),
        unit: "waiting",
        lines: vec![first, second],
        bar: Some((waiting as f32 / BAR_FULL_JOBS).min(1.)),
        tone,
    }
}

pub fn flow_nodes(rows: &[Value], lanes: &[Value], index: Option<&Value>) -> Vec<FlowNode> {
    let rec = is_recording(rows);
    let behind = int(index, "backlogCount");
    vec![
        capture_node(rows),
        lane_node("Reading", lane(lanes, "ocr"), rec),
        lane_node("Transcription", lane(lanes, "audio_transcription"), rec),
        lane_node("Speakers", lane(lanes, "speaker_analysis"), rec),
        FlowNode {
            name: "Meaning",
            big: index.map_or("\u{2014}".into(), |_| count(behind)),
            unit: "behind",
            lines: vec![nl(
                index.map_or("\u{2014}".into(), |i| {
                    format!("{} done", count(int(Some(i), "vectorCount")))
                }),
                None,
            )],
            bar: Some((behind as f32 / BAR_FULL_PASSAGES).min(1.)),
            tone: (!rec && behind > 0).then_some(Tone::Info),
        },
    ]
}

fn provider_name(p: &str) -> String {
    match p {
        "apple_vision" => "Apple Vision".into(),
        "tesseract" => "Tesseract".into(),
        "parakeet" => "Parakeet".into(),
        "local_whisper" => "Whisper".into(),
        "apple_speech_on_device" => "Apple Speech".into(),
        "deepgram" => "Deepgram".into(),
        "speakrs" => "speakrs".into(),
        other => other.replace('_', " "),
    }
}

/// `engineWords`: "Parakeet, on this Mac" / "Deepgram, cloud" (· can't be
/// reached while its jobs wait on a transient error); `lane` = `None` skips that.
pub fn engine_words(
    spec: &super::detail::Spec,
    settings: Option<&Value>,
    lane: Option<&Value>,
) -> String {
    let Some(c) = settings.map(|s| s.at(&[spec.cfg])).filter(|c| !c.is_null()) else {
        return String::new();
    };
    let provider = c.str_at(&["provider"]).unwrap_or("");
    let label = if spec.key == "meaningIndex" {
        c.str_at(&["modelId"])
            .unwrap_or("Meaning index")
            .to_string()
    } else {
        provider_name(provider)
    };
    if provider != "deepgram" {
        return format!("{label}, on this Mac");
    }
    let unreachable = lane.is_some_and(|l| {
        int(Some(l), "retrying") > 0 && super::detail::is_transient(l.str_at(&["lastError"]))
    });
    if unreachable {
        format!("{label}, cloud \u{b7} can\u{2019}t be reached")
    } else {
        format!("{label}, cloud")
    }
}

/// The four processor rows: `(name, tone, engine words, avg)`.
pub fn processor_rows(rows: &[Value], lanes: &[Value], settings: Option<&Value>) -> Vec<RowView> {
    super::detail::SPECS
        .iter()
        .map(|spec| {
            let l = spec.processor.and_then(|p| lane(lanes, p));
            let metric = l
                .and_then(|l| l.num_at(&["averageCompletedSecondsLast24h"]))
                .map_or(String::new(), |s| format!("{} avg", avg_time(s)));
            RowView {
                feature: spec.key.to_string(),
                tone: tone_of(severity_for(rows, spec.key).unwrap_or("ok")),
                name: spec.name.to_string(),
                why: engine_words(spec, settings, l),
                metric,
                tab: Tab::Pipeline,
            }
        })
        .collect()
}

/// "400 ms" / "6.2 s" / "11 s".
pub fn avg_time(s: f64) -> String {
    if s < 0.1 {
        format!("{} ms", (s * 1000.).round())
    } else if s < 10. {
        format!("{s:.1} s")
    } else {
        format!("{} s", s.round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(f: &str, s: &str, why: &str) -> Value {
        json!({ "feature": f, "severity": s, "reason": why, "metric": null, "metricAtMs": null })
    }

    #[test]
    fn verdict_names_problems_then_off_then_info() {
        let ok = vec![row("screen", "ok", "a"), row("reading", "ok", "b")];
        assert_eq!(verdict(&ok).text, "Everything is running");
        let warn = vec![
            row("screen", "ok", ""),
            row("systemAudio", "warn", "x"),
            row("reading", "error", "y"),
        ];
        assert_eq!(
            verdict(&warn),
            Verdict {
                tone: Tone::Danger,
                text: "2 things need a look".into()
            }
        );
        let off = vec![
            row("screen", "off", ""),
            row("microphone", "off", ""),
            row("reading", "info", "draining"),
        ];
        assert_eq!(
            verdict(&off).text,
            "Not recording \u{b7} finishing earlier work"
        );
        assert_eq!(health_verdict(&[], None).text, "Checking\u{2026}");
        assert_eq!(
            health_verdict(&ok, Some("boom")).text,
            "Can\u{2019}t read health"
        );
        assert_eq!(mini_footer(&warn, None).text, "Reading: y");
        let g = group(&warn, &|_| String::new());
        assert_eq!(g[0].0, "Needs a look");
        assert_eq!(g[0].1[0].name, "Reading");
    }

    #[test]
    fn flow_and_counts() {
        assert_eq!(count(12480), "12,480");
        assert_eq!(count(7), "7");
        let rows = vec![row("screen", "ok", ""), row("systemAudio", "warn", "")];
        let lanes = vec![
            json!({"processor": "ocr", "queued": 25, "retrying": 0, "running": 1, "failedLast24h": 1}),
        ];
        let n = flow_nodes(
            &rows,
            &lanes,
            Some(&json!({"backlogCount": 0, "vectorCount": 12480})),
        );
        assert_eq!(n[0].big, "2");
        assert_eq!(n[0].lines[0].text, "sys audio");
        assert_eq!(n[1].big, "25");
        assert_eq!(n[1].tone, Some(Tone::Danger));
        assert_eq!(n[1].bar, Some(0.5));
        assert_eq!(n[4].lines[0].text, "12,480 done");
        assert_eq!(n[0].unit, "sources live");
        // Real app: one source live (screen only, audio off) reads singular.
        let one = flow_nodes(
            &[row("screen", "ok", ""), row("microphone", "off", "")],
            &[],
            None,
        );
        assert_eq!((one[0].big.as_str(), one[0].unit), ("1", "source live"));
    }
}
