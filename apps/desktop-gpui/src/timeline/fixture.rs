//! A [`Source`] over the P0 parity map (`apps/desktop/parity/fixtures/timeline/
//! <state>.json`, command → JSON), so goldens, behaviour tests and the scrub
//! bench show what the Svelte stub serves. It also records every call and can be
//! told to fail, for the timing tests.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use app_infra::MatchBox;
use capture_types::Activity;
use serde::Deserialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::data::{ocr_provider_label, parse_ocr_payload, OcrLoad, Res, Source, TimelineSettings};
use super::model::{
    AudioSegment, AudioSource, FocusedWindow, Frame, Landing, OcrBox, Preview, ScrubInterval,
    SourceKind,
};

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    ListFrames { before: Option<i64>, limit: usize },
    Window(i64),
    Preview(i64),
    Scrub { enqueue: bool },
    Cancel,
    Audio,
    Episodes,
    MatchBoxes(i64),
    Ocr(i64),
    RerunOcr(i64),
}

#[derive(Default)]
pub struct FixtureSource {
    frames: Mutex<Vec<Frame>>,
    preview_kind: Option<(String, SourceKind)>,
    /// `list_audio_segments` (tests drop rows to mimic a refresh).
    pub audio: Mutex<Vec<AudioSegment>>,
    episodes: Vec<Activity>,
    boxes: Vec<MatchBox>,
    landing: Mutex<Option<Landing>>,
    pub calls: Mutex<Vec<Call>>,
    /// Make `list_frames` fail (the stale state).
    pub fail_list: Mutex<bool>,
    /// Answer scrub availability with one ready still per frame.
    pub scrub_ready: bool,
    icons: Vec<(String, PathBuf)>,
    /// The OCR read the map's `list_processing_jobs` / `get_processing_result` give.
    /// `None` = never OCR'd.
    pub ocr: Mutex<Option<Res<OcrLoad>>>,
    /// Make `list_audio_segments` fail with this (the map's `{ "__reject": .. }`).
    pub fail_audio: Mutex<Option<String>>,
    settings: TimelineSettings,
    /// `get_earliest_earlier_equivalent_frame` (the dev panel's duplicateOf).
    equivalent: Option<Frame>,
}

