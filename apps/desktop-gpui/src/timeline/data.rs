//! Where the Timeline's data comes from. Views never call the backend: the view
//! runs these on the background executor and hands the answers to `paging` /
//! `scrub_engine`. `CoreSource` asks `AppCore`; `fixture::FixtureSource` serves
//! the parity JSON (goldens, tests, the scrub bench).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use app_core::app_infra as ai;
use app_core::{AppCore, AppInfraState};
use app_infra::MatchBox;
use capture_types::{Activity, RetentionPolicy};

use super::model::{
    AudioSegment, AudioSource, FocusedWindow, Frame, Landing, OcrBox, Preview, ScrubInterval,
    SourceKind,
};

pub type Res<T> = Result<T, String>;

/// The timeline commands, synchronous (the view calls them off the main thread).
pub trait Source: Send + Sync + 'static {
    /// `list_frames { limit, beforeId }`, newest first.
    fn list_frames(&self, before_id: Option<i64>, limit: usize) -> Res<Vec<Frame>>;
    /// `get_timeline_window_around_frame`.
    fn window_around(&self, frame_id: i64, newer: usize, older: usize) -> Res<FocusedWindow>;
    /// `get_latest_frame_in_range` (the jumper's "latest at or before").
    fn latest_in_range(&self, start_ms: i64, end_ms: i64) -> Res<Option<Frame>>;
    /// `list_frame_summaries` capture instants in a range (the jumper's density).
    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Res<Vec<i64>>;
    /// `get_frame_preview { videoScope: active_frame }`.
    fn frame_preview(&self, frame_id: i64) -> Res<Option<Preview>>;
    /// `get_scrub_preview_availability`.
    fn scrub_availability(
        &self,
        start_ms: i64,
        end_ms: i64,
        enqueue: bool,
    ) -> Res<Vec<ScrubInterval>>;
    /// `cancel_active_frame_preview_video_requests`.
    fn cancel_video(&self) {}
    /// `list_audio_segments` over the loaded window (padded).
    fn audio_segments(&self, start_ms: i64, end_ms: i64) -> Res<Vec<AudioSegment>>;
    /// `get_audio_segment`: `None` = gone (deleted, retention).
    fn audio_segment(&self, _id: i64) -> Res<Option<AudioSegment>> {
        Ok(None)
    }
    /// `list_user_context_activities` (band colours only).
    fn episodes(&self, start_ms: i64, end_ms: i64) -> Res<Vec<Activity>>;
    /// `resolve_app_icons`: bundle id → icon PNG on disk.
    fn app_icons(&self, bundle_ids: &[String]) -> Vec<(String, PathBuf)>;
    /// `get_frame_match_boxes`.
    fn match_boxes(&self, frame_id: i64, query: &str, by_meaning: bool) -> Res<Vec<MatchBox>>;
    /// The active frame's OCR (`loadOcrForFrame`): `None` = never OCR'd.
    fn ocr(&self, frame_id: i64) -> Res<Option<Vec<OcrBox>>>;
    /// One look at [`Source::ocr`] with the provider label and, when the frame
    /// was never OCR'd, the identical earlier frame whose result it reuses. A
    /// queued / running job is `Pending`: the view re-checks it with [`Source::ocr_job`].
    fn ocr_load(&self, frame_id: i64) -> Res<OcrCheck> {
        Ok(OcrCheck::Done(OcrLoad {
            boxes: self.ocr(frame_id)?,
            ..Default::default()
        }))
    }
    /// One look at a pending OCR job (`get_processing_job`, then its result).
    fn ocr_job(&self, _job_id: i64) -> Res<OcrCheck> {
        Err("OCR job not found".into())
    }
    /// `drain_pending_broker_open_capture_results` (a Quick Recall hit).
    fn take_landing(&self) -> Option<Landing>;
    /// `reprocess_captured_frame_ocr`, then one look at the new job. `source_image`
    /// is the frame's own preview (never another frame's fallback image).
    fn rerun_ocr(&self, _frame_id: i64, _source_image: Option<PathBuf>) -> Res<OcrCheck> {
        Err("OCR can't be rerun here.".into())
    }
    /// `get_earliest_earlier_equivalent_frame`: the frame this one duplicates.
    fn earlier_equivalent(&self, _frame_id: i64) -> Res<Option<Frame>> {
        Ok(None)
    }
    /// `open_captured_url`: the backend opens the raw URL behind the guarded
    /// host. `Ok(false)` = the frame has no http(s) URL.
    fn open_captured_url(&self, _frame_id: i64) -> Res<bool> {
        Ok(false)
    }
    /// The recording settings the Timeline reads.
    fn settings(&self) -> TimelineSettings {
        TimelineSettings::default()
    }
}

