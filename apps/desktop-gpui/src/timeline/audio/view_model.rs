//! `audio-drawer-view.ts`: the pure view model of the drawer's reading surface
//! (speaker groups and labels, colour × shape marks, the speaker strip, karaoke,
//! seek clamps, waveform bars, the repair panel's labels, the status pill and
//! which panel replaces the reader). No GPUI here; `view_model_tests.rs` covers it.

use std::collections::{HashMap, HashSet};

use gpui_kit::Hsla;

use super::model::{Cluster, Profile, Provenance, Run, Src, Turn};
use crate::theme::Palette;

/// Consecutive turns of one cluster, merged into one readable paragraph.
/// Cluster −1 is a transcription run with no diarized owner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub cluster_id: i64,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub merge_target: Option<i64>,
    pub merge_score: Option<f32>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub overlaps: bool,
    pub turn_ids: Vec<i64>,
}

/// `buildSpeakerGroups`: wordless turns are dropped (no words, no speaker).
pub fn speaker_groups(turns: &[Turn], clusters: &[Cluster]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for t in turns {
        let text = t.transcript_text.as_deref().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        if let Some(prev) = groups.last_mut().filter(|g| g.cluster_id == t.cluster_id) {
            prev.end_ms = prev.end_ms.max(t.end_ms);
            prev.text = format!("{} {text}", prev.text).trim().to_string();
            prev.overlaps |= t.overlaps;
            prev.turn_ids.push(t.id);
            continue;
        }
        let c = clusters.iter().find(|c| c.id == t.cluster_id);
        groups.push(Group {
            cluster_id: t.cluster_id,
            speaker_label: t.speaker_label.clone(),
            person_id: t.person_id,
            suggested_person_id: t.suggested_person_id,
            recognition_confidence: t.recognition_confidence.clone(),
            recognition_score: t.recognition_score,
            merge_target: c.and_then(|c| c.suggested_merge_target_cluster_id),
            merge_score: c.and_then(|c| c.suggested_merge_score),
            start_ms: t.start_ms,
            end_ms: t.end_ms,
            text: text.to_string(),
            overlaps: t.overlaps,
            turn_ids: vec![t.id],
        });
    }
    groups
}

/// `transcriptFallbackGroups`: the transcript's own runs as paragraphs.
pub fn fallback_groups(runs: &[Run], text: Option<&str>, duration_s: f64) -> Vec<Group> {
    let whole;
    let runs = if !runs.is_empty() {
        runs
    } else if let Some(t) = text.map(str::trim).filter(|t| !t.is_empty()) {
        whole = [Run {
            start_ms: 0,
            end_ms: (duration_s * 1000.).round() as i64,
            text: t.to_string(),
        }];
        &whole[..]
    } else {
        &[]
    };
    runs.iter()
        .enumerate()
        .map(|(i, r)| Group {
            cluster_id: -1,
            speaker_label: "Voice".into(),
            start_ms: r.start_ms,
            end_ms: r.end_ms,
            text: r.text.clone(),
            turn_ids: vec![-(i as i64 + 1)],
            ..Default::default()
        })
        .collect()
}

pub fn clean_label(label: &str) -> String {
    let t = label.trim();
    let lower = t.to_ascii_lowercase();
    match lower.strip_prefix("maybe") {
        Some(rest) if rest.starts_with(char::is_whitespace) => t[5..].trim().to_string(),
        _ => t.to_string(),
    }
}

/// `Unknown speaker N`.
pub fn is_default_label(label: &str) -> bool {
    let l = clean_label(label).to_ascii_lowercase();
    let Some(rest) = l.strip_prefix("unknown speaker") else {
        return false;
    };
    let n = rest.trim_start();
    rest.starts_with(char::is_whitespace) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

pub fn profile_name(profiles: &[Profile], id: Option<i64>) -> Option<&str> {
    let id = id?;
    profiles
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.display_name.as_str())
}

/// `speakerPersistedName`: the linked person, else the cluster's label.
pub fn persisted_name(g: &Group, profiles: &[Profile]) -> String {
    if g.cluster_id < 0 {
        return g.speaker_label.clone();
    }
    match g.person_id {
        Some(_) => clean_label(profile_name(profiles, g.person_id).unwrap_or(&g.speaker_label)),
        None => clean_label(&g.speaker_label),
    }
}

