//! Quick Recall's view models: the commands' camelCase JSON read through serde,
//! so app-core's DTOs (serialised) and the parity fixture feed the same types.

use std::path::PathBuf;

use app_infra::MatchBox;
use serde::Deserialize;
use time::OffsetDateTime;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: i64,
    #[serde(default)]
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameHit {
    pub rank: f64,
    pub group_key: String,
    pub representative_frame: Frame,
    pub group_start_at: String,
    pub group_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    pub thumbnail_frame_id: i64,
    #[serde(default)]
    pub found_by_meaning: bool,
    #[serde(default)]
    pub match_boxes: Vec<MatchBox>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSegment {
    pub id: i64,
    pub source_kind: String,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioHit {
    pub rank: f64,
    pub group_key: String,
    pub audio_segment: AudioSegment,
    pub source_kind: String,
    pub span_start_ms: u64,
    pub span_end_ms: u64,
    pub absolute_start_at: String,
    pub absolute_end_at: String,
    pub match_count: u32,
    pub snippet: String,
    pub aligned_frame: Option<Frame>,
    #[serde(default)]
    pub found_by_meaning: bool,
}

impl AudioHit {
    pub fn mic(&self) -> bool {
        self.source_kind == "microphone"
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    #[serde(default)]
    pub frames: Vec<FrameHit>,
    #[serde(default)]
    pub audio: Vec<AudioHit>,
    #[serde(default)]
    pub has_more_frames: bool,
    #[serde(default)]
    pub has_more_audio: bool,
    #[serde(default)]
    pub residual_query: String,
    #[serde(default)]
    pub parse_errors: Vec<app_infra::SearchParseError>,
    #[serde(default)]
    pub index_empty: bool,
    #[serde(default)]
    pub semantic_degraded: bool,
    #[serde(default)]
    pub untranscribed_segments_in_range: u32,
}

/// One row of the ranked list: either kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Ranked {
    Frame(FrameHit),
    Audio(AudioHit),
}

impl Ranked {
    pub fn key(&self) -> &str {
        match self {
            Ranked::Frame(f) => &f.group_key,
            Ranked::Audio(a) => &a.group_key,
        }
    }

    pub fn start(&self) -> &str {
        match self {
            Ranked::Frame(f) => &f.group_start_at,
            Ranked::Audio(a) => &a.absolute_start_at,
        }
    }

    pub fn meaning(&self) -> bool {
        match self {
            Ranked::Frame(f) => f.found_by_meaning,
            Ranked::Audio(a) => a.found_by_meaning,
        }
    }

    pub fn snippet(&self) -> &str {
        match self {
            Ranked::Frame(f) => &f.snippet,
            Ranked::Audio(a) => &a.snippet,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Ranked::Frame(f) => f
                .window_title
                .clone()
                .or_else(|| f.app_name.clone())
                .unwrap_or_else(|| "Unknown app".into()),
            Ranked::Audio(a) if a.mic() => "Microphone".into(),
            Ranked::Audio(_) => "System audio".into(),
        }
    }

    /// The captured page behind a frame result (`⌘↵`).
    pub fn page_frame(&self) -> Option<i64> {
        match self {
            Ranked::Frame(f) if f.url.is_some() => Some(f.thumbnail_frame_id),
            _ => None,
        }
    }
}

/// Two-pointer merge by rank (the broker's rule): lower first; on a tie the
/// newer start wins, and a frame wins a full tie.
pub fn merge_by_rank(frames: &[FrameHit], audio: &[AudioHit]) -> Vec<Ranked> {
    let (mut f, mut a) = (0, 0);
    let mut out = Vec::with_capacity(frames.len() + audio.len());
    while f < frames.len() || a < audio.len() {
        let take_frame = match (frames.get(f), audio.get(a)) {
            (Some(_), None) => true,
            (None, _) => false,
            (Some(fr), Some(cl)) => {
                if fr.rank != cl.rank {
                    fr.rank < cl.rank
                } else {
                    ms(&fr.group_start_at) >= ms(&cl.absolute_start_at)
                }
            }
        };
        if take_frame {
            out.push(Ranked::Frame(frames[f].clone()));
            f += 1;
        } else {
            out.push(Ranked::Audio(audio[a].clone()));
            a += 1;
        }
    }
    out
}

/// `get_quick_recall_index_status`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub queued_frames: i64,
    pub oldest_queued_ms: Option<i64>,
    #[serde(default)]
    pub semantic_building: bool,
    pub semantic_indexed_percent: Option<u8>,
}

/// One spoken turn with text (`list_speaker_turns`, empty turns dropped).
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
    pub text: String,
}

/// One `get_frame_scrub_previews` entry: the file to paint, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub frame_id: i64,
    pub path: Option<PathBuf>,
    pub missing: Option<String>,
}

/// One cited Answer Source (`AskAiSource`), narrowed from the opaque JSON.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cite {
    pub kind: String,
    pub frame_id: Option<i64>,
    pub audio_segment_id: Option<i64>,
    pub span_start_ms: Option<u64>,
    pub aligned_frame_id: Option<i64>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: String,
    pub source_kind: Option<String>,
}

impl Cite {
    pub fn mic(&self) -> bool {
        self.source_kind.as_deref() == Some("microphone")
    }

    /// Does this citation point at `r`?
    pub fn cites(&self, r: &Ranked) -> bool {
        match r {
            Ranked::Frame(f) => self.frame_id == Some(f.representative_frame.id),
            Ranked::Audio(a) => self.audio_segment_id == Some(a.audio_segment.id),
        }
    }
}

pub fn cites(value: &serde_json::Value) -> Vec<Cite> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value::<Cite>(v.clone()).ok())
        .filter(|c| c.kind == "frame" || c.kind == "audio")
        .collect()
}

/// A backend timestamp ("2026-11-04T09:01:56.000Z" or "2026-11-04 09:01:56", UTC)
/// as unix ms; 0 when it won't parse.
pub fn ms(iso: &str) -> i64 {
    use time::format_description::well_known::Rfc3339;
    let parsed = OffsetDateTime::parse(iso, &Rfc3339).or_else(|_| {
        let t = format!("{}Z", iso.replacen(' ', "T", 1));
        OffsetDateTime::parse(&t, &Rfc3339)
    });
    parsed.map_or(0, |d| (d.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// A snippet with its `<mark>` runs: `(text, marked)`.
pub fn snippet_runs(s: &str) -> Vec<(String, bool)> {
    // Snippets are HTML to the Svelte app, which folds newlines; GPUI text breaks the
    // line on each one and spills out of a one-line row.
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = Vec::new();
    let mut rest = s.as_str();
    while let Some(open) = rest.find("<mark>") {
        if open > 0 {
            out.push((rest[..open].to_string(), false));
        }
        let after = &rest[open + 6..];
        let close = after.find("</mark>").unwrap_or(after.len());
        out.push((after[..close].to_string(), true));
        rest = after.get(close + 7..).unwrap_or("");
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), false));
    }
    out
}

pub fn plain_snippet(s: &str) -> String {
    snippet_runs(s).into_iter().map(|r| r.0).collect()
}