/// `followTimelineLive`, the retention window (`retentionPolicy`) and `ocr.enabled`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineSettings {
    pub follow_live: bool,
    /// Frames older than this many days are removed; `None` = kept forever.
    pub keep_days: Option<u32>,
    /// OCR is on: off, saved OCR still shows but nothing reruns.
    pub ocr_enabled: bool,
}

impl Default for TimelineSettings {
    fn default() -> Self {
        Self {
            follow_live: false,
            keep_days: None,
            ocr_enabled: true,
        }
    }
}

/// One OCR read (`OcrLoadResult`): `boxes: None` = never OCR'd.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrLoad {
    pub boxes: Option<Vec<OcrBox>>,
    /// `ocrProviderLabel`: "Apple Vision", "Tesseract · eng", …
    pub provider: Option<String>,
    /// The identical earlier frame whose OCR this frame reuses (`ocrSourceFrame`).
    pub from_frame: Option<i64>,
}

/// One OCR check: done (or never OCR'd), or a job still queued / running.
#[derive(Clone, Debug, PartialEq)]
pub enum OcrCheck {
    Done(OcrLoad),
    Pending {
        job_id: i64,
        /// The identical earlier frame the job belongs to (`ocrSourceFrame`).
        from_frame: Option<i64>,
    },
}

/// `formatOcrProviderLabel`.
fn provider_name(p: &str) -> &str {
    match p {
        "apple_vision" => "Apple Vision",
        "tesseract" => "Tesseract",
        "paddle_ocr" => "PaddleOCR",
        other => other,
    }
}

/// `resolveOcrProviderLabel`: the result's provider (or its provenance), else the
/// job's, else the processor version's prefix; plus the model id when known.
pub fn ocr_provider_label(
    job_payload: Option<&str>,
    result_payload: Option<&str>,
    processor_version: Option<&str>,
) -> Option<String> {
    let parse = |j: Option<&str>| j.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok());
    let (job, result) = (parse(job_payload), parse(result_payload));
    let field = |v: &Option<serde_json::Value>, k: &str| {
        let v = v.as_ref()?;
        v.get(k)
            .and_then(|x| x.as_str())
            .or_else(|| v.get("provenance")?.get(k)?.as_str())
            .map(str::to_string)
    };
    let provider = field(&result, "provider")
        .or_else(|| field(&job, "provider"))
        .or_else(|| processor_version.map(|v| v.split(':').next().unwrap_or(v).to_string()))
        .filter(|p| !p.is_empty())?;
    let name = provider_name(&provider).to_string();
    Some(
        match field(&result, "modelId").or_else(|| field(&job, "modelId")) {
            Some(m) => format!("{name} · {m}"),
            None => name,
        },
    )
}

/// No backend (a shell built without an `AppCore`): an empty timeline.
pub struct NoSource;

impl Source for NoSource {
    fn list_frames(&self, _: Option<i64>, _: usize) -> Res<Vec<Frame>> {
        Ok(Vec::new())
    }
    fn window_around(&self, _: i64, _: usize, _: usize) -> Res<FocusedWindow> {
        Err("no capture index".into())
    }
    fn latest_in_range(&self, _: i64, _: i64) -> Res<Option<Frame>> {
        Ok(None)
    }
    fn frame_times(&self, _: i64, _: i64) -> Res<Vec<i64>> {
        Ok(Vec::new())
    }
    fn frame_preview(&self, _: i64) -> Res<Option<Preview>> {
        Ok(None)
    }
    fn scrub_availability(&self, _: i64, _: i64, _: bool) -> Res<Vec<ScrubInterval>> {
        Ok(Vec::new())
    }
    fn audio_segments(&self, _: i64, _: i64) -> Res<Vec<AudioSegment>> {
        Ok(Vec::new())
    }
    fn episodes(&self, _: i64, _: i64) -> Res<Vec<Activity>> {
        Ok(Vec::new())
    }
    fn app_icons(&self, _: &[String]) -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    fn match_boxes(&self, _: i64, _: &str, _: bool) -> Res<Vec<MatchBox>> {
        Ok(Vec::new())
    }
    fn ocr(&self, _: i64) -> Res<Option<Vec<OcrBox>>> {
        Ok(None)
    }
    fn take_landing(&self) -> Option<Landing> {
        None
    }
}