/// `speakerIsUnnamed` (a fallback paragraph always is).
pub fn is_unnamed(g: &Group) -> bool {
    g.cluster_id < 0 || (g.person_id.is_none() && is_default_label(&g.speaker_label))
}

/// `speakerClusterOptionLabel`: auto links say so.
pub fn cluster_option_label(c: &Cluster, profiles: &[Profile]) -> String {
    if c.person_id.is_some() {
        let name = profile_name(profiles, c.person_id).unwrap_or(&c.speaker_label);
        return if c.person_link_auto {
            format!("{name} (auto)")
        } else {
            name.to_string()
        };
    }
    if c.suggested_person_id.is_some() {
        return profile_name(profiles, c.suggested_person_id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("Maybe {}", clean_label(&c.speaker_label)));
    }
    clean_label(&c.speaker_label)
}

pub fn suggested_name(g: &Group, profiles: &[Profile]) -> String {
    profile_name(profiles, g.suggested_person_id)
        .map(str::to_string)
        .unwrap_or_else(|| clean_label(&g.speaker_label))
}

pub fn merge_target_label(g: &Group, clusters: &[Cluster], profiles: &[Profile]) -> Option<String> {
    let id = g.merge_target?;
    clusters
        .iter()
        .find(|c| c.id == id)
        .map(|c| cluster_option_label(c, profiles))
}

/// Cosine to two decimals (`formatScore`).
pub fn format_score(score: Option<f32>) -> Option<String> {
    score.filter(|s| s.is_finite()).map(|s| format!("{s:.2}"))
}

/// The gutter's inline suggestion chip: (name, meta) on the cluster's first paragraph.
pub fn suggestion_chip(
    groups: &[Group],
    ix: usize,
    profiles: &[Profile],
    score: bool,
) -> Option<(String, Option<String>)> {
    let g = groups.get(ix)?;
    if g.suggested_person_id.is_none() || g.person_id.is_some() {
        return None;
    }
    if groups.iter().position(|c| c.cluster_id == g.cluster_id) != Some(ix) {
        return None;
    }
    let s = score.then(|| format_score(g.recognition_score)).flatten();
    let meta = match (&g.recognition_confidence, s) {
        (Some(c), Some(s)) => Some(format!("{c} · {s}")),
        (c, s) => c.clone().or(s),
    };
    Some((suggested_name(g, profiles), meta))
}

/// `samplePreviewHeadsMs`: this cluster's first turn, then the merge candidate's.
pub fn sample_heads(g: &Group, turns: &[Turn]) -> Vec<i64> {
    let head = |id: i64| {
        turns
            .iter()
            .find(|t| t.cluster_id == id)
            .map(|t| t.start_ms)
    };
    [Some(head(g.cluster_id)), g.merge_target.map(head)]
        .into_iter()
        .flatten()
        .flatten()
        .collect()
}

// ── Dual encoding: colour AND shape ─────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Square,
    Triangle,
    Diamond,
}

/// `SPEAKER_COLOR_PALETTE` (owner pinned to communication).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Owner,
    Meetings,
    Research,
    Entertainment,
    Learning,
}

