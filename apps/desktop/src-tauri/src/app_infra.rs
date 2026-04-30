use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use capture_screen::ScreenFrameArtifact;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub type AppInfraState = Arc<::app_infra::AppInfra>;
pub type FramePreviewCacheState = Mutex<FramePreviewCache>;

const APP_INFRA_BASE_DIR_NAME: &str = ".z";
const RECORDINGS_DIR_NAME: &str = "recordings";
const FRAME_PREVIEW_CACHE_MAX_ENTRIES: usize = 256;

/// Returns the recordings root directory: `<saveDirectory>/.z/recordings`.
///
/// All capture output (segments, audio, etc.) is placed under this root so
/// that recordings live inside the managed `.z` tree alongside the database
/// and other app-infra state.
pub fn recordings_root_dir(save_directory: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(save_directory)
        .join(APP_INFRA_BASE_DIR_NAME)
        .join(RECORDINGS_DIR_NAME)
}
const PROCESSING_WORKER_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROCESSING_WORKER_ERROR_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedFramePreview {
    preview: FramePreviewDto,
    cached_at: Instant,
}

#[derive(Debug, Default)]
pub struct FramePreviewCache {
    entries: HashMap<i64, CachedFramePreview>,
}

impl FramePreviewCache {
    fn get(&mut self, frame_id: i64, ttl: Duration, now: Instant) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        self.entries.get(&frame_id).map(|entry| entry.preview.clone())
    }

    fn insert(&mut self, frame_id: i64, preview: FramePreviewDto, ttl: Duration, now: Instant) {
        self.evict_expired(ttl, now);
        self.entries.insert(
            frame_id,
            CachedFramePreview {
                preview,
                cached_at: now,
            },
        );
        self.evict_oldest_excess_entries();
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn evict_expired(&mut self, ttl: Duration, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.cached_at) < ttl);
    }

    fn evict_oldest_excess_entries(&mut self) {
        while self.entries.len() > FRAME_PREVIEW_CACHE_MAX_ENTRIES {
            let Some(oldest_frame_id) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(frame_id, _)| *frame_id)
            else {
                break;
            };

            self.entries.remove(&oldest_frame_id);
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedAppInfraBaseDir {
    save_directory: String,
    base_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitDebugCpuJobRequest {
    pub document_name: String,
    pub source_text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertFrameAndEnqueueProcessingJobRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub processor: String,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertFrameAndEnqueueOcrRequest {
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFramePreviewRequest {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFramesRequest {
    pub session_id: Option<String>,
    pub before_id: Option<i64>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameCapturedAtRangeRequest {
    pub captured_at_start: String,
    pub captured_at_end: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingJobRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingJobsRequest {
    pub subject_type: String,
    pub subject_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetProcessingResultRequest {
    pub job_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProcessingResultsRequest {
    pub subject_type: String,
    pub subject_id: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppJobDto {
    pub id: i64,
    pub kind: String,
    pub status: ::app_infra::BackgroundJobStatus,
    pub payload_json: Option<String>,
    pub result_text: Option<String>,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDto {
    pub id: i64,
    pub session_id: String,
    pub file_path: String,
    pub captured_at: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSummaryDto {
    pub id: i64,
    pub captured_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FramePreviewSourceKindDto {
    OriginalFrame,
    SegmentFrameFallback,
    VideoFallback,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FramePreviewDto {
    pub mime_type: String,
    pub data_base64: String,
    pub source_kind: FramePreviewSourceKindDto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSegmentPreviewPaths {
    workspace_dir: PathBuf,
    video_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJobDto {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub status: ::app_infra::ProcessingJobStatus,
    pub attempt_count: i64,
    pub payload_json: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingResultDto {
    pub id: i64,
    pub job_id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub processor: String,
    pub result_text: Option<String>,
    pub structured_payload_json: Option<String>,
    pub processor_version: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameProcessingJobDto {
    pub frame: FrameDto,
    pub job: ProcessingJobDto,
}

impl From<::app_infra::BackgroundJob> for AppJobDto {
    fn from(job: ::app_infra::BackgroundJob) -> Self {
        Self {
            id: job.id,
            kind: job.kind,
            status: job.status,
            payload_json: job.payload_json,
            result_text: job.result_text,
            attempt_count: job.attempt_count,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

impl From<::app_infra::Frame> for FrameDto {
    fn from(frame: ::app_infra::Frame) -> Self {
        Self {
            id: frame.id,
            session_id: frame.session_id,
            file_path: frame.file_path,
            captured_at: frame.captured_at,
            width: frame.width,
            height: frame.height,
            content_fingerprint: frame.content_fingerprint,
            created_at: frame.created_at,
            updated_at: frame.updated_at,
        }
    }
}

impl From<::app_infra::FrameSummary> for FrameSummaryDto {
    fn from(frame: ::app_infra::FrameSummary) -> Self {
        Self {
            id: frame.id,
            captured_at: frame.captured_at,
        }
    }
}

impl From<::app_infra::ProcessingJob> for ProcessingJobDto {
    fn from(job: ::app_infra::ProcessingJob) -> Self {
        Self {
            id: job.id,
            subject_type: job.subject_type,
            subject_id: job.subject_id,
            processor: job.processor,
            status: job.status,
            attempt_count: job.attempt_count,
            payload_json: job.payload_json,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
        }
    }
}

impl From<::app_infra::ProcessingResult> for ProcessingResultDto {
    fn from(result: ::app_infra::ProcessingResult) -> Self {
        Self {
            id: result.id,
            job_id: result.job_id,
            subject_type: result.subject_type,
            subject_id: result.subject_id,
            processor: result.processor,
            result_text: result.result_text,
            structured_payload_json: result.structured_payload_json,
            processor_version: result.processor_version,
            created_at: result.created_at,
        }
    }
}

impl From<::app_infra::FrameProcessingJob> for FrameProcessingJobDto {
    fn from(value: ::app_infra::FrameProcessingJob) -> Self {
        Self {
            frame: value.frame.into(),
            job: value.job.into(),
        }
    }
}

impl From<SubmitDebugCpuJobRequest> for ::app_infra::DebugCpuJobRequest {
    fn from(request: SubmitDebugCpuJobRequest) -> Self {
        Self {
            document_name: request.document_name,
            source_text: request.source_text,
        }
    }
}

impl InsertFrameAndEnqueueProcessingJobRequest {
    fn into_frame_pipeline_request(self) -> ::app_infra::FramePipelineRequest {
        let Self {
            session_id,
            file_path,
            captured_at,
            width,
            height,
            content_fingerprint,
            processor,
            payload_json,
        } = self;

        let mut frame = ::app_infra::NewFrame::new(session_id, file_path, captured_at);

        if let (Some(width), Some(height)) = (width, height) {
            frame = frame.with_dimensions(width, height);
        }

        if let Some(content_fingerprint) = content_fingerprint {
            frame = frame.with_content_fingerprint(content_fingerprint);
        }

        let mut request = ::app_infra::FramePipelineRequest::new(frame, processor);

        if let Some(payload_json) = payload_json {
            request = request.with_payload_json(payload_json);
        }

        request
    }
}

impl From<InsertFrameAndEnqueueOcrRequest> for InsertFrameAndEnqueueProcessingJobRequest {
    fn from(request: InsertFrameAndEnqueueOcrRequest) -> Self {
        Self {
            session_id: request.session_id,
            file_path: request.file_path,
            captured_at: request.captured_at,
            width: request.width,
            height: request.height,
            content_fingerprint: request.content_fingerprint,
            processor: ::app_infra::OCR_PROCESSOR.to_string(),
            payload_json: request.payload_json,
        }
    }
}

fn captured_at_from_unix_ms(unix_ms: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn fingerprint_string(content_fingerprint: Option<u64>) -> Option<String> {
    content_fingerprint.map(|value| format!("{value:016x}"))
}

fn frame_preview_payload(
    bytes: Vec<u8>,
    source_kind: FramePreviewSourceKindDto,
) -> FramePreviewDto {
    FramePreviewDto {
        mime_type: "image/png".to_string(),
        data_base64: BASE64_STANDARD.encode(bytes),
        source_kind,
    }
}

fn resolve_segment_preview_paths(frame_file_path: &Path) -> Option<ResolvedSegmentPreviewPaths> {
    let frames_dir = frame_file_path.parent()?;
    if frames_dir.file_name()?.to_str()? != "frames" {
        return None;
    }

    let workspace_dir = frames_dir.parent()?;
    let workspace_name = workspace_dir.file_name()?.to_str()?;
    let visible_segment_name = workspace_name.strip_prefix('.')?;
    let video_path = workspace_dir
        .parent()?
        .join(format!("{visible_segment_name}.mov"));

    Some(ResolvedSegmentPreviewPaths {
        workspace_dir: workspace_dir.to_path_buf(),
        video_path,
    })
}

fn parse_frame_unix_ms_from_path(frame_file_path: &Path) -> Option<i128> {
    let stem = frame_file_path.file_stem()?.to_str()?;
    let raw = stem.strip_prefix("frame-")?;
    let (unix_ms, _) = raw.rsplit_once('-')?;
    unix_ms.parse().ok()
}

fn parse_captured_at_unix_ms(captured_at: &str) -> Option<i128> {
    OffsetDateTime::parse(captured_at, &Rfc3339)
        .ok()
        .map(|timestamp| timestamp.unix_timestamp_nanos() / 1_000_000)
}

fn estimate_frame_preview_offset_seconds(
    frame: &::app_infra::Frame,
    related_frames: &[::app_infra::Frame],
) -> f64 {
    let target_unix_ms = frame_preview_unix_ms(frame);

    let first_unix_ms = related_frames.first().and_then(frame_preview_unix_ms);

    match (target_unix_ms, first_unix_ms) {
        (Some(target), Some(first)) if target >= first => (target - first) as f64 / 1000.0,
        _ => 0.0,
    }
}

fn frame_preview_unix_ms(frame: &::app_infra::Frame) -> Option<i128> {
    parse_frame_unix_ms_from_path(Path::new(&frame.file_path))
        .or_else(|| parse_captured_at_unix_ms(&frame.captured_at))
}

fn read_nearest_segment_frame_preview(
    frame: &::app_infra::Frame,
    related_frames: &[::app_infra::Frame],
) -> std::io::Result<Option<Vec<u8>>> {
    let target_unix_ms = frame_preview_unix_ms(frame);
    let mut best_match: Option<(bool, i128, usize, &str)> = None;

    for (index, related_frame) in related_frames.iter().enumerate() {
        let candidate_path = Path::new(&related_frame.file_path);
        if !candidate_path.is_file() {
            continue;
        }

        let candidate_unix_ms = frame_preview_unix_ms(related_frame);
        let (has_distance, distance) = match (target_unix_ms, candidate_unix_ms) {
            (Some(target), Some(candidate)) => (true, (target - candidate).abs()),
            _ => (false, 0),
        };

        let should_replace = match best_match {
            Some((best_has_distance, best_distance, best_index, _)) => {
                (!has_distance, distance, index)
                    < (!best_has_distance, best_distance, best_index)
            }
            None => true,
        };

        if should_replace {
            best_match = Some((has_distance, distance, index, &related_frame.file_path));
        }
    }

    best_match
        .map(|(_, _, _, file_path)| fs::read(file_path))
        .transpose()
}

#[cfg(target_os = "macos")]
fn png_bytes_from_cg_image(image: &cidre::cg::Image) -> Result<Vec<u8>, String> {
    use cidre::{cf, cg, ut};
    use tempfile::NamedTempFile;

    let png_type_identifier = ut::Type::png().id();
    let output_file = NamedTempFile::new()
        .map_err(|error| format!("failed to create temporary PNG output file: {error}"))?;
    let output_path = output_file.path();
    let output_url = cf::Url::with_file_path(&output_path).ok_or_else(|| {
        format!(
            "failed to create temporary PNG output URL at {}",
            output_path.display()
        )
    })?;
    let mut image_destination = cg::ImageDst::with_url(output_url.as_ref(), png_type_identifier.as_cf(), 1)
        .ok_or_else(|| {
            format!(
                "failed to create temporary PNG image destination at {}",
                output_path.display()
            )
        })?;
    image_destination.add_image(image, None);

    if !image_destination.finalize() {
        return Err(format!(
            "failed to finalize temporary PNG image destination at {}",
            output_path.display()
        ));
    }

    fs::read(output_path).map_err(|error| {
        format!(
            "failed to read temporary PNG output at {}: {error}",
            output_path.display()
        )
    })
}

#[cfg(target_os = "macos")]
fn extract_preview_png_from_video_blocking(
    video_path: PathBuf,
    offset_seconds: f64,
) -> Result<Vec<u8>, String> {
    use cidre::{av, blocks, cm, ns};
    use std::sync::mpsc;

    let video_url = ns::Url::with_fs_path_str(&video_path.to_string_lossy(), false);
    let asset = av::UrlAsset::with_url(&video_url, None)
        .ok_or_else(|| format!("failed to open video asset at {}", video_path.display()))?;

    let mut image_generator = av::AssetImageGenerator::with_asset(&asset);
    image_generator.set_applies_preferred_track_transform(true);

    let duration_seconds = asset.duration().as_secs();
    let clamped_offset_seconds = if duration_seconds.is_finite() && duration_seconds > 0.0 {
        offset_seconds.clamp(0.0, (duration_seconds - 0.001).max(0.0))
    } else {
        0.0
    };

    let request_time = cm::Time::with_secs(clamped_offset_seconds, 600);
    let (sender, receiver) = mpsc::sync_channel(1);
    let video_path_for_error = video_path.clone();
    let mut callback = blocks::EscBlock::new3(
        move |image: Option<&cidre::cg::Image>,
              _actual_time: cm::Time,
              error: Option<&ns::Error>| {
            let result = if let Some(error) = error {
                Err(format!(
                    "failed to extract preview from video {}: {error}",
                    video_path_for_error.display()
                ))
            } else if let Some(image) = image {
                png_bytes_from_cg_image(image)
            } else {
                Err(format!(
                    "failed to extract preview from video {}: empty image result",
                    video_path_for_error.display()
                ))
            };

            let _ = sender.send(result);
        },
    );

    image_generator.cg_image_for_time_ch(request_time, &mut callback);
    receiver
        .recv()
        .map_err(|error| format!("failed to receive extracted preview bytes: {error}"))?
}

#[cfg(target_os = "macos")]
async fn extract_preview_png_from_video(
    video_path: &Path,
    offset_seconds: f64,
) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking({
        let video_path = video_path.to_path_buf();
        move || extract_preview_png_from_video_blocking(video_path, offset_seconds)
    })
    .await
    .map_err(|error| format!("failed to join video preview extraction task: {error}"))?
}

#[cfg(not(target_os = "macos"))]
async fn extract_preview_png_from_video(
    _video_path: &Path,
    _offset_seconds: f64,
) -> Result<Vec<u8>, String> {
    Err("video frame preview fallback is only supported on macOS".to_string())
}

async fn get_frame_preview_inner(
    infra: &::app_infra::AppInfra,
    frame_id: i64,
) -> ::app_infra::Result<Option<FramePreviewDto>> {
    let Some(frame) = infra.get_frame(frame_id).await? else {
        return Ok(None);
    };

    let frame_file_path = PathBuf::from(&frame.file_path);
    if frame_file_path.is_file() {
        let bytes = fs::read(&frame_file_path)?;
        return Ok(Some(frame_preview_payload(
            bytes,
            FramePreviewSourceKindDto::OriginalFrame,
        )));
    }

    let segment_paths = resolve_segment_preview_paths(&frame_file_path).ok_or_else(|| {
        ::app_infra::AppInfraError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "unable to infer segment video path from frame artifact path {}",
                frame.file_path
            ),
        ))
    })?;

    let workspace_prefix = format!("{}/", segment_paths.workspace_dir.to_string_lossy());
    let related_frames = infra
        .list_frames_for_segment_workspace(&frame.session_id, &workspace_prefix)
        .await?;

    if !segment_paths.video_path.is_file() {
        if let Some(bytes) = read_nearest_segment_frame_preview(&frame, &related_frames)? {
            return Ok(Some(frame_preview_payload(
                bytes,
                FramePreviewSourceKindDto::SegmentFrameFallback,
            )));
        }

        return Err(::app_infra::AppInfraError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "segment video does not exist for frame {} at {}",
                frame.id,
                segment_paths.video_path.display()
            ),
        )));
    }

    let offset_seconds = estimate_frame_preview_offset_seconds(&frame, &related_frames);
    let bytes = extract_preview_png_from_video(&segment_paths.video_path, offset_seconds)
        .await
        .map_err(|error| ::app_infra::AppInfraError::Io(std::io::Error::other(error)))?;

    Ok(Some(frame_preview_payload(
        bytes,
        FramePreviewSourceKindDto::VideoFallback,
    )))
}

fn preview_cache_ttl(settings: &crate::native_capture::RecordingSettingsState) -> Duration {
    let ttl_seconds = settings
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .preview_cache_ttl_seconds;

    Duration::from_secs(ttl_seconds)
}

pub async fn persist_screen_frame_artifact(
    infra: &::app_infra::AppInfra,
    session_id: &str,
    artifact: ScreenFrameArtifact,
) -> ::app_infra::Result<::app_infra::FrameOcrEnqueueResult> {
    let mut frame = ::app_infra::NewFrame::new(
        session_id,
        artifact.file_path,
        captured_at_from_unix_ms(artifact.captured_at_unix_ms),
    );

    if let (Some(width), Some(height)) = (artifact.width, artifact.height) {
        frame = frame.with_dimensions(i64::from(width), i64::from(height));
    }

    if let Some(content_fingerprint) = fingerprint_string(artifact.content_fingerprint) {
        frame = frame.with_content_fingerprint(content_fingerprint);
    }

    infra
        .insert_frame_into_batch_and_maybe_enqueue_ocr_job(&frame, None)
        .await
}

pub fn initialize(app: &mut tauri::App) -> Result<(), String> {
    let resolved_base_dir = resolve_base_dir(app.handle())?;
    crate::native_capture_debug_log::log(format!(
        "initializing app infrastructure (save_directory='{}', base_dir='{}')",
        resolved_base_dir.save_directory,
        resolved_base_dir.base_dir.display()
    ));

    let infra = tauri::async_runtime::block_on(::app_infra::AppInfra::initialize(
        &resolved_base_dir.base_dir,
    ))
    .map_err(|error| {
        crate::native_capture_debug_log::log(format!(
            "failed to initialize app infrastructure (save_directory='{}', base_dir='{}'): {error}",
            resolved_base_dir.save_directory,
            resolved_base_dir.base_dir.display()
        ));

        format!(
            "failed to initialize app infrastructure at {}: {error}",
            resolved_base_dir.base_dir.display()
        )
    })?;
    let infra = Arc::new(infra);
    let frame_preview_cache = FramePreviewCacheState::default();

    if !app.manage(Arc::clone(&infra)) {
        crate::native_capture_debug_log::log(
            "app infrastructure state was already initialized; refusing duplicate setup",
        );
        return Err("app infrastructure state was already initialized".to_string());
    }

    if !app.manage(frame_preview_cache) {
        crate::native_capture_debug_log::log(
            "frame preview cache state was already initialized; refusing duplicate setup",
        );
        return Err("frame preview cache state was already initialized".to_string());
    }

    crate::native_capture_debug_log::log(format!(
        "initialized app infrastructure successfully (base_dir='{}')",
        resolved_base_dir.base_dir.display()
    ));

    spawn_processing_worker(infra, resolved_base_dir.base_dir);

    Ok(())
}

fn spawn_processing_worker(infra: AppInfraState, base_dir: PathBuf) {
    let base_dir_display = base_dir.display().to_string();
    crate::native_capture_debug_log::log(format!(
        "starting app infrastructure processing worker (base_dir='{}', idle_poll_ms={}, error_retry_ms={})",
        base_dir_display,
        PROCESSING_WORKER_IDLE_POLL_INTERVAL.as_millis(),
        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
    ));

    tauri::async_runtime::spawn(async move {
        let mut consecutive_failures = 0u64;

        loop {
            match process_pending_jobs_once(&infra).await {
                Ok(Some(_)) => {
                    if consecutive_failures > 0 {
                        crate::native_capture_debug_log::log(format!(
                            "app infrastructure processing worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    continue;
                }
                Ok(None) => {
                    if consecutive_failures > 0 {
                        crate::native_capture_debug_log::log(format!(
                            "app infrastructure processing worker recovered after {} failed iteration(s) (base_dir='{}')",
                            consecutive_failures, base_dir_display
                        ));
                        consecutive_failures = 0;
                    }

                    tokio::time::sleep(PROCESSING_WORKER_IDLE_POLL_INTERVAL).await;
                }
                Err(error) => {
                    consecutive_failures += 1;
                    crate::native_capture_debug_log::log(format!(
                        "app infrastructure processing worker iteration failed (base_dir='{}', consecutive_failures={}, retry_in_ms={}): {error}",
                        base_dir_display,
                        consecutive_failures,
                        PROCESSING_WORKER_ERROR_RETRY_INTERVAL.as_millis()
                    ));
                    tokio::time::sleep(PROCESSING_WORKER_ERROR_RETRY_INTERVAL).await;
                }
            }
        }
    });
}

async fn process_pending_jobs_once(
    infra: &::app_infra::AppInfra,
) -> ::app_infra::Result<Option<()>> {
    let did_processing = infra.process_next_processing_job().await?.is_some();

    let did_finalize = match infra.process_next_frame_batch_job().await {
        Ok(result) => result.is_some(),
        Err(error) => {
            crate::native_capture_debug_log::log(format!(
                "app infrastructure frame-batch finalization failed after state update; worker will continue: {error}"
            ));
            true
        }
    };

    if did_processing || did_finalize {
        Ok(Some(()))
    } else {
        Ok(None)
    }
}

fn resolve_base_dir(app_handle: &tauri::AppHandle) -> Result<ResolvedAppInfraBaseDir, String> {
    let settings = crate::native_capture_settings::load_recording_settings_or_default(app_handle);
    let base_dir = resolve_base_dir_from_save_directory(&settings.save_directory);

    crate::native_capture_debug_log::log(format!(
        "resolved app infrastructure base directory (save_directory='{}', base_dir='{}')",
        settings.save_directory,
        base_dir.display()
    ));

    Ok(ResolvedAppInfraBaseDir {
        save_directory: settings.save_directory,
        base_dir,
    })
}

fn resolve_base_dir_from_save_directory(save_directory: &str) -> PathBuf {
    PathBuf::from(save_directory).join(APP_INFRA_BASE_DIR_NAME)
}

fn processing_subject(subject_type: String, subject_id: i64) -> ::app_infra::ProcessingSubject {
    ::app_infra::ProcessingSubject::new(subject_type, subject_id)
}

async fn insert_frame_and_enqueue_processing_job_inner(
    infra: &::app_infra::AppInfra,
    request: InsertFrameAndEnqueueProcessingJobRequest,
) -> ::app_infra::Result<FrameProcessingJobDto> {
    let request = request.into_frame_pipeline_request();

    infra
        .frame_pipeline()
        .enqueue(&request)
        .await
        .map(FrameProcessingJobDto::from)
}

#[tauri::command]
pub async fn get_app_infra_status(
    state: tauri::State<'_, AppInfraState>,
) -> Result<::app_infra::AppInfraStatus, String> {
    let infra = Arc::clone(&*state);
    infra
        .status()
        .await
        .map_err(|error| format!("failed to read app infrastructure status: {error}"))
}

#[tauri::command]
pub async fn submit_debug_cpu_job(
    request: SubmitDebugCpuJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<AppJobDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .submit_debug_cpu_job(request.into())
        .await
        .map(AppJobDto::from)
        .map_err(|error| format!("failed to submit debug cpu job: {error}"))
}

#[tauri::command]
pub async fn list_app_jobs(
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_jobs()
        .await
        .map(|jobs| jobs.into_iter().map(AppJobDto::from).collect())
        .map_err(|error| format!("failed to list app jobs: {error}"))
}

#[tauri::command]
pub async fn get_app_job(
    request: GetAppJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_job(request.job_id)
        .await
        .map(|job| job.map(AppJobDto::from))
        .map_err(|error| format!("failed to get app job {}: {error}", request.job_id))
}

#[tauri::command]
pub async fn insert_frame_and_enqueue_processing_job(
    request: InsertFrameAndEnqueueProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    insert_frame_and_enqueue_processing_job_inner(&infra, request)
        .await
        .map_err(|error| format!("failed to insert frame and enqueue processing job: {error}"))
}

#[tauri::command]
pub async fn insert_frame_and_enqueue_ocr(
    request: InsertFrameAndEnqueueOcrRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    insert_frame_and_enqueue_processing_job_inner(&infra, request.into())
        .await
        .map_err(|error| format!("failed to insert frame and enqueue ocr job: {error}"))
}

#[tauri::command]
pub async fn list_frames(
    request: Option<ListFramesRequest>,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    let (session_id, before_id, limit, offset) = match request {
        Some(request) => (
            request.session_id,
            request.before_id,
            request.limit,
            request.offset,
        ),
        None => (None, None, None, None),
    };

    infra
        .list_frames(session_id.as_deref(), before_id, limit, offset)
        .await
        .map(|frames| frames.into_iter().map(FrameDto::from).collect())
        .map_err(|error| format!("failed to list frames: {error}"))
}

#[tauri::command]
pub async fn list_frame_summaries_in_range(
    request: FrameCapturedAtRangeRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<FrameSummaryDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_frame_summaries_in_range(&request.captured_at_start, &request.captured_at_end)
        .await
        .map(|frames| frames.into_iter().map(FrameSummaryDto::from).collect())
        .map_err(|error| format!("failed to list frame summaries in range: {error}"))
}

#[tauri::command]
pub async fn get_latest_frame_in_range(
    request: FrameCapturedAtRangeRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_latest_frame_in_range(&request.captured_at_start, &request.captured_at_end)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get latest frame in range: {error}"))
}

#[tauri::command]
pub async fn get_frame(
    request: GetFrameRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get frame {}: {error}", request.frame_id))
}

#[tauri::command]
pub async fn get_frame_preview(
    request: GetFramePreviewRequest,
    state: tauri::State<'_, AppInfraState>,
    cache: tauri::State<'_, FramePreviewCacheState>,
    settings: tauri::State<'_, crate::native_capture::RecordingSettingsState>,
) -> Result<Option<FramePreviewDto>, String> {
    let infra = Arc::clone(&*state);
    let ttl = preview_cache_ttl(&settings);

    if ttl.is_zero() {
        cache.lock().expect("frame preview cache poisoned").clear();
        return get_frame_preview_inner(&infra, request.frame_id)
            .await
            .map_err(|error| format!("failed to get frame preview {}: {error}", request.frame_id));
    }

    let now = Instant::now();
    if let Some(preview) = cache
        .lock()
        .expect("frame preview cache poisoned")
        .get(request.frame_id, ttl, now)
    {
        return Ok(Some(preview));
    }

    let preview = get_frame_preview_inner(&infra, request.frame_id)
        .await
        .map_err(|error| format!("failed to get frame preview {}: {error}", request.frame_id))?;

    if let Some(preview) = preview.as_ref() {
        cache
            .lock()
            .expect("frame preview cache poisoned")
            .insert(request.frame_id, preview.clone(), ttl, now);
    }

    Ok(preview)
}

#[tauri::command]
pub async fn list_processing_jobs(
    request: ListProcessingJobsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_jobs_for_subject(&subject)
        .await
        .map(|jobs| jobs.into_iter().map(ProcessingJobDto::from).collect())
        .map_err(|error| format!("failed to list processing jobs: {error}"))
}

#[tauri::command]
pub async fn get_processing_job(
    request: GetProcessingJobRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_job(request.job_id)
        .await
        .map(|job| job.map(ProcessingJobDto::from))
        .map_err(|error| format!("failed to get processing job {}: {error}", request.job_id))
}

#[tauri::command]
pub async fn get_processing_result(
    request: GetProcessingResultRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Option<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_processing_result_for_job(request.job_id)
        .await
        .map(|result| result.map(ProcessingResultDto::from))
        .map_err(|error| {
            format!(
                "failed to get processing result for job {}: {error}",
                request.job_id
            )
        })
}

#[tauri::command]
pub async fn list_processing_results(
    request: ListProcessingResultsRequest,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<ProcessingResultDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    infra
        .list_processing_results_for_subject(&subject)
        .await
        .map(|results| results.into_iter().map(ProcessingResultDto::from).collect())
        .map_err(|error| format!("failed to list processing results: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[test]
    fn insert_frame_processing_request_maps_optional_dimensions() {
        let request = InsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-a".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: Some(720),
            content_fingerprint: Some("abcd".to_string()),
            processor: "custom-processor".to_string(),
            payload_json: Some("{\"language\":\"eng\"}".to_string()),
        }
        .into_frame_pipeline_request();

        assert_eq!(request.frame.session_id, "session-a");
        assert_eq!(request.frame.file_path, "/tmp/frame.png");
        assert_eq!(request.frame.width, Some(1280));
        assert_eq!(request.frame.height, Some(720));
        assert_eq!(request.frame.content_fingerprint.as_deref(), Some("abcd"));
        assert_eq!(request.processor, "custom-processor");
        assert_eq!(
            request.payload_json.as_deref(),
            Some("{\"language\":\"eng\"}")
        );
    }

    #[test]
    fn insert_frame_processing_request_ignores_partial_dimensions() {
        let request = InsertFrameAndEnqueueProcessingJobRequest {
            session_id: "session-b".to_string(),
            file_path: "/tmp/frame.png".to_string(),
            captured_at: "2026-04-12T10:00:00Z".to_string(),
            width: Some(1280),
            height: None,
            content_fingerprint: None,
            processor: "custom-processor".to_string(),
            payload_json: None,
        }
        .into_frame_pipeline_request();

        assert_eq!(request.frame.width, None);
        assert_eq!(request.frame.height, None);
        assert_eq!(request.processor, "custom-processor");
        assert_eq!(request.payload_json, None);
    }

    #[test]
    fn insert_frame_ocr_request_wraps_generic_processing_request() {
        let request =
            InsertFrameAndEnqueueProcessingJobRequest::from(InsertFrameAndEnqueueOcrRequest {
                session_id: "session-ocr".to_string(),
                file_path: "/tmp/frame-ocr.png".to_string(),
                captured_at: "2026-04-12T10:00:00Z".to_string(),
                width: Some(1920),
                height: Some(1080),
                content_fingerprint: Some("ef01".to_string()),
                payload_json: Some("{\"language\":\"eng\"}".to_string()),
            })
            .into_frame_pipeline_request();

        assert_eq!(request.frame.session_id, "session-ocr");
        assert_eq!(request.frame.file_path, "/tmp/frame-ocr.png");
        assert_eq!(request.frame.width, Some(1920));
        assert_eq!(request.frame.height, Some(1080));
        assert_eq!(request.frame.content_fingerprint.as_deref(), Some("ef01"));
        assert_eq!(request.processor, ::app_infra::OCR_PROCESSOR);
        assert_eq!(
            request.payload_json.as_deref(),
            Some("{\"language\":\"eng\"}")
        );
    }

    #[test]
    fn resolve_base_dir_from_save_directory_uses_hidden_subdirectory() {
        let save_directory = "/tmp/z-recordings";

        assert_eq!(
            resolve_base_dir_from_save_directory(save_directory),
            PathBuf::from(save_directory).join(APP_INFRA_BASE_DIR_NAME)
        );
    }

    #[test]
    fn resolve_base_dir_from_save_directory_keeps_database_out_of_segment_root() {
        let save_directory = "/tmp/z-recordings/session-output";
        let base_dir = resolve_base_dir_from_save_directory(save_directory);

        assert_eq!(base_dir.parent(), Some(Path::new(save_directory)));
        assert_eq!(
            base_dir.file_name().and_then(|value| value.to_str()),
            Some(".z")
        );
    }

    #[test]
    fn recordings_root_dir_nests_under_dot_z() {
        let save_directory = "/tmp/z-recordings";

        assert_eq!(
            super::recordings_root_dir(save_directory),
            PathBuf::from(save_directory).join(".z").join("recordings")
        );
    }

    #[test]
    fn recordings_root_dir_is_child_of_base_dir() {
        let save_directory = "/tmp/z-recordings";
        let base_dir = resolve_base_dir_from_save_directory(save_directory);
        let recordings_root = super::recordings_root_dir(save_directory);

        assert_eq!(recordings_root.parent(), Some(base_dir.as_path()));
    }

    #[test]
    fn resolve_segment_preview_paths_maps_hidden_workspace_to_visible_video() {
        let frame_path =
            Path::new("/tmp/2026/04/12/.session-abc-segment-0004/frames/frame-1744459200123-7.png");

        let resolved =
            resolve_segment_preview_paths(frame_path).expect("segment paths should resolve");

        assert_eq!(
            resolved.workspace_dir,
            PathBuf::from("/tmp/2026/04/12/.session-abc-segment-0004")
        );
        assert_eq!(
            resolved.video_path,
            PathBuf::from("/tmp/2026/04/12/session-abc-segment-0004.mov")
        );
    }

    #[test]
    fn estimate_frame_preview_offset_seconds_uses_segment_frame_times() {
        let frame = ::app_infra::Frame {
            id: 2,
            session_id: "session-estimate".to_string(),
            file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459201500-1.png"
                .to_string(),
            captured_at: "2025-04-12T10:00:01.500Z".to_string(),
            width: None,
            height: None,
            content_fingerprint: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let related_frames = vec![::app_infra::Frame {
            id: 1,
            session_id: "session-estimate".to_string(),
            file_path: "/tmp/.session-estimate-segment-0004/frames/frame-1744459200000-0.png"
                .to_string(),
            captured_at: "2025-04-12T10:00:00Z".to_string(),
            width: None,
            height: None,
            content_fingerprint: None,
            created_at: String::new(),
            updated_at: String::new(),
        }];

        let offset_seconds = estimate_frame_preview_offset_seconds(&frame, &related_frames);

        assert!((offset_seconds - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn get_frame_preview_inner_returns_original_frame_bytes_when_png_exists() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-original");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let frame_path = dir.path().join("frame-preview.png");
            let frame_bytes = b"not-a-real-png-but-preview-bytes";
            fs::write(&frame_path, frame_bytes).expect("frame preview file should be written");

            let stored_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should be inserted");

            let preview = get_frame_preview_inner(&infra, stored_frame.id)
                .await
                .expect("preview should load")
                .expect("preview should exist");

            assert_eq!(preview.mime_type, "image/png");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::OriginalFrame
            );
            assert_eq!(preview.data_base64, BASE64_STANDARD.encode(frame_bytes));
        });
    }

    #[test]
    fn get_frame_preview_inner_returns_segment_frame_bytes_when_exact_png_and_video_are_missing() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-segment-fallback");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let workspace_dir = dir
                .path()
                .join("2026/04/12/.session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
            let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
            let sibling_bytes = b"segment-frame-preview-bytes";
            fs::write(&sibling_frame_path, sibling_bytes)
                .expect("sibling frame preview file should be written");

            let target_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    target_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.500Z",
                ))
                .await
                .expect("target frame should be inserted");

            infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    sibling_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.000Z",
                ))
                .await
                .expect("sibling frame should be inserted");

            let preview = get_frame_preview_inner(&infra, target_frame.id)
                .await
                .expect("preview should load")
                .expect("preview should exist");

            assert_eq!(preview.mime_type, "image/png");
            assert_eq!(
                preview.source_kind,
                FramePreviewSourceKindDto::SegmentFrameFallback
            );
            assert_eq!(preview.data_base64, BASE64_STANDARD.encode(sibling_bytes));
        });
    }

    #[test]
    fn get_frame_preview_inner_does_not_use_segment_frame_fallback_when_visible_video_exists() {
        run_async_test(async {
            let dir = TestDir::new("frame-preview-video-preferred");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");
            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames directory should be created");

            let target_frame_path = frames_dir.join("frame-1744459201500-1.png");
            let sibling_frame_path = frames_dir.join("frame-1744459201000-0.png");
            let video_path = segment_dir.join("session-preview-segment-0001.mov");
            let sibling_bytes = b"segment-frame-preview-bytes";
            fs::write(&sibling_frame_path, sibling_bytes)
                .expect("sibling frame preview file should be written");
            fs::write(&video_path, b"not-a-real-video")
                .expect("visible segment video should be written");

            let target_frame = infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    target_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.500Z",
                ))
                .await
                .expect("target frame should be inserted");

            infra
                .insert_frame(&::app_infra::NewFrame::new(
                    "session-preview",
                    sibling_frame_path.to_string_lossy().to_string(),
                    "2025-04-12T10:00:01.000Z",
                ))
                .await
                .expect("sibling frame should be inserted");

            let error = get_frame_preview_inner(&infra, target_frame.id)
                .await
                .expect_err("visible video should be attempted before sibling PNG fallback");

            let error_message = error.to_string();
            assert!(
                error_message.contains(&video_path.display().to_string())
                    || error_message.contains("video frame preview fallback is only supported on macOS"),
                "unexpected error: {error_message}"
            );
            assert!(!error_message.contains("segment video does not exist"));
            assert_ne!(BASE64_STANDARD.encode(sibling_bytes), error_message);
        });
    }

    #[test]
    fn frame_preview_cache_returns_entries_within_ttl() {
        let mut cache = FramePreviewCache::default();
        let now = Instant::now();
        let preview = FramePreviewDto {
            mime_type: "image/png".to_string(),
            data_base64: "abc".to_string(),
            source_kind: FramePreviewSourceKindDto::OriginalFrame,
        };

        cache.insert(42, preview.clone(), Duration::from_secs(60), now);

        assert_eq!(cache.get(42, Duration::from_secs(60), now), Some(preview));
    }

    #[test]
    fn frame_preview_cache_evicts_expired_entries() {
        let mut cache = FramePreviewCache::default();
        let now = Instant::now();

        cache.insert(
            42,
            FramePreviewDto {
                mime_type: "image/png".to_string(),
                data_base64: "abc".to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
            },
            Duration::from_secs(1),
            now,
        );

        assert_eq!(
            cache.get(42, Duration::from_secs(1), now + Duration::from_secs(1)),
            None
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn frame_preview_cache_clear_removes_existing_entries() {
        let mut cache = FramePreviewCache::default();
        cache.insert(
            42,
            FramePreviewDto {
                mime_type: "image/png".to_string(),
                data_base64: "abc".to_string(),
                source_kind: FramePreviewSourceKindDto::OriginalFrame,
            },
            Duration::from_secs(60),
            Instant::now(),
        );

        cache.clear();

        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn frame_preview_cache_evicts_oldest_entries_when_max_size_is_reached() {
        let mut cache = FramePreviewCache::default();
        let now = Instant::now();
        let ttl = Duration::from_secs(60);

        for frame_id in 0..=FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64 {
            cache.insert(
                frame_id,
                FramePreviewDto {
                    mime_type: "image/png".to_string(),
                    data_base64: frame_id.to_string(),
                    source_kind: FramePreviewSourceKindDto::OriginalFrame,
                },
                ttl,
                now + Duration::from_millis(frame_id as u64),
            );
        }

        assert_eq!(cache.len(), FRAME_PREVIEW_CACHE_MAX_ENTRIES);
        assert_eq!(cache.get(0, ttl, now + Duration::from_secs(1)), None);
        assert!(cache
            .get(
                FRAME_PREVIEW_CACHE_MAX_ENTRIES as i64,
                ttl,
                now + Duration::from_secs(1)
            )
            .is_some());
    }

    #[test]
    fn persist_screen_frame_artifact_maps_metadata_for_dedupable_ocr() {
        run_async_test(async {
            let dir = TestDir::new("screen-frame-artifact");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = persist_screen_frame_artifact(
                &infra,
                "session-artifact",
                ScreenFrameArtifact {
                    file_path: "/tmp/frame-artifact.png".to_string(),
                    captured_at_unix_ms: 1_744_539_600_123,
                    width: Some(1440),
                    height: Some(900),
                    content_fingerprint: Some(0xfeed_beef),
                },
            )
            .await
            .expect("artifact should persist");

            assert_eq!(persisted.frame.session_id, "session-artifact");
            assert_eq!(persisted.frame.file_path, "/tmp/frame-artifact.png");
            assert_eq!(persisted.frame.width, Some(1440));
            assert_eq!(persisted.frame.height, Some(900));
            assert_eq!(
                persisted.frame.content_fingerprint.as_deref(),
                Some("00000000feedbeef")
            );
            assert_eq!(
                persisted.job.as_ref().map(|job| job.processor.as_str()),
                Some("ocr")
            );

            let batches = infra
                .list_frame_batches(Some("session-artifact"))
                .await
                .expect("frame batches should list");
            assert_eq!(batches.len(), 1);
            assert_eq!(batches[0].status, ::app_infra::FrameBatchStatus::Open);
            assert_eq!(batches[0].frame_count, 1);
        });
    }

    #[test]
    fn process_pending_jobs_once_claims_and_processes_queued_work() {
        run_async_test(async {
            let dir = TestDir::new("processing-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            let persisted = infra
                .insert_frame_and_enqueue_processing_job(
                    &::app_infra::NewFrame::new(
                        "session-worker",
                        "/tmp/frame-worker.png",
                        "2026-04-12T10:00:00Z",
                    ),
                    "missing-processor",
                    None,
                )
                .await
                .expect("frame and job should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let job = infra
                .get_processing_job(persisted.job.id)
                .await
                .expect("job should be readable")
                .expect("job should exist");
            assert_eq!(job.status, ::app_infra::ProcessingJobStatus::Failed);
            assert_eq!(job.attempt_count, 1);
            assert_eq!(
                job.last_error.as_deref(),
                Some("processor backend is not registered for 'missing-processor'")
            );
        });
    }

    #[test]
    fn process_pending_jobs_once_can_process_frame_batch_finalize_jobs() {
        run_async_test(async {
            let dir = TestDir::new("frame-batch-worker-once");
            let infra = ::app_infra::AppInfra::initialize(dir.path())
                .await
                .expect("app infra should initialize");

            infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0001/frames/frame-1.png",
                        "2026-04-12T10:01:00Z",
                    ),
                    None,
                )
                .await
                .expect("first frame should persist");
            infra
                .insert_frame_into_batch_and_maybe_enqueue_ocr_job(
                    &::app_infra::NewFrame::new(
                        "session-batch-worker",
                        "/tmp/session-batch-worker-segment-0002/frames/frame-2.png",
                        "2026-04-12T10:11:00Z",
                    ),
                    None,
                )
                .await
                .expect("second frame should persist");

            let processed = process_pending_jobs_once(&infra)
                .await
                .expect("worker iteration should succeed");
            assert_eq!(processed, Some(()));

            let batches = infra
                .list_frame_batches(Some("session-batch-worker"))
                .await
                .expect("frame batches should list");

            // The first batch's OCR job is processed (fails: no backend) in the
            // same iteration, making the finalize job claimable.  Finalization
            // completes successfully (frame cleanup skips missing files), so the
            // batch ends up Completed — proving the worker now services finalize
            // jobs alongside processing jobs instead of starving them.
            assert!(batches
                .iter()
                .any(|batch| batch.status == ::app_infra::FrameBatchStatus::Completed));
        });
    }
}