/// The real app: every command is app-core's, run with `AppCore::run` (the view
/// calls a `Source` on the background executor, never the main thread).
pub struct CoreSource(pub Arc<AppCore>);

/// How long the Timeline re-checks a queued / running OCR job before giving up.
pub const OCR_WAIT: Duration = Duration::from_secs(30);
pub const OCR_POLL: Duration = Duration::from_millis(750);
/// What the Timeline says when [`OCR_WAIT`] runs out.
pub const OCR_STILL_RUNNING: &str = "OCR is still running. Try again in a moment.";

fn audio_segment(a: ai::AudioSegmentDto) -> AudioSegment {
    AudioSegment {
        id: a.id,
        source: match a.source_kind {
            ::app_infra::AudioSegmentSourceKind::Microphone => AudioSource::Microphone,
            _ => AudioSource::SystemAudio,
        },
        segment_index: a.segment_index,
        start_ms: ms(&a.started_at),
        end_ms: ms(&a.ended_at),
    }
}

fn iso(ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn ms(s: &str) -> i64 {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or_default()
}

fn frame(f: ai::FrameDto) -> Frame {
    Frame {
        id: f.id,
        session_id: f.session_id,
        file_path: Arc::from(Path::new(&f.file_path)),
        captured_at_ms: ms(&f.captured_at),
        width: f.width.and_then(|w| u32::try_from(w).ok()),
        height: f.height.and_then(|h| u32::try_from(h).ok()),
        app_bundle_id: f.app_bundle_id,
        app_name: f.app_name,
        window_title: f.window_title,
        url: f.url,
        equivalence_hint: f.equivalence_hint,
    }
}

fn source_kind(kind: ai::frame_preview::FramePreviewSourceKindDto) -> SourceKind {
    use ai::frame_preview::FramePreviewSourceKindDto as K;
    match kind {
        K::OriginalFrame => SourceKind::OriginalFrame,
        K::SegmentFrameFallback => SourceKind::SegmentFrameFallback,
        K::VideoFallback => SourceKind::VideoFallback,
        K::ScrubPreview => SourceKind::ScrubPreview,
    }
}

/// `parseOcrPayload` (frame-ocr.ts): the observations with a usable box, flipped
/// from the provider's lower-left origin to top-left. `None` = not an OCR payload.
pub fn parse_ocr_payload(json: &str) -> Option<Vec<OcrBox>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let observations = value.get("observations")?.as_array()?;
    Some(
        observations
            .iter()
            .filter_map(|o| {
                let bb = o.get("boundingBox")?;
                let n = |k: &str| bb.get(k)?.as_f64().map(|v| v as f32);
                let (x, y, w, h) = (n("x")?, n("y")?, n("width")?, n("height")?);
                Some(OcrBox {
                    x,
                    y: 1. - y - h,
                    w,
                    h,
                    text: o
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    confidence: o.get("confidence").and_then(|c| c.as_f64()).unwrap_or(0.) as f32,
                })
            })
            .collect(),
    )
}

/// One look at an OCR job (`loadOcrFromJob`): `Pending` while queued / running,
/// else its error or its result.
async fn check_ocr_job(job: ai::ProcessingJobDto, infra: &AppInfraState) -> Res<OcrCheck> {
    use ::app_infra::ProcessingJobStatus as S;
    if matches!(job.status, S::Queued | S::Running) {
        return Ok(OcrCheck::Pending {
            job_id: job.id,
            from_frame: None,
        });
    }
    if job.status == S::Failed {
        return Err(job.last_error.unwrap_or_else(|| "OCR job failed".into()));
    }
    let request = ai::GetProcessingResultRequest { job_id: job.id };
    let result = ai::get_processing_result(request, infra)
        .await?
        .ok_or("OCR result not available")?;
    let payload = result.structured_payload_json.as_deref();
    let boxes = payload
        .and_then(parse_ocr_payload)
        .ok_or_else(|| "OCR result payload is missing or invalid".to_string())?;
    Ok(OcrCheck::Done(OcrLoad {
        boxes: Some(boxes),
        provider: ocr_provider_label(
            job.payload_json.as_deref(),
            payload,
            result.processor_version.as_deref(),
        ),
        from_frame: None,
    }))
}

