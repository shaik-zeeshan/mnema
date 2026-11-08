//! The drawer's view models: the commands' camelCase JSON (`AudioSegmentDto`,
//! `SpeakerTurnDto`, `SpeakerClusterDto`, `PersonProfileDto`, `ProcessingJobDto`,
//! `ProcessingResultDto`) read straight off the wire shape, so the parity fixture
//! and app-core feed the same structs. Unknown fields are ignored.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

pub const SUBJECT_TYPE: &str = "audio_segment";
pub const TRANSCRIPTION: &str = "audio_transcription";
pub const SPEAKER_ANALYSIS: &str = "speaker_analysis";
pub const SPEECH_ACTIVITY: &str = "system_audio_speech_activity";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Mic,
    Sys,
}

impl Src {
    /// `audioSourceLabel`.
    pub fn label(self) -> &'static str {
        match self {
            Src::Mic => "microphone",
            Src::Sys => "system audio",
        }
    }
}

/// `AudioSegmentRecord`.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub id: i64,
    pub source: Src,
    pub session_id: String,
    pub segment_index: i64,
    pub file_path: PathBuf,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Segment {
    /// From the Timeline's lane row; the file path arrives with `get_audio_segment`.
    pub fn seed(s: &crate::timeline::model::AudioSegment) -> Self {
        use crate::timeline::model::AudioSource;
        Self {
            id: s.id,
            source: if s.source == AudioSource::Microphone {
                Src::Mic
            } else {
                Src::Sys
            },
            session_id: String::new(),
            segment_index: s.segment_index,
            file_path: PathBuf::new(),
            start_ms: s.start_ms,
            end_ms: s.end_ms,
        }
    }
    pub fn duration_s(&self) -> f64 {
        (self.end_ms - self.start_ms).max(0) as f64 / 1000.
    }
    pub fn file_name(&self) -> String {
        self.file_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SegmentRow {
    id: i64,
    source_kind: String,
    source_session_id: String,
    segment_index: i64,
    file_path: String,
    started_at: String,
    ended_at: String,
}

fn ms(s: &str) -> i64 {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::parse(s, &Rfc3339)
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or_default()
}

/// An `AudioSegmentDto` JSON value.
pub fn segment(v: &Value) -> Option<Segment> {
    let r: SegmentRow = serde_json::from_value(v.clone()).ok()?;
    Some(Segment {
        id: r.id,
        source: if r.source_kind == "microphone" {
            Src::Mic
        } else {
            Src::Sys
        },
        session_id: r.source_session_id,
        segment_index: r.segment_index,
        file_path: PathBuf::from(r.file_path),
        start_ms: ms(&r.started_at),
        end_ms: ms(&r.ended_at),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Turn {
    pub id: i64,
    pub session_id: String,
    pub cluster_id: i64,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub suggested_person_id: Option<i64>,
    pub recognition_confidence: Option<String>,
    pub recognition_score: Option<f32>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub transcript_text: Option<String>,
    pub overlaps: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cluster {
    pub id: i64,
    pub speaker_label: String,
    pub person_id: Option<i64>,
    pub person_link_auto: bool,
    pub suggested_person_id: Option<i64>,
    pub suggested_merge_target_cluster_id: Option<i64>,
    pub suggested_merge_score: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub id: i64,
    pub display_name: String,
    pub embedding_count: i64,
    pub is_account_owner: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Job {
    pub id: i64,
    pub processor: String,
    pub status: String,
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub queued_at: String,
}

impl Job {
    pub fn pending(&self) -> bool {
        self.status == "queued" || self.status == "running"
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcResult {
    pub result_text: Option<String>,
    pub structured_payload_json: Option<String>,
}

/// A transcription run or word (`TranscriptionSegment` / `TranscriptionWord`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

fn payload(json: Option<&str>) -> Option<Value> {
    json.and_then(|j| serde_json::from_str(j).ok())
}

/// `normalizeTranscriptionTimedRuns`: drops malformed / empty runs, clamps, sorts.
pub fn timed_runs(v: Option<&Value>) -> Vec<Run> {
    let mut out: Vec<Run> = v
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let (s, e) = (r.get("startMs")?.as_f64()?, r.get("endMs")?.as_f64()?);
            let text = r.get("text")?.as_str()?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let start_ms = s.round().max(0.) as i64;
            Some(Run {
                start_ms,
                end_ms: (e.round() as i64).max(start_ms),
                text,
            })
        })
        .collect();
    out.sort_by_key(|r| (r.start_ms, r.end_ms));
    out
}

/// A transcription result's runs (segments, else words) and words.
pub fn transcript(result: &ProcResult) -> (Vec<Run>, Vec<Run>) {
    let p = payload(result.structured_payload_json.as_deref());
    let words = timed_runs(p.as_ref().and_then(|p| p.get("words")));
    let mut runs = timed_runs(p.as_ref().and_then(|p| p.get("segments")));
    if runs.is_empty() {
        runs = words.clone();
    }
    (runs, words)
}

/// `parseSystemAudioSpeechActivityPayload(..).speechDetected`.
pub fn speech_detected(result: Option<&ProcResult>) -> Option<bool> {
    payload(result?.structured_payload_json.as_deref())?
        .get("speechDetected")?
        .as_bool()
}

/// Speaker-analysis provenance (`metadata.provenance`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Provenance {
    pub skip_reason: Option<String>,
    pub audio_peak: Option<f64>,
    pub chunking_mode: Option<String>,
    pub chunk_count: Option<i64>,
}

pub fn provenance(result: Option<&ProcResult>) -> Option<Provenance> {
    let p = payload(result?.structured_payload_json.as_deref())?;
    let v = p.get("metadata")?.get("provenance")?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Provenance {
        skip_reason: s("skipReason"),
        audio_peak: v.get("audioPeak").and_then(Value::as_f64),
        chunking_mode: s("chunkingMode"),
        chunk_count: v.get("chunkCount").and_then(Value::as_i64),
    })
}

/// `TranscriptModelLabel`: short label in the strip, full name in its tip.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelLabel {
    pub label: String,
    pub tip: String,
}

/// `resolveAudioTranscriptionModelLabel` + `transcriptModelLabel`.
pub fn model_label(job_payload: Option<&str>, result_payload: Option<&str>) -> Option<ModelLabel> {
    let (job, res) = (payload(job_payload), payload(result_payload));
    let pick = |k: &str| {
        res.as_ref()
            .and_then(|r| r.get(k)?.as_str().map(str::to_string))
            .or_else(|| job.as_ref()?.get(k)?.as_str().map(str::to_string))
    };
    let provider = pick("provider")?;
    let model = pick("modelId");
    let (name, short) = match provider.as_str() {
        "local_whisper" => ("Local Whisper", "whisper"),
        "apple_speech_on_device" => ("Apple Speech (on-device)", "apple speech"),
        "parakeet" => ("Parakeet", "parakeet"),
        "deepgram" => ("Deepgram", "deepgram"),
        p => (p, p),
    };
    let tail = match (&model, provider.as_str()) {
        (Some(m), "parakeet") => {
            let version = m.split(|c: char| !c.is_ascii_alphanumeric()).find(|t| {
                t.len() > 1 && t.starts_with('v') && t[1..].bytes().all(|b| b.is_ascii_digit())
            });
            [version, m.contains("int8").then_some("int8")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        }
        (m, _) => m.clone().unwrap_or_default(),
    };
    Some(ModelLabel {
        label: [short, tail.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        tip: match model {
            Some(m) => format!("{name} · {m}"),
            None => name.to_string(),
        },
    })
}
