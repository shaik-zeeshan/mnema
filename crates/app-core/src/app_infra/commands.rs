//! The timeline, audio, search and processing-job commands.

use super::*;

pub async fn list_app_jobs(state: &AppInfraState) -> Result<Vec<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_jobs()
        .await
        .map(|jobs| jobs.into_iter().map(AppJobDto::from).collect())
        .map_err(|error| format!("failed to list app jobs: {error}"))
}

pub async fn get_app_job(
    request: GetAppJobRequest,
    state: &AppInfraState,
) -> Result<Option<AppJobDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_job(request.job_id)
        .await
        .map(|job| job.map(AppJobDto::from))
        .map_err(|error| format!("failed to get app job {}: {error}", request.job_id))
}

pub async fn debug_insert_frame_and_enqueue_processing_job(
    request: DebugInsertFrameAndEnqueueProcessingJobRequest,
    state: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    debug_insert_frame_and_enqueue_processing_job_inner(&infra, request, Some(&settings))
        .await
        .map_err(|error| {
            format!("failed to debug-insert frame and enqueue processing job: {error}")
        })
}

pub async fn debug_insert_frame_and_enqueue_ocr(
    request: DebugInsertFrameAndEnqueueOcrRequest,
    state: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<FrameProcessingJobDto, String> {
    let infra = Arc::clone(&*state);

    debug_insert_frame_and_enqueue_processing_job_inner(&infra, request.into(), Some(&settings))
        .await
        .map_err(|error| format!("failed to debug-insert frame and enqueue ocr job: {error}"))
}

pub async fn reprocess_captured_frame_ocr(
    request: ReprocessCapturedFrameOcrRequest,
    state: &AppInfraState,
    settings: &crate::recording_settings::RecordingSettingsState,
) -> Result<CapturedFrameReprocessingResultDto, String> {
    let infra = Arc::clone(&*state);

    reprocess_captured_frame_ocr_inner(&infra, request.clone(), &settings)
        .await
        .map_err(|error| {
            format!(
                "failed to reprocess captured frame OCR for frame {}: {error}",
                request.frame_id
            )
        })
}

pub async fn reprocess_audio_segment_transcription(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: &AppInfraState,
    core: &AppCore,
) -> Result<AudioSegmentTranscriptionReprocessingResultDto, String> {
    let infra = Arc::clone(&*state);

    reprocess_audio_segment_transcription_inner(&infra, request.clone(), core)
        .await
        .map_err(|error| {
            format!(
                "failed to reprocess audio segment transcription for segment {}: {error}",
                request.audio_segment_id
            )
        })
}

pub async fn reprocess_audio_segment_speaker_analysis(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: &AppInfraState,
    core: &AppCore,
) -> Result<AudioSegmentSpeakerAnalysisReprocessingResultDto, String> {
    let infra = Arc::clone(&*state);

    reprocess_audio_segment_speaker_analysis_inner(&infra, request.clone(), core)
        .await
        .map_err(|error| {
            format!(
                "failed to reprocess audio segment speaker analysis for segment {}: {error}",
                request.audio_segment_id
            )
        })
}

pub async fn reprocess_system_audio_speech_activity(
    request: ReprocessAudioSegmentTranscriptionRequest,
    state: &AppInfraState,
    core: &AppCore,
) -> Result<SystemAudioSpeechActivityReprocessingResultDto, String> {
    let infra = Arc::clone(&*state);

    reprocess_system_audio_speech_activity_inner(&infra, request.clone(), core)
        .await
        .map_err(|error| {
            format!(
                "failed to reprocess system-audio speech activity for segment {}: {error}",
                request.audio_segment_id
            )
        })
}

pub async fn list_frames(
    request: Option<ListFramesRequest>,
    state: &AppInfraState,
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

pub async fn list_frame_summaries_in_range(
    request: FrameCapturedAtRangeRequest,
    state: &AppInfraState,
) -> Result<Vec<FrameSummaryDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_frame_summaries_in_range(
            &request.captured_at_start,
            &request.captured_at_end,
            request.exclude_lock_screen,
        )
        .await
        .map(|frames| frames.into_iter().map(FrameSummaryDto::from).collect())
        .map_err(|error| format!("failed to list frame summaries in range: {error}"))
}

pub async fn get_latest_frame_in_range(
    request: FrameCapturedAtRangeRequest,
    state: &AppInfraState,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_latest_frame_in_range(&request.captured_at_start, &request.captured_at_end)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get latest frame in range: {error}"))
}

pub async fn list_audio_segments(
    request: ListAudioSegmentsRequest,
    state: &AppInfraState,
) -> Result<Vec<AudioSegmentDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .list_audio_segments_overlapping_range(
            &request.captured_at_start,
            &request.captured_at_end,
            None,
            None,
        )
        .await
        // The persisted `ended_at` is already media-accurate: the row is only
        // written once the file is complete, from the real AVAsset duration
        // (`audio_segment_window_for_file`). Re-probing every row on every list
        // call opened an `AVURLAsset` per segment on a ~1 Hz IPC path and leaked
        // the file `NSURL` + its path `CFString` each time (~200 B a piece,
        // 414 MB over a day) — AVFoundation never released them, autorelease
        // pool or not.
        .map(|segments| segments.into_iter().map(AudioSegmentDto::from).collect())
        .map_err(|error| format!("failed to list audio segments: {error}"))
}