/// `loadOcrForFrame`: the newest completed OCR job, else the newest one.
async fn ocr_for_frame(frame_id: i64, infra: &AppInfraState) -> Res<OcrCheck> {
    let request = ai::ListProcessingJobsRequest {
        subject_type: "frame".into(),
        subject_id: frame_id,
    };
    let mut jobs = ai::list_processing_jobs(request, infra).await?;
    jobs.retain(|j| j.processor == "ocr");
    jobs.sort_by_key(|j| {
        (
            j.status != ::app_infra::ProcessingJobStatus::Completed,
            std::cmp::Reverse(j.id),
        )
    });
    match jobs.into_iter().next() {
        Some(job) => check_ocr_job(job, infra).await,
        None => Ok(OcrCheck::Done(OcrLoad::default())),
    }
}

/// The frame's own OCR, else (never OCR'd) an identical earlier frame's.
async fn ocr_load(frame_id: i64, infra: AppInfraState) -> Res<OcrCheck> {
    let own = ocr_for_frame(frame_id, &infra).await?;
    if !matches!(&own, OcrCheck::Done(l) if l.boxes.is_none()) {
        return Ok(own);
    }
    let request = ai::GetEarliestEarlierEquivalentFrameRequest { frame_id };
    let Some(f) = ai::get_earliest_earlier_equivalent_frame(request, &infra).await? else {
        return Ok(own);
    };
    Ok(match ocr_for_frame(f.id, &infra).await? {
        OcrCheck::Done(l) => OcrCheck::Done(OcrLoad {
            from_frame: Some(f.id),
            ..l
        }),
        OcrCheck::Pending { job_id, .. } => OcrCheck::Pending {
            job_id,
            from_frame: Some(f.id),
        },
    })
}

async fn ocr_job(job_id: i64, infra: AppInfraState) -> Res<OcrCheck> {
    let request = ai::GetProcessingJobRequest { job_id };
    let job = ai::get_processing_job(request, &infra)
        .await?
        .ok_or("OCR job not found")?;
    check_ocr_job(job, &infra).await
}

impl CoreSource {
    fn infra(&self) -> Res<AppInfraState> {
        self.0
            .try_state::<AppInfraState>()
            .map(|i| i.as_ref().clone())
            .ok_or_else(|| "The capture index isn't open yet.".to_string())
    }

    fn range(start_ms: i64, end_ms: i64) -> ai::FrameCapturedAtRangeRequest {
        ai::FrameCapturedAtRangeRequest {
            captured_at_start: iso(start_ms),
            captured_at_end: iso(end_ms),
            exclude_lock_screen: false,
        }
    }
}

impl Source for CoreSource {
    fn list_frames(&self, before_id: Option<i64>, limit: usize) -> Res<Vec<Frame>> {
        let request = ai::ListFramesRequest {
            session_id: None,
            before_id,
            limit: Some(limit as u32),
            offset: None,
        };
        let infra = self.infra()?;
        let rows = self
            .0
            .run(async move { ai::list_frames(Some(request), &infra).await })?;
        Ok(rows.into_iter().map(frame).collect())
    }

    fn window_around(&self, frame_id: i64, newer: usize, older: usize) -> Res<FocusedWindow> {
        let request = ai::GetTimelineWindowAroundFrameRequest {
            frame_id,
            newer_limit: newer as u32,
            older_limit: older as u32,
        };
        let infra = self.infra()?;
        let w = self
            .0
            .run(async move { ai::get_timeline_window_around_frame(request, &infra).await })?;
        Ok(FocusedWindow {
            frames: w.frames.into_iter().map(frame).collect(),
            target_index: w.target_index,
            has_newer: w.has_newer,
            has_older: w.has_older,
        })
    }