impl Hue {
    pub fn color(self, p: &Palette) -> Hsla {
        match self {
            Hue::Owner => p.cat_communication,
            Hue::Meetings => p.cat_meetings,
            Hue::Research => p.cat_research,
            Hue::Entertainment => p.cat_entertainment,
            Hue::Learning => p.cat_learning,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub hue: Hue,
    pub shape: Shape,
}

pub type Marks = HashMap<i64, Mark>;

/// `assignSpeakerMarks`: colour and shape in first-appearance order.
pub fn assign_marks(ordered: impl IntoIterator<Item = i64>, owners: &HashSet<i64>) -> Marks {
    const HUES: [Hue; 4] = [
        Hue::Meetings,
        Hue::Research,
        Hue::Entertainment,
        Hue::Learning,
    ];
    const SHAPES: [Shape; 4] = [
        Shape::Circle,
        Shape::Square,
        Shape::Triangle,
        Shape::Diamond,
    ];
    let mut out = Marks::new();
    let (mut hue, mut shape) = (0, 0);
    for id in ordered {
        if out.contains_key(&id) {
            continue;
        }
        let h = if owners.contains(&id) {
            Hue::Owner
        } else {
            hue += 1;
            HUES[(hue - 1) % HUES.len()]
        };
        out.insert(
            id,
            Mark {
                hue: h,
                shape: SHAPES[shape % SHAPES.len()],
            },
        );
        shape += 1;
    }
    out
}

/// One entry of the speaker strip.
#[derive(Clone, Debug, PartialEq)]
pub struct StripSpeaker {
    pub cluster_id: i64,
    pub name: String,
    pub owner: bool,
    /// `you · auto` / `you` / `maybe Daniel` / `unnamed` / ``.
    pub state: String,
    pub unnamed: bool,
    pub talk_ms: i64,
    pub share_pct: i64,
}

/// `speakerStrip`: talk time summed over the text-bearing turns.
pub fn strip(
    groups: &[Group],
    turns: &[Turn],
    clusters: &[Cluster],
    profiles: &[Profile],
) -> Vec<StripSpeaker> {
    let mut talk: HashMap<i64, i64> = HashMap::new();
    let mut total = 0;
    for t in turns
        .iter()
        .filter(|t| !t.transcript_text.as_deref().unwrap_or("").trim().is_empty())
    {
        let ms = (t.end_ms - t.start_ms).max(0);
        *talk.entry(t.cluster_id).or_default() += ms;
        total += ms;
    }
    let owners: HashSet<i64> = profiles
        .iter()
        .filter(|p| p.is_account_owner)
        .map(|p| p.id)
        .collect();
    let mut out: Vec<StripSpeaker> = Vec::new();
    for g in groups {
        if g.cluster_id < 0 || out.iter().any(|s| s.cluster_id == g.cluster_id) {
            continue;
        }
        let unnamed = is_unnamed(g);
        let owner = g.person_id.is_some_and(|id| owners.contains(&id));
        let auto = clusters
            .iter()
            .any(|c| c.id == g.cluster_id && c.person_link_auto);
        let state = if owner {
            if auto { "you · auto" } else { "you" }.to_string()
        } else if g.person_id.is_none() && g.suggested_person_id.is_some() {
            let name = suggested_name(g, profiles);
            format!("maybe {}", name.split(' ').next().unwrap_or(&name))
        } else if unnamed {
            "unnamed".into()
        } else {
            String::new()
        };
        let talk_ms = talk.get(&g.cluster_id).copied().unwrap_or(0);
        out.push(StripSpeaker {
            cluster_id: g.cluster_id,
            name: persisted_name(g, profiles),
            owner,
            state,
            unnamed,
            talk_ms,
            share_pct: if total > 0 {
                (talk_ms as f64 / total as f64 * 100.).round() as i64
            } else {
                0
            },
        });
    }
    out
}

// ── Karaoke ─────────────────────────────────────────────────────────────────

/// Slack around a word's bounds, so the highlight doesn't blink off between words.
pub const KARAOKE_TOLERANCE_MS: i64 = 150;
/// Word timings covering less of a paragraph than this degrade to the paragraph.
pub const KARAOKE_MIN_COVERAGE: f64 = 0.8;

/// `karaokeWordsForRange`: words whose midpoint is in `(start, end]`.
pub fn words_in_range(words: &[Run], start: i64, end: i64) -> Vec<Run> {
    let mut out: Vec<Run> = words
        .iter()
        .filter_map(|w| {
            let text = w.text.trim();
            let mid = (w.start_ms + w.end_ms) as f64 / 2.;
            (!text.is_empty() && mid > start as f64 && mid <= end as f64).then(|| Run {
                start_ms: w.start_ms,
                end_ms: w.end_ms.max(w.start_ms),
                text: text.to_string(),
            })
        })
        .collect();
    out.sort_by_key(|w| w.start_ms);
    out
}

/// `activeKaraokeIndex`: the word holding the floor, bounded at both ends.
pub fn active_word(words: &[Run], now: i64) -> Option<usize> {
    for (i, w) in words.iter().enumerate() {
        if now < w.start_ms - KARAOKE_TOLERANCE_MS {
            break;
        }
        if let Some(next) = words.get(i + 1) {
            if now >= w.end_ms.max(next.start_ms - KARAOKE_TOLERANCE_MS) {
                continue;
            }
        }
        return (now <= w.end_ms + KARAOKE_TOLERANCE_MS).then_some(i);
    }
    None
}

/// `karaokeForGroup`: the paragraph's words, or `None` to degrade (no words, too
/// few to cover the text, or words leaking in from an overlapping turn).
pub fn karaoke(words: &[Run], g: &Group) -> Option<Vec<Run>> {
    if words.is_empty() {
        return None;
    }
    let picked = words_in_range(words, g.start_ms, g.end_ms);
    if picked.is_empty() {
        return None;
    }
    let joined: Vec<&str> = picked.iter().map(|w| w.text.as_str()).collect();
    let joined = joined.join(" ");
    let chars = |s: &str| s.chars().count() as f64;
    if chars(&joined) < chars(&g.text) * KARAOKE_MIN_COVERAGE {
        return None;
    }
    let spoken = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count() as f64;
    if spoken(&joined) > spoken(&g.text) / KARAOKE_MIN_COVERAGE {
        return None;
    }
    Some(picked)
}

/// `activeGroupIndex`: start-bounded, never blanks once playback has started.
pub fn active_group(groups: &[Group], now: i64, started: bool) -> Option<usize> {
    if !started {
        return None;
    }
    groups.iter().rposition(|g| now >= g.start_ms)
}

pub fn clamp_seek(ms: i64, start: i64, end: i64) -> i64 {
    ms.clamp(start.min(end), start.max(end))
}

/// `segmentSeekMs`: the overlapping run start nearest the paragraph's own start.
pub fn segment_seek(runs: &[Run], g: &Group) -> i64 {
    let best = runs
        .iter()
        .filter(|r| r.end_ms >= g.start_ms && r.start_ms <= g.end_ms)
        .map(|r| r.start_ms)
        .min_by_key(|s| (s - g.start_ms).abs());
    clamp_seek(best.unwrap_or(g.start_ms), g.start_ms, g.end_ms)
}

// ── Waveform ────────────────────────────────────────────────────────────────

/// `WAVEFORM_BUCKETS`.
pub const WAVEFORM_BUCKETS: usize = 150;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// 4–100.
    pub height_pct: f32,
    pub cluster_id: Option<i64>,
    pub at_ms: i64,
}

/// `waveformBars`: each bar's hue is whoever held the floor at that moment.
pub fn wave_bars(peaks: &[f32], duration_ms: f64, turns: &[Turn]) -> Vec<Bar> {
    if peaks.is_empty() || !(duration_ms > 0.) {
        return Vec::new();
    }
    let step = duration_ms / peaks.len() as f64;
    peaks
        .iter()
        .enumerate()
        .map(|(i, peak)| {
            let at_ms = (i as f64 * step + step / 2.).round() as i64;
            let amp = if peak.is_finite() {
                peak.clamp(0., 1.)
            } else {
                0.
            };
            Bar {
                height_pct: (amp * 100.).round().max(4.),
                cluster_id: turns
                    .iter()
                    .find(|t| at_ms >= t.start_ms && at_ms <= t.end_ms)
                    .map(|t| t.cluster_id),
                at_ms,
            }
        })
        .collect()
}

// ── Repair slide-over ───────────────────────────────────────────────────────

pub fn compact_duration(ms: i64) -> String {
    let total = (ms.max(0) as f64 / 1000.).round() as i64;
    let (m, s) = (total / 60, total % 60);
    if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// `cluster 7 · 11 turns · 1m 48s`.
pub fn cluster_summary(cluster_id: i64, turns: &[Turn]) -> String {
    let mine: Vec<&Turn> = turns
        .iter()
        .filter(|t| t.cluster_id == cluster_id)
        .collect();
    let total: i64 = mine.iter().map(|t| (t.end_ms - t.start_ms).max(0)).sum();
    let n = if mine.len() == 1 {
        "1 turn".into()
    } else {
        format!("{} turns", mine.len())
    };
    format!("cluster {cluster_id} · {n} · {}", compact_duration(total))
}

/// `validateSpeakerName`: the warning, or `None` when the name can be saved.
pub fn name_warning(value: &str) -> Option<&'static str> {
    let t = value.trim();
    if t.is_empty() {
        return Some("A voice needs a name before it can become a saved person.");
    }
    let lower = t.to_ascii_lowercase();
    let placeholder = ["unknown speaker", "speaker", "voice"].iter().any(|p| {
        lower
            .strip_prefix(p)
            .is_some_and(|rest| rest.trim_start().bytes().all(|b| b.is_ascii_digit()))
    });
    placeholder.then_some(
        "Still the placeholder label. Give it a real name, or link to a saved person below.",
    )
}

/// `· 34 samples`.
pub fn samples_label(count: i64) -> String {
    match count {
        c if c <= 0 => String::new(),
        1 => "1 sample".into(),
        c => format!("{c} samples"),
    }
}

// ── Status pill + which panel replaces the reader ───────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TranscriptStatus {
    #[default]
    Idle,
    Loading,
    Success,
    Empty,
    Missing,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Work,
    Warn,
    Bad,
    Idle,
}