pub async fn get_audio_segment_media(
    request: GetAudioSegmentMediaRequest,
    state: &AppInfraState,
) -> Result<AudioSegmentMediaDto, String> {
    let infra = Arc::clone(&*state);
    get_audio_segment_media_inner(&infra, request.audio_segment_id)
        .await
        .map_err(|error| {
            format!(
                "failed to get audio segment media {}: {error}",
                request.audio_segment_id
            )
        })?
        .ok_or_else(|| format!("audio segment {} not found", request.audio_segment_id))
}

/// Amplitude peaks for the transcription reader's waveform scrubber.
///
/// Never errors: any failure (unknown segment, missing file, undecodable
/// audio) returns an empty `Vec` so a decode hiccup degrades the scrubber to a
/// plain bar instead of breaking the reader. A *failure* still leaves a log
/// breadcrumb on the way out — otherwise a `SQLITE_BUSY` lookup or a panicking
/// FFI decode is byte-identical to a genuinely silent segment and undiagnosable.
pub async fn get_audio_segment_waveform_peaks(
    request: GetAudioSegmentWaveformPeaksRequest,
    state: &AppInfraState,
) -> Result<Vec<f32>, String> {
    let infra = Arc::clone(&*state);
    let lookup = infra.get_audio_segment(request.audio_segment_id).await;
    let file_path = match waveform_peaks_segment_path(request.audio_segment_id, lookup) {
        Ok(Some(file_path)) => file_path,
        Ok(None) => return Ok(Vec::new()),
        Err(error) => {
            crate::debug_log::log_error(error);
            return Ok(Vec::new());
        }
    };
    let bucket_count = request.bucket_count;
    let task = tokio::task::spawn_blocking(move || {
        audio_transcription::audio_waveform_peaks(&file_path, bucket_count)
    });
    Ok(join_waveform_peaks(request.audio_segment_id, task)
        .await
        .unwrap_or_else(|error| {
            crate::debug_log::log_error(error);
            Vec::new()
        }))
}

/// `Ok(None)` = "this segment has no waveform" (unknown id). A lookup *failure*
/// is not the same thing and must not be silently reported as one.
pub(super) fn waveform_peaks_segment_path(
    audio_segment_id: i64,
    lookup: ::app_infra::Result<Option<::app_infra::AudioSegment>>,
) -> Result<Option<PathBuf>, String> {
    match lookup {
        Ok(Some(segment)) => Ok(Some(PathBuf::from(&segment.file_path))),
        Ok(None) => Ok(None),
        Err(error) => Err(format!(
            "failed to get audio segment {audio_segment_id}: {error}"
        )),
    }
}