fn ms(s: &str) -> i64 {
    OffsetDateTime::parse(s, &Rfc3339)
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or_default()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameRow {
    id: i64,
    session_id: String,
    file_path: String,
    captured_at: String,
    width: Option<f64>,
    height: Option<f64>,
    app_bundle_id: Option<String>,
    app_name: Option<String>,
    window_title: Option<String>,
    url: Option<String>,
    equivalence_hint: Option<String>,
}

impl From<FrameRow> for Frame {
    fn from(r: FrameRow) -> Self {
        Frame {
            id: r.id,
            session_id: r.session_id,
            file_path: Arc::from(Path::new(&r.file_path)),
            captured_at_ms: ms(&r.captured_at),
            width: r.width.map(|w| w as u32),
            height: r.height.map(|h| h as u32),
            app_bundle_id: r.app_bundle_id,
            app_name: r.app_name,
            window_title: r.window_title,
            url: r.url,
            equivalence_hint: r.equivalence_hint,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioRow {
    id: i64,
    source_kind: String,
    segment_index: i64,
    started_at: String,
    ended_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LandingRow {
    kind: String,
    frame_id: Option<i64>,
    query: Option<String>,
    by_meaning: Option<bool>,
}

fn kind(s: &str) -> SourceKind {
    match s {
        "segment_frame_fallback" => SourceKind::SegmentFrameFallback,
        "video_fallback" => SourceKind::VideoFallback,
        "scrub_preview" => SourceKind::ScrubPreview,
        _ => SourceKind::OriginalFrame,
    }
}

fn rows<T: for<'de> Deserialize<'de>>(map: &Value, cmd: &str) -> Vec<T> {
    // A malformed fixture fails loudly: a silent default hid a bad enum once.
    map.get(cmd).map_or_else(Vec::new, |v| {
        serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("fixture {cmd}: {e}"))
    })
}

/// `loadOcrForFrame` over the map: the newest OCR job, its result's boxes and
/// provider; a failed job is its error; no OCR job = never OCR'd.
fn ocr_of(map: &Value) -> Res<OcrLoad> {
    let job = map
        .get("list_processing_jobs")
        .and_then(Value::as_array)
        .and_then(|jobs| {
            jobs.iter()
                .filter(|j| j.get("processor").and_then(Value::as_str) == Some("ocr"))
                .max_by_key(|j| j.get("id").and_then(Value::as_i64))
        });
    let Some(job) = job else {
        return Ok(OcrLoad::default());
    };
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    if text(job, "status").as_deref() == Some("failed") {
        return Err(text(job, "lastError").unwrap_or_else(|| "OCR job failed".into()));
    }
    let result = map
        .get("get_processing_result")
        .ok_or("OCR result not available")?;
    let payload = text(result, "structuredPayloadJson");
    Ok(OcrLoad {
        boxes: Some(
            payload
                .as_deref()
                .and_then(parse_ocr_payload)
                .ok_or("OCR result payload is missing or invalid")?,
        ),
        provider: ocr_provider_label(
            text(job, "payloadJson").as_deref(),
            payload.as_deref(),
            text(result, "processorVersion").as_deref(),
        ),
        from_frame: None,
    })
}

impl FixtureSource {
    pub fn from_map(map: &Value) -> Self {
        let frames: Vec<FrameRow> = rows(map, "list_frames");
        let reject = |cmd: &str| map.get(cmd)?.get("__reject")?.as_str().map(str::to_string);
        let fail_audio = reject("list_audio_segments");
        let audio: Vec<AudioRow> = if fail_audio.is_some() {
            Vec::new()
        } else {
            rows(map, "list_audio_segments")
        };
        let landing: Vec<LandingRow> = rows(map, "drain_pending_broker_open_capture_results");
        let preview_kind = map.get("get_frame_preview").and_then(|p| {
            Some((
                p.get("mimeType")?.as_str()?.to_string(),
                kind(p.get("sourceKind")?.as_str()?),
            ))
        });
        Self {
            frames: Mutex::new(frames.into_iter().map(Frame::from).collect()),
            preview_kind,
            audio: Mutex::new(
                audio
                    .into_iter()
                    .map(|a| AudioSegment {
                        id: a.id,
                        source: if a.source_kind == "microphone" {
                            AudioSource::Microphone
                        } else {
                            AudioSource::SystemAudio
                        },
                        segment_index: a.segment_index,
                        start_ms: ms(&a.started_at),
                        end_ms: ms(&a.ended_at),
                    })
                    .collect(),
            ),
            episodes: rows(map, "list_user_context_activities"),
            boxes: rows(map, "get_frame_match_boxes"),
            landing: Mutex::new(
                landing
                    .into_iter()
                    .find(|l| l.kind == "frame")
                    .and_then(|l| {
                        Some(Landing {
                            query: l.query?,
                            frame_id: l.frame_id?,
                            screen: true,
                            by_meaning: l.by_meaning.unwrap_or(false),
                        })
                    }),
            ),
            ocr: Mutex::new(Some(ocr_of(map))),
            equivalent: map
                .get("get_earliest_earlier_equivalent_frame")
                .filter(|v| !v.is_null())
                .map(|v| {
                    serde_json::from_value::<FrameRow>(v.clone())
                        .unwrap_or_else(|e| panic!("fixture equivalent frame: {e}"))
                        .into()
                }),
            fail_audio: Mutex::new(fail_audio),
            settings: TimelineSettings {
                ocr_enabled: map
                    .pointer("/get_recording_settings/ocr/enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// `apps/desktop/parity/fixtures/timeline/<state>.json`.
    pub fn load(state: &str) -> anyhow::Result<Self> {
        Ok(Self::from_map(&crate::shell::fixture::load(
            "timeline", state,
        )?))
    }

    /// Answer scrub availability with one ready still per frame.
    pub fn with_scrub_ready(mut self) -> Self {
        self.scrub_ready = true;
        self
    }

    /// Answer `resolve_app_icons` with these (bundle id, PNG) pairs.
    pub fn with_icons(mut self, icons: Vec<(String, PathBuf)>) -> Self {
        self.icons = icons;
        self
    }

    /// Replace the frames (tests and the bench tile them out).
    pub fn with_frames(self, frames: Vec<Frame>) -> Self {
        *self.frames.lock().unwrap() = frames;
        self
    }

    pub fn frames(&self) -> Vec<Frame> {
        self.frames.lock().unwrap().clone()
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn log(&self, c: Call) {
        self.calls.lock().unwrap().push(c);
    }
}

impl Source for FixtureSource {
    fn list_frames(&self, before: Option<i64>, limit: usize) -> Res<Vec<Frame>> {
        self.log(Call::ListFrames { before, limit });
        if *self.fail_list.lock().unwrap() {
            return Err("database is locked".into());
        }
        let frames = self.frames.lock().unwrap();
        Ok(frames
            .iter()
            .filter(|f| before.is_none_or(|b| f.id < b))
            .take(limit)
            .cloned()
            .collect())
    }

    fn window_around(&self, frame_id: i64, newer: usize, older: usize) -> Res<FocusedWindow> {
        self.log(Call::Window(frame_id));
        let frames = self.frames.lock().unwrap();
        let at = frames
            .iter()
            .position(|f| f.id == frame_id)
            .ok_or("frame not found")?;
        let lo = at.saturating_sub(newer);
        let hi = (at + older + 1).min(frames.len());
        Ok(FocusedWindow {
            frames: frames[lo..hi].to_vec(),
            target_index: at - lo,
            has_newer: lo > 0,
            has_older: hi < frames.len(),
        })
    }

    fn latest_in_range(&self, start_ms: i64, end_ms: i64) -> Res<Option<Frame>> {
        let frames = self.frames.lock().unwrap();
        Ok(frames
            .iter()
            .find(|f| f.captured_at_ms >= start_ms && f.captured_at_ms < end_ms)
            .cloned())
    }

    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Res<Vec<i64>> {
        let frames = self.frames.lock().unwrap();
        Ok(frames
            .iter()
            .map(|f| f.captured_at_ms)
            .filter(|t| *t >= start_ms && *t < end_ms)
            .collect())
    }

    fn frame_preview(&self, frame_id: i64) -> Res<Option<Preview>> {
        self.log(Call::Preview(frame_id));
        let frames = self.frames.lock().unwrap();
        let Some((mime, kind)) = &self.preview_kind else {
            return Ok(None);
        };
        Ok(frames.iter().find(|f| f.id == frame_id).map(|f| Preview {
            file_path: f.file_path.clone(),
            mime_type: mime.clone(),
            source_kind: *kind,
            redactions: 0,
        }))
    }

    fn scrub_availability(
        &self,
        start_ms: i64,
        end_ms: i64,
        enqueue: bool,
    ) -> Res<Vec<ScrubInterval>> {
        self.log(Call::Scrub { enqueue });
        if !self.scrub_ready {
            return Ok(Vec::new());
        }
        let frames = self.frames.lock().unwrap();
        Ok(frames
            .iter()
            .filter(|f| f.captured_at_ms >= start_ms && f.captured_at_ms <= end_ms)
            .map(|f| ScrubInterval {
                segment_key: f.session_id.clone(),
                start_offset_ms: f.id as u64,
                start_ms: f.captured_at_ms,
                end_ms: f.captured_at_ms + 1,
                preview: Some(Preview {
                    file_path: f.file_path.clone(),
                    mime_type: "image/jpeg".into(),
                    source_kind: SourceKind::ScrubPreview,
                    redactions: 0,
                }),
                queued: false,
            })
            .collect())
    }

    fn cancel_video(&self) {
        self.log(Call::Cancel);
    }

    fn audio_segments(&self, _: i64, _: i64) -> Res<Vec<AudioSegment>> {
        self.log(Call::Audio);
        if let Some(e) = self.fail_audio.lock().unwrap().clone() {
            return Err(e);
        }
        Ok(self.audio.lock().unwrap().clone())
    }

    fn audio_segment(&self, id: i64) -> Res<Option<AudioSegment>> {
        Ok(self
            .audio
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }

    fn episodes(&self, _: i64, _: i64) -> Res<Vec<Activity>> {
        self.log(Call::Episodes);
        Ok(self.episodes.clone())
    }

    fn app_icons(&self, _: &[String]) -> Vec<(String, PathBuf)> {
        self.icons.clone()
    }

    fn match_boxes(&self, frame_id: i64, _: &str, _: bool) -> Res<Vec<MatchBox>> {
        self.log(Call::MatchBoxes(frame_id));
        Ok(self.boxes.clone())
    }

    fn ocr(&self, frame_id: i64) -> Res<Option<Vec<OcrBox>>> {
        self.ocr_load(frame_id).map(|l| l.boxes)
    }

    fn ocr_load(&self, frame_id: i64) -> Res<OcrLoad> {
        self.log(Call::Ocr(frame_id));
        self.ocr
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(OcrLoad::default()))
    }

    fn rerun_ocr(&self, frame_id: i64, _: Option<PathBuf>) -> Res<OcrLoad> {
        self.log(Call::RerunOcr(frame_id));
        self.ocr
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(OcrLoad::default()))
    }

    fn settings(&self) -> TimelineSettings {
        self.settings
    }

    fn earlier_equivalent(&self, _: i64) -> Res<Option<Frame>> {
        Ok(self.equivalent.clone())
    }

    fn take_landing(&self) -> Option<Landing> {
        self.landing.lock().unwrap().take()
    }
}