    fn latest_in_range(&self, start_ms: i64, end_ms: i64) -> Res<Option<Frame>> {
        let request = Self::range(start_ms, end_ms);
        let infra = self.infra()?;
        let found = self
            .0
            .run(async move { ai::get_latest_frame_in_range(request, &infra).await })?;
        Ok(found.map(frame))
    }

    fn frame_times(&self, start_ms: i64, end_ms: i64) -> Res<Vec<i64>> {
        let request = Self::range(start_ms, end_ms);
        let infra = self.infra()?;
        let rows = self
            .0
            .run(async move { ai::list_frame_summaries_in_range(request, &infra).await })?;
        Ok(rows.iter().map(|f| ms(&f.captured_at)).collect())
    }

    fn frame_preview(&self, frame_id: i64) -> Res<Option<Preview>> {
        use ai::frame_preview::{
            get_frame_preview, FramePreviewVideoScopeDto, GetFramePreviewRequest,
        };
        self.infra()?;
        let request = GetFramePreviewRequest {
            frame_id,
            video_scope: Some(FramePreviewVideoScopeDto::ActiveFrame),
        };
        let core = self.0.clone();
        let preview = self
            .0
            .run(async move { get_frame_preview(&core, request).await })?;
        Ok(preview.map(|p| Preview {
            file_path: Arc::from(Path::new(&p.file_path)),
            mime_type: p.mime_type,
            source_kind: source_kind(p.source_kind),
            redactions: p.secret_redaction_count,
        }))
    }

    fn scrub_availability(
        &self,
        start_ms: i64,
        end_ms: i64,
        enqueue: bool,
    ) -> Res<Vec<ScrubInterval>> {
        use ai::frame_preview::{
            get_scrub_preview_availability, GetScrubPreviewAvailabilityRequest,
            ScrubPreviewAvailabilityStatusDto as Status,
        };
        self.infra()?;
        let request = GetScrubPreviewAvailabilityRequest {
            start_unix_ms: start_ms,
            end_unix_ms: end_ms,
            enqueue_missing: Some(enqueue),
        };
        let core = self.0.clone();
        let dto = self
            .0
            .run(async move { get_scrub_preview_availability(&core, request).await })?;
        Ok(dto
            .intervals
            .into_iter()
            .map(|i| ScrubInterval {
                segment_key: i.segment_cache_key,
                start_offset_ms: i.interval_start_video_offset_ms,
                start_ms: i.interval_start_unix_ms,
                end_ms: i.interval_end_unix_ms,
                queued: i.status == Status::Queued,
                preview: i.preview.map(|p| Preview {
                    file_path: Arc::from(Path::new(&p.file_path)),
                    mime_type: p.mime_type,
                    source_kind: source_kind(p.source_kind),
                    redactions: 0,
                }),
            })
            .collect())
    }

    fn cancel_video(&self) {
        if self.infra().is_ok() {
            let _ = ai::frame_preview::cancel_active_frame_preview_video_requests(&self.0);
        }
    }

    fn audio_segments(&self, start_ms: i64, end_ms: i64) -> Res<Vec<AudioSegment>> {
        let request = ai::ListAudioSegmentsRequest {
            captured_at_start: iso(start_ms),
            captured_at_end: iso(end_ms),
        };
        let infra = self.infra()?;
        let rows = self
            .0
            .run(async move { ai::list_audio_segments(request, &infra).await })?;
        Ok(rows.into_iter().map(audio_segment).collect())
    }

    fn audio_segment(&self, id: i64) -> Res<Option<AudioSegment>> {
        let request = ai::GetAudioSegmentRequest {
            audio_segment_id: id,
        };
        let infra = self.infra()?;
        let found = self
            .0
            .run(async move { ai::get_audio_segment(request, &infra).await })?;
        Ok(found.map(audio_segment))
    }

    fn episodes(&self, start_ms: i64, end_ms: i64) -> Res<Vec<Activity>> {
        use app_core::user_context::commands::list_user_context_activities;
        let infra = self.infra()?;
        self.0.run(async move {
            list_user_context_activities(&infra, None, None, Some(start_ms), Some(end_ms)).await
        })
    }