/// Joins the blocking decode. A panic inside the AVFoundation/cidre FFI still
/// degrades to an empty waveform, but must not vanish without a breadcrumb.
pub(super) async fn join_waveform_peaks(
    audio_segment_id: i64,
    task: JoinHandle<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    task.await.map_err(|error| {
        format!("waveform peak decode for audio segment {audio_segment_id} failed: {error}")
    })
}

pub async fn get_audio_segment(
    request: GetAudioSegmentRequest,
    state: &AppInfraState,
) -> Result<Option<AudioSegmentDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_audio_segment(request.audio_segment_id)
        .await
        .map(|segment| segment.map(AudioSegmentDto::from))
        .map_err(|error| {
            format!(
                "failed to get audio segment {}: {error}",
                request.audio_segment_id
            )
        })
}

pub async fn get_frame(
    request: GetFrameRequest,
    state: &AppInfraState,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| format!("failed to get frame {}: {error}", request.frame_id))
}

/// Whether a captured `browser_url` may be handed to the OS opener: it must parse
/// and carry an `http`/`https` scheme. This is the security gate — a captured URL
/// of `javascript:`, `file:`, `data:`, `mnema:`, etc. must NEVER reach the opener.
/// Extracted as a pure helper so the gate is unit-testable without a live opener.
pub(super) fn captured_url_is_openable(raw_url: &str) -> bool {
    url::Url::parse(raw_url)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// App-mediated open of a frame's RAW captured `browser_url` in the default
/// browser. The trusted local frontend opens captures by frame id (it never uses
/// opaque broker ids). Loads the frame's raw URL, gates it to `http`/`https`, and
/// hands it to the host's opener (NOT a bare `Command::new` — the packaged
/// app has no Homebrew PATH). The raw URL is local-only: it is never returned,
/// logged, or echoed in an error.
///
/// Returns `true` if a frame was found, its captured URL was http(s), and the
/// opener accepted it; `false` if the frame has no http(s) `browser_url` to open.
pub async fn open_captured_url(
    frame_id: i64,
    core: &AppCore,
    state: &AppInfraState,
) -> Result<bool, String> {
    let infra = Arc::clone(&*state);
    let raw_url = infra
        .get_frame(frame_id)
        .await
        .map_err(|error| format!("failed to get frame {frame_id}: {error}"))?
        .and_then(|frame| frame.metadata_snapshot)
        .and_then(|snapshot| snapshot.browser_url);
    let Some(raw_url) = raw_url else {
        return Ok(false);
    };
    // Scheme gate: only ever hand http(s) to the OS opener. The raw URL never
    // crosses into the error path or a log line.
    if !captured_url_is_openable(&raw_url) {
        return Ok(false);
    }
    core.host()
        .open_url(&raw_url)
        .map_err(|error| format!("failed to open captured url for frame {frame_id}: {error}"))?;
    Ok(true)
}

pub async fn get_nearest_earlier_equivalent_frame(
    request: GetNearestEarlierEquivalentFrameRequest,
    state: &AppInfraState,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_nearest_earlier_equivalent_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| {
            format!(
                "failed to resolve nearest earlier equivalent frame for frame {}: {error}",
                request.frame_id
            )
        })
}

pub async fn get_earliest_earlier_equivalent_frame(
    request: GetEarliestEarlierEquivalentFrameRequest,
    state: &AppInfraState,
) -> Result<Option<FrameDto>, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_earliest_earlier_equivalent_frame(request.frame_id)
        .await
        .map(|frame| frame.map(FrameDto::from))
        .map_err(|error| {
            format!(
                "failed to resolve earliest earlier equivalent frame for frame {}: {error}",
                request.frame_id
            )
        })
}

pub async fn get_timeline_window_around_frame(
    request: GetTimelineWindowAroundFrameRequest,
    state: &AppInfraState,
) -> Result<FocusedFrameWindowDto, String> {
    let infra = Arc::clone(&*state);
    infra
        .get_timeline_window_around_frame(
            request.frame_id,
            request.newer_limit,
            request.older_limit,
        )
        .await
        .map(FocusedFrameWindowDto::from)
        .map_err(|error| {
            format!(
                "failed to get timeline window around frame {}: {error}",
                request.frame_id
            )
        })
}