/// `drawerStatusPill`: (tone, label, busy).
pub fn status_pill(
    src: Src,
    t: TranscriptStatus,
    running: bool,
    failed: bool,
    speakers: usize,
) -> (Tone, String, bool) {
    let sys = src == Src::Sys;
    let pick = |tone, label: &str, busy| (tone, label.to_string(), busy);
    use TranscriptStatus as S;
    if running {
        return pick(Tone::Work, "finding speakers", true);
    }
    match t {
        S::Loading => pick(Tone::Work, "loading", true),
        S::Running => pick(
            Tone::Work,
            if sys {
                "detecting speech"
            } else {
                "transcribing"
            },
            true,
        ),
        S::Error => pick(
            Tone::Bad,
            if sys {
                "speech detection failed"
            } else {
                "failed"
            },
            false,
        ),
        _ if failed => pick(Tone::Bad, "speaker analysis failed", false),
        S::Empty => pick(Tone::Idle, "no speech", false),
        S::Missing => pick(Tone::Warn, "not transcribed", false),
        S::Success => {
            let n = match speakers {
                0 => String::new(),
                1 => " · 1 speaker".into(),
                n => format!(" · {n} speakers"),
            };
            (Tone::Ok, format!("transcribed{n}"), false)
        }
        S::Idle => pick(Tone::Idle, "unavailable", false),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Reader,
    Skeleton,
    Processing,
    NoSpeech,
    Failed,
    SpeakersFailed,
    NotRun,
}

/// `drawerPanelKind`.
pub fn panel(
    t: TranscriptStatus,
    speakers_failed: bool,
    ignore_failure: bool,
    groups: usize,
) -> Panel {
    use TranscriptStatus as S;
    match t {
        S::Loading => Panel::Skeleton,
        S::Running => Panel::Processing,
        S::Empty => Panel::NoSpeech,
        S::Error => Panel::Failed,
        S::Success if speakers_failed && !ignore_failure => Panel::SpeakersFailed,
        S::Success if groups > 0 => Panel::Reader,
        S::Success => Panel::NoSpeech,
        _ => Panel::NotRun,
    }
}

/// `stage 2 of 3`.
pub fn stage_label(src: Src, processor: &str) -> String {
    use super::model::{SPEAKER_ANALYSIS, SPEECH_ACTIVITY, TRANSCRIPTION};
    let stages: &[&str] = match src {
        Src::Sys => &[SPEECH_ACTIVITY, TRANSCRIPTION, SPEAKER_ANALYSIS],
        Src::Mic => &[TRANSCRIPTION, SPEAKER_ANALYSIS],
    };
    stages
        .iter()
        .position(|s| *s == processor)
        .map(|i| format!("stage {} of {}", i + 1, stages.len()))
        .unwrap_or_default()
}

/// `skipReason silent · audioPeak 0.004`.
pub fn provenance_footnote(p: Option<&Provenance>) -> String {
    let Some(p) = p else { return String::new() };
    let mut parts = Vec::new();
    if let Some(r) = &p.skip_reason {
        parts.push(format!("skipReason {r}"));
    }
    if let Some(a) = p.audio_peak {
        parts.push(format!("audioPeak {a:.3}"));
    }
    if let Some(m) = &p.chunking_mode {
        parts.push(format!("chunkingMode {m}"));
    }
    if let Some(n) = p.chunk_count.filter(|n| *n > 0) {
        parts.push(format!("{n} chunks"));
    }
    parts.join(" · ")
}

/// `M:SS`.
pub fn player_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0. {
        return "0:00".into();
    }
    let t = seconds.floor() as i64;
    format!("{}:{:02}", t / 60, t % 60)
}

#[cfg(test)]
#[path = "view_model_tests.rs"]
mod tests;