    fn app_icons(&self, bundle_ids: &[String]) -> Vec<(String, PathBuf)> {
        use app_core::native_capture::app_candidates::{resolve_app_icons, ResolveAppIconsRequest};
        let request = ResolveAppIconsRequest {
            bundle_ids: bundle_ids.to_vec(),
        };
        let core = self.0.clone();
        self.0
            .run(async move { resolve_app_icons(&core, request).await })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| Some((r.bundle_id, PathBuf::from(r.icon_path?))))
            .collect()
    }

    fn match_boxes(&self, frame_id: i64, query: &str, by_meaning: bool) -> Res<Vec<MatchBox>> {
        use ai::quick_recall::{get_frame_match_boxes, GetFrameMatchBoxesRequest};
        let request = GetFrameMatchBoxesRequest {
            frame_id,
            query: query.into(),
            by_meaning,
        };
        let infra = self.infra()?;
        self.0
            .run(async move { get_frame_match_boxes(request, &infra).await })
    }

    /// Chat's peek: waits out a queued / running job here, on its own thread.
    // shortcut: a blocking poll on Chat's background thread; move it to the
    // Timeline's timer re-check (`ocr_overlay`) when Chat's peek owns one.
    fn ocr(&self, frame_id: i64) -> Res<Option<Vec<OcrBox>>> {
        let started = std::time::Instant::now();
        let mut check = self.ocr_load(frame_id)?;
        loop {
            match check {
                OcrCheck::Done(l) => return Ok(l.boxes),
                OcrCheck::Pending { job_id, .. } => {
                    if started.elapsed() > OCR_WAIT {
                        return Err(OCR_STILL_RUNNING.into());
                    }
                    std::thread::sleep(OCR_POLL);
                    check = self.ocr_job(job_id)?;
                }
            }
        }
    }

    fn ocr_load(&self, frame_id: i64) -> Res<OcrCheck> {
        let infra = self.infra()?;
        self.0.run(ocr_load(frame_id, infra))
    }

    fn ocr_job(&self, job_id: i64) -> Res<OcrCheck> {
        let infra = self.infra()?;
        self.0.run(ocr_job(job_id, infra))
    }

    fn rerun_ocr(&self, frame_id: i64, source_image: Option<PathBuf>) -> Res<OcrCheck> {
        let settings = self
            .0
            .try_state::<app_core::recording_settings::RecordingSettingsState>()
            .ok_or("Settings aren't loaded yet.")?;
        let payload_json = source_image.map(|path| {
            serde_json::json!({
                "provider": "apple_vision",
                "options": { "mnemaSourceImagePath": path },
            })
            .to_string()
        });
        let request = ai::ReprocessCapturedFrameOcrRequest {
            frame_id,
            payload_json,
        };
        let infra = self.infra()?;
        self.0.run(async move {
            let result = ai::reprocess_captured_frame_ocr(request, &infra, &settings).await?;
            check_ocr_job(result.job, &infra).await
        })
    }

    fn earlier_equivalent(&self, frame_id: i64) -> Res<Option<Frame>> {
        let request = ai::GetEarliestEarlierEquivalentFrameRequest { frame_id };
        let infra = self.infra()?;
        let found = self
            .0
            .run(async move { ai::get_earliest_earlier_equivalent_frame(request, &infra).await })?;
        Ok(found.map(frame))
    }

    fn open_captured_url(&self, frame_id: i64) -> Res<bool> {
        let (core, infra) = (self.0.clone(), self.infra()?);
        self.0
            .run(async move { ai::open_captured_url(frame_id, &core, &infra).await })
    }

    fn settings(&self) -> TimelineSettings {
        let Some(state) = self
            .0
            .try_state::<app_core::recording_settings::RecordingSettingsState>()
        else {
            return TimelineSettings::default();
        };
        let s = app_core::recording_settings::current_recording_settings(&state);
        TimelineSettings {
            follow_live: s.follow_timeline_live,
            ocr_enabled: s.ocr.enabled,
            keep_days: match s.retention_policy {
                RetentionPolicy::Never => None,
                RetentionPolicy::Days7 => Some(7),
                RetentionPolicy::Days14 => Some(14),
                RetentionPolicy::Days30 => Some(30),
            },
        }
    }

    /// Nothing queues for a cold window here: Quick Recall and a broker link
    /// call `TimelineView::land` / `jump_to` on the live view themselves.
    fn take_landing(&self) -> Option<Landing> {
        None
    }
}