pub async fn search_capture(
    request: SearchCaptureRequest,
    core: &AppCore,
    state: &AppInfraState,
    query_embedder: &crate::semantic_search_query::SemanticQueryEmbedderState,
) -> Result<SearchCaptureResponseDto, String> {
    let infra = Arc::clone(&*state);

    // **Hybrid Search**: embed the query string into a **Semantic Search Vector**
    // so app-infra can fuse the `vec0` KNN with FTS5 by RRF. `None` when no
    // **Semantic Search Model** is installed (default/Anthropic-only) — search
    // then degrades to keyword-only with no regression. Never errors: a failed or
    // absent embed just leaves `query_embedding = None`.
    //
    // The embed runs over the operator-stripped residual — exactly what FTS ranks
    // on — so `app:`/`before:`/`source:` operators and quoted phrases never pollute
    // the meaning vector. The full `request.query` still drives FTS below; only the
    // EMBED input is the residual. An all-operators query yields an empty residual,
    // which `embed_search_query` empty-guards to `None` (keyword-only).
    let residual_query = ::app_infra::semantic_search_residual_query(&request.query);
    let query_embedding =
        crate::semantic_search_query::embed_search_query(core, &query_embedder, &residual_query)
            .await;

    let query = request.query.clone();
    let mut response = infra
        .search_capture(::app_infra::SearchCaptureRequest {
            query: request.query,
            frame_limit: request.frame_limit,
            frame_offset: request.frame_offset,
            audio_limit: request.audio_limit,
            audio_offset: request.audio_offset,
            snapshot_document_id: request.snapshot_document_id,
            refinements: request.refinements,
            query_embedding,
        })
        .await
        .map(SearchCaptureResponseDto::from)
        .map_err(|error| format!("failed to search captured content: {error}"))?;
    // ponytail: one indexed lookup per frame on the page (≤ 24); batch if it shows.
    // A missing outline never fails the search.
    for frame in &mut response.frames {
        frame.match_boxes = infra
            .search()
            .frame_match_boxes(
                frame.representative_frame.id,
                &query,
                frame.found_by_meaning,
            )
            .await
            .unwrap_or_default();
    }
    Ok(response)
}

pub async fn list_processing_jobs(
    request: ListProcessingJobsRequest,
    state: &AppInfraState,
) -> Result<Vec<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    let subject = processing_subject(request.subject_type, request.subject_id);

    let locked_model_keys = infra
        .list_locked_processing_model_keys()
        .await
        .map_err(|error| format!("failed to list locked processing models: {error}"))?;

    infra
        .list_processing_jobs_for_subject(&subject)
        .await
        .map(|jobs| {
            jobs.into_iter()
                .map(|job| ProcessingJobDto::from_job(job, &locked_model_keys))
                .collect()
        })
        .map_err(|error| format!("failed to list processing jobs: {error}"))
}

pub async fn get_processing_job(
    request: GetProcessingJobRequest,
    state: &AppInfraState,
) -> Result<Option<ProcessingJobDto>, String> {
    let infra = Arc::clone(&*state);
    let locked_model_keys = infra
        .list_locked_processing_model_keys()
        .await
        .map_err(|error| format!("failed to list locked processing models: {error}"))?;
    infra
        .get_processing_job(request.job_id)
        .await
        .map(|job| job.map(|job| ProcessingJobDto::from_job(job, &locked_model_keys)))
        .map_err(|error| format!("failed to get processing job {}: {error}", request.job_id))
}

/// Developer window "Retry now": skip a waiting job's backoff. Failed jobs use the
/// `reprocess_*` commands instead.
pub async fn retry_processing_job_now(
    job_id: i64,
    state: &AppInfraState,
) -> Result<ProcessingJobDto, String> {
    let infra = Arc::clone(&*state);
    let cleared = infra
        .clear_processing_job_backoff(job_id)
        .await
        .map_err(|error| format!("failed to retry processing job {job_id}: {error}"))?;
    if !cleared {
        return Err(
            "Only a waiting job can be retried now. Use Reprocess for a failed job.".to_string(),
        );
    }
    crate::debug_log::log_info(format!(
        "processing job {job_id} retry requested from developer window"
    ));
    let locked_model_keys = infra
        .list_locked_processing_model_keys()
        .await
        .map_err(|error| format!("failed to list locked processing models: {error}"))?;
    infra
        .get_processing_job(job_id)
        .await
        .map_err(|error| format!("failed to get processing job {job_id}: {error}"))?
        .map(|job| ProcessingJobDto::from_job(job, &locked_model_keys))
        .ok_or_else(|| format!("processing job {job_id} not found"))
}

pub async fn get_processing_result(
    request: GetProcessingResultRequest,
    state: &AppInfraState,
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

pub async fn list_processing_results(
    request: ListProcessingResultsRequest,
    state: &AppInfraState,
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
    use super::*;

    // The `open_captured_url` scheme gate is the security boundary between a
    // captured `browser_url` and the OS opener. Only `http`/`https` may pass; a
    // captured `javascript:`/`file:`/`data:`/`mnema:` URL (or an unparseable
    // string) must be rejected so it can never reach the opener. Pinning the pure
    // gate guards that boundary without a live Tauri opener.
    #[test]
    fn captured_url_open_gate_admits_only_http_and_https() {
        assert!(captured_url_is_openable("https://example.com/x"));
        assert!(captured_url_is_openable("http://example.com/x"));
        for rejected in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "mnema://open/abc",
            "ftp://example.com/x",
            "not a url",
            "",
        ] {
            assert!(
                !captured_url_is_openable(rejected),
                "non-http(s) captured url must be rejected by the open gate: {rejected:?}"
            );
        }
    }

    // The waveform scrubber degrades to a plain bar on an empty peak list, so
    // "empty" is only allowed to mean "this segment genuinely has no waveform".
    // A failed lookup (SQLITE_BUSY under a lock storm, pool starvation) is a
    // failure: the command still degrades, but it must not be indistinguishable
    // from success on the way there.
    #[test]
    fn a_failed_segment_lookup_is_not_an_empty_waveform() {
        let failure = waveform_peaks_segment_path(
            41,
            Err(::app_infra::AppInfraError::Sqlx(sqlx::Error::PoolTimedOut)),
        );

        let error = failure
            .expect_err("a database failure must not be reported as a segment with no waveform");
        assert!(
            error.contains("41"),
            "the error should name the audio segment: {error}"
        );
    }

    #[test]
    fn an_unknown_segment_is_still_an_empty_waveform() {
        assert_eq!(
            waveform_peaks_segment_path(41, Ok(None)).expect("an unknown segment is not an error"),
            None
        );
    }

    // A panic inside the AVFoundation/cidre decode still degrades to a plain
    // scrub bar, but it must leave a breadcrumb: an FFI crash that produces the
    // exact same value as "this file is silent" is undiagnosable.
    #[test]
    fn a_panicking_waveform_decode_is_reported_not_defaulted_away() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime starts");
        let outcome = runtime.block_on(join_waveform_peaks(
            41,
            runtime.spawn_blocking(|| -> Vec<f32> { panic!("cidre decode blew up") }),
        ));

        let error = outcome
            .expect_err("a panicking decode must be reported, not returned as an empty waveform");
        assert!(
            error.contains("41"),
            "the failure should name the audio segment: {error}"
        );
    }

    #[test]
    fn a_successful_waveform_decode_passes_its_peaks_through() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime starts");
        let outcome = runtime.block_on(join_waveform_peaks(
            41,
            runtime.spawn_blocking(|| vec![0.25_f32, 1.0]),
        ));

        assert_eq!(
            outcome.expect("a successful decode should not be an error"),
            vec![0.25, 1.0]
        );
    }
}
