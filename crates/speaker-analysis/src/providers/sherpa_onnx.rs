use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

use async_trait::async_trait;
use serde_json::json;

#[cfg(target_os = "macos")]
use crate::macos_audio_decode::{
    decode_audio_to_mono_with_avassetreader_fallback, DecodedAudio,
};
#[cfg(any(test, target_os = "macos"))]
use crate::macos_audio_decode::resample_linear;
use crate::{
    model_install_dir, SpeakerAnalysisError, SpeakerAnalysisMetadata, SpeakerAnalysisOutput,
    SpeakerAnalysisProvider, SpeakerAnalysisRequest, SpeakerAnalysisResult, SpeakerCluster,
    SpeakerRecognitionSuggestion, SpeakerTurn, DEFAULT_CLUSTERING_THRESHOLD,
    DEFAULT_SHERPA_ONNX_MODEL_ID, SHERPA_ONNX_PROVIDER_ID,
};

const SAMPLE_RATE_HZ: u32 = 16_000;
const CLUSTERING_THRESHOLD_OPTION: &str = "clusteringThreshold";
const CROSS_CHUNK_THRESHOLD_OPTION: &str = "crossChunkThreshold";
const NUM_CLUSTERS_OPTION: &str = "numClusters";
const MIN_DURATION_ON_OPTION: &str = "minDurationOn";
const MIN_DURATION_OFF_OPTION: &str = "minDurationOff";
const MIN_DIARIZATION_AUDIO_MS: u64 = 1_000;
const MIN_DIARIZATION_PEAK: f32 = 1.0e-5;
const SAFE_SINGLE_CHUNK_DIARIZATION_MS: u64 = 10_000;
const SAFE_CHUNK_OVERLAP_MS: u64 = 1_000;
const MERGE_ADJACENT_TURN_GAP_MS: u64 = 250;
/// Maximum audio (in samples) fed to the speaker-embedding extractor in a single
/// call. The NeMo TitaNet ONNX models have a fixed internal dimension (~12288
/// frames, ≈123s) and throw an uncatchable C++ exception that aborts the process
/// when a speaker's concatenated audio exceeds it (see the global representative
/// embedding loop). 60s is well under that limit and is far more than enough for
/// a stable speaker embedding, so we cap the concatenation defensively. This only
/// bounds the stored representative embedding — turns and cluster assignments are
/// already finalized — so it does not change diarization labeling.
const MAX_EMBEDDING_AUDIO_SAMPLES: usize = 60 * SAMPLE_RATE_HZ as usize;
const MIN_RECOGNITION_SUGGESTION_SCORE: f32 = 0.60;
const HIGH_RECOGNITION_SUGGESTION_SCORE: f32 = 0.72;
const PERSON_AMBIGUITY_MARGIN: f32 = 0.05;
const REJECTED_PERSON_SIMILARITY_THRESHOLD: f32 = 0.80;
static SHERPA_DIARIZATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct SherpaOnnxSpeakerAnalysisProvider {
    models_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
struct SherpaModelSelection {
    model_id: String,
    segmentation_model_path: PathBuf,
    embedding_model_path: PathBuf,
    /// Per-model fast-clustering similarity threshold (accuracy #3). The
    /// request-option override still wins over this in `diarization_config`.
    clustering_threshold: f32,
    /// Per-model cross-chunk cluster similarity threshold used by the
    /// order-independent agglomerative pass (`agglomerate_local_clusters`)
    /// when stitching safe-chunked clusters.
    cross_chunk_threshold: f32,
    /// Per-model minimum speaker-turn duration in milliseconds (accuracy #2);
    /// turns shorter than this are skipped when forming per-chunk embeddings.
    min_turn_ms: u64,
}

impl SherpaOnnxSpeakerAnalysisProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }
}

#[cfg(feature = "sherpa-onnx")]
#[derive(Debug, Clone)]
struct LocalSpeakerCluster {
    key: usize,
    ranges: Vec<(usize, usize)>,
    embedding: Vec<f32>,
    total_samples: usize,
}

#[cfg(feature = "sherpa-onnx")]
#[derive(Debug, Clone)]
struct PendingSpeakerTurn {
    local_cluster_key: usize,
    start_ms: u64,
    end_ms: u64,
}

#[cfg(feature = "sherpa-onnx")]
#[derive(Debug, Clone)]
struct GlobalSpeakerClusterState {
    id: usize,
    ranges: Vec<(usize, usize)>,
    representative_embedding: Vec<f32>,
}

#[async_trait]
impl SpeakerAnalysisProvider for SherpaOnnxSpeakerAnalysisProvider {
    fn provider(&self) -> &'static str {
        SHERPA_ONNX_PROVIDER_ID
    }

    async fn analyze(
        &self,
        request: SpeakerAnalysisRequest,
    ) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
        let models_dir = self.models_dir.clone();
        tokio_spawn_blocking(move || run_sherpa_blocking(request, &models_dir)).await
    }
}

#[cfg(feature = "sherpa-onnx")]
pub fn analyze_sherpa_request_blocking(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    run_sherpa_blocking(request, models_dir)
}

#[cfg(feature = "sherpa-onnx")]
pub fn analyze_sherpa_samples_blocking(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
    samples: Vec<f32>,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    run_sherpa_on_samples(request, models_dir, samples)
}

#[cfg(feature = "sherpa-onnx")]
pub fn decode_sherpa_audio_to_mono_16khz(path: &Path) -> SpeakerAnalysisResult<Vec<f32>> {
    decode_audio_to_mono_16khz(path)
}

async fn tokio_spawn_blocking<F>(task: F) -> SpeakerAnalysisResult<SpeakerAnalysisOutput>
where
    F: FnOnce() -> SpeakerAnalysisResult<SpeakerAnalysisOutput> + Send + 'static,
{
    #[cfg(feature = "sherpa-onnx")]
    {
        tokio::task::spawn_blocking(task).await.map_err(|error| {
            SpeakerAnalysisError::Analysis(format!("sherpa-onnx worker failed to join: {error}"))
        })?
    }

    #[cfg(not(feature = "sherpa-onnx"))]
    {
        let _ = task;
        Err(SpeakerAnalysisError::ProviderUnavailable(
            "sherpa-onnx runtime is not enabled in this build".to_string(),
        ))
    }
}

#[cfg(feature = "sherpa-onnx")]
fn run_sherpa_blocking(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    let started_at = Instant::now();
    let samples = decode_audio_to_mono_16khz(&request.audio_path)?;
    let mut output = run_sherpa_on_samples(request, models_dir, samples)?;
    output.metadata.provenance.insert(
        "elapsedMs".to_string(),
        json!(started_at.elapsed().as_millis() as u64),
    );
    Ok(output)
}

#[cfg(feature = "sherpa-onnx")]
fn run_sherpa_on_samples(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
    samples: Vec<f32>,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    let selection = resolve_model_selection(&request, models_dir)?;
    if !request.audio_path.is_file() {
        return Err(SpeakerAnalysisError::InvalidRequest(format!(
            "audio file does not exist: {}",
            request.audio_path.display()
        )));
    }
    if !selection.segmentation_model_path.is_file() {
        return Err(SpeakerAnalysisError::MissingModel {
            model_kind: "segmentation".to_string(),
            path: selection.segmentation_model_path.clone(),
        });
    }
    if !selection.embedding_model_path.is_file() {
        return Err(SpeakerAnalysisError::MissingModel {
            model_kind: "embedding".to_string(),
            path: selection.embedding_model_path.clone(),
        });
    }

    validate_decoded_samples(&samples)?;
    let duration_ms = samples.len() as u64 * 1000 / SAMPLE_RATE_HZ as u64;
    let audio_peak = audio_peak(&samples);
    let mut output = speaker_output_for_request(&request, &selection, duration_ms, audio_peak);
    if let Some(skip_reason) = speaker_skip_reason(audio_peak, duration_ms) {
        output
            .metadata
            .provenance
            .insert("skipReason".to_string(), json!(skip_reason));
        finalize_provenance_counts(&mut output);
        return Ok(output);
    }

    let config = diarization_config(&request, &selection);
    let _guard = SHERPA_DIARIZATION_LOCK
        .lock()
        .map_err(|_| SpeakerAnalysisError::Runtime {
            stage: "create_diarizer".to_string(),
            message: "sherpa-onnx diarization lock was poisoned".to_string(),
        })?;
    let diarizer =
        sherpa_onnx_runtime::OfflineSpeakerDiarization::create(&config).ok_or_else(|| {
            SpeakerAnalysisError::Runtime {
                stage: "create_diarizer".to_string(),
                message: "failed to create sherpa-onnx speaker diarizer".to_string(),
            }
        })?;
    let extractor = sherpa_onnx_runtime::SpeakerEmbeddingExtractor::create(
        &sherpa_onnx_runtime::SpeakerEmbeddingExtractorConfig {
            model: Some(selection.embedding_model_path.display().to_string()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
    )
    .ok_or_else(|| SpeakerAnalysisError::Runtime {
        stage: "create_embedding_extractor".to_string(),
        message: "failed to create sherpa-onnx speaker embedding extractor".to_string(),
    })?;

    if samples.len() > safe_single_chunk_sample_limit() {
        output
            .metadata
            .provenance
            .insert("chunkingMode".to_string(), json!("safe_chunked"));
        output.metadata.provenance.insert(
            "safeChunkDurationMs".to_string(),
            json!(SAFE_SINGLE_CHUNK_DIARIZATION_MS),
        );
        return analyze_long_audio_with_safe_chunking(
            &request, &samples, &selection, &diarizer, &extractor, output,
        );
    }

    let result = diarizer
        .process(&samples)
        .ok_or_else(|| SpeakerAnalysisError::Runtime {
            stage: "diarize_single_chunk".to_string(),
            message: "sherpa-onnx diarization returned no result".to_string(),
        })?;
    output
        .metadata
        .provenance
        .insert("chunkCount".to_string(), json!(1));
    let segments = result.sort_by_start_time();

    let mut speaker_segments = BTreeMap::<i32, Vec<(usize, usize)>>::new();
    for segment in segments {
        let start_ms = seconds_to_ms(segment.start);
        let end_ms = seconds_to_ms(segment.end);
        output.turns.push(SpeakerTurn {
            provider_cluster_id: provider_cluster_id(segment.speaker),
            start_ms,
            end_ms,
            transcript_text: None,
            overlaps: false,
        });
        let start = ms_to_sample_index(start_ms, samples.len());
        let end = ms_to_sample_index(end_ms, samples.len());
        if end > start {
            speaker_segments
                .entry(segment.speaker)
                .or_default()
                .push((start, end));
        }
    }

    for (speaker, ranges) in speaker_segments {
        let cluster_samples = concatenate_ranges(&samples, &ranges);
        let embedding = compute_embedding(&extractor, &cluster_samples)?;
        let suggestion = if request.recognize_people {
            best_enrollment_match(&request, &embedding, &selection.model_id)
        } else {
            None
        };
        output.clusters.push(SpeakerCluster {
            provider_cluster_id: provider_cluster_id(speaker),
            stable_label: format!("Unknown Speaker {}", speaker + 1),
            embedding: f32_embedding_to_le_bytes(&embedding),
            embedding_model_id: selection.model_id.clone(),
            suggestion,
        });
    }
    output.turns = mark_overlapping_turns(output.turns);
    finalize_provenance_counts(&mut output);

    Ok(output)
}

#[cfg(feature = "sherpa-onnx")]
fn analyze_long_audio_with_safe_chunking(
    request: &SpeakerAnalysisRequest,
    samples: &[f32],
    selection: &SherpaModelSelection,
    diarizer: &sherpa_onnx_runtime::OfflineSpeakerDiarization,
    extractor: &sherpa_onnx_runtime::SpeakerEmbeddingExtractor,
    mut output: SpeakerAnalysisOutput,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    let chunk_len = safe_single_chunk_sample_limit();
    let mut local_clusters = Vec::new();
    let mut pending_turns = Vec::new();
    let mut next_local_cluster_key = 0usize;
    let mut chunk_count = 0usize;
    let mut warning_reasons = Vec::<String>::new();

    let step_len = chunk_len.saturating_sub(overlap_sample_limit()).max(1);
    for chunk_start in (0..samples.len()).step_by(step_len) {
        let chunk_end = (chunk_start + chunk_len).min(samples.len());
        let trim_start = if chunk_start == 0 {
            0
        } else {
            overlap_sample_limit() / 2
        };
        let trim_end = if chunk_end == samples.len() {
            chunk_end - chunk_start
        } else {
            (chunk_end - chunk_start).saturating_sub(overlap_sample_limit() / 2)
        };
        let chunk_samples = &samples[chunk_start..chunk_end];
        let chunk_duration_ms = chunk_samples.len() as u64 * 1000 / SAMPLE_RATE_HZ as u64;
        if let Some(skip_reason) = speaker_skip_reason(audio_peak(chunk_samples), chunk_duration_ms)
        {
            warning_reasons.push(format!("chunk_skipped_{skip_reason}"));
            continue;
        }
        chunk_count += 1;

        let result =
            diarizer
                .process(chunk_samples)
                .ok_or_else(|| SpeakerAnalysisError::Runtime {
                    stage: "diarize_safe_chunk".to_string(),
                    message: "sherpa-onnx diarization returned no result for a safe chunk"
                        .to_string(),
                })?;
        let segments = result.sort_by_start_time();
        let (mut chunk_clusters, mut chunk_turns) = analyze_single_safe_chunk(
            samples,
            chunk_start,
            chunk_samples.len(),
            trim_start,
            trim_end,
            &segments,
            extractor,
            next_local_cluster_key,
            selection.min_turn_ms,
            &mut warning_reasons,
        )?;
        next_local_cluster_key += chunk_clusters.len();
        local_clusters.append(&mut chunk_clusters);
        pending_turns.append(&mut chunk_turns);
    }
    output
        .metadata
        .provenance
        .insert("chunkCount".to_string(), json!(chunk_count));
    output
        .metadata
        .provenance
        .insert("warningReasons".to_string(), json!(warning_reasons));

    if local_clusters.is_empty() {
        finalize_provenance_counts(&mut output);
        return Ok(output);
    }

    // Accuracy #1: order-independent agglomerative pass over ALL chunk-local
    // clusters at once, instead of greedily assigning each local cluster as
    // chunks stream in. The greedy pass seeded global clusters in chunk order
    // and blended into a moving representative, so a real speaker could split
    // into several global clusters depending on which chunk arrived first.
    let (global_clusters, local_to_global) =
        agglomerate_local_clusters(&local_clusters, selection.cross_chunk_threshold);

    for pending in pending_turns {
        let Some(global_id) = local_to_global.get(&pending.local_cluster_key).copied() else {
            continue;
        };
        output.turns.push(SpeakerTurn {
            provider_cluster_id: provider_cluster_id(global_id as i32),
            start_ms: pending.start_ms,
            end_ms: pending.end_ms,
            transcript_text: None,
            overlaps: false,
        });
    }
    output.turns = mark_overlapping_turns(merge_adjacent_turns(output.turns));

    for cluster in global_clusters {
        // Cap the audio fed to the extractor: an unbounded concatenation of a
        // talkative speaker's whole-clip ranges overflows TitaNet's fixed input
        // dimension and aborts the process (see MAX_EMBEDDING_AUDIO_SAMPLES).
        let cluster_samples = if range_sample_count(&cluster.ranges) > MAX_EMBEDDING_AUDIO_SAMPLES {
            add_warning_reason(&mut output, "global_embedding_capped");
            concatenate_ranges_capped(samples, &cluster.ranges, MAX_EMBEDDING_AUDIO_SAMPLES)
        } else {
            concatenate_ranges(samples, &cluster.ranges)
        };
        let embedding = match compute_embedding(extractor, &cluster_samples) {
            Ok(embedding) => embedding,
            Err(_) => {
                add_warning_reason(&mut output, "global_embedding_fallback");
                cluster.representative_embedding
            }
        };
        let suggestion = if request.recognize_people {
            best_enrollment_match(request, &embedding, &selection.model_id)
        } else {
            None
        };
        output.clusters.push(SpeakerCluster {
            provider_cluster_id: provider_cluster_id(cluster.id as i32),
            stable_label: format!("Unknown Speaker {}", cluster.id + 1),
            embedding: f32_embedding_to_le_bytes(&embedding),
            embedding_model_id: selection.model_id.clone(),
            suggestion,
        });
    }
    finalize_provenance_counts(&mut output);

    Ok(output)
}

/// Which audio source produced a chunk-local cluster embedding, recorded so the
/// caller can set the correct cross-chunk blend weight and provenance warning.
#[cfg(feature = "sherpa-onnx")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClusterEmbeddingSource {
    /// The min-turn-filtered ranges (the preferred, cleanest signal).
    Filtered,
    /// This speaker's full unfiltered ranges — still single-speaker, used when
    /// the filtered ranges held too little audio for the extractor.
    UnfilteredRanges,
    /// The whole chunk (mixes speakers); last resort that can corrupt identity.
    WholeChunk,
}

#[cfg(feature = "sherpa-onnx")]
fn range_sample_count(ranges: &[(usize, usize)]) -> usize {
    ranges.iter().map(|(start, end)| end - start).sum()
}

/// Compute a chunk-local cluster embedding with a tiered fallback that keeps the
/// audio as close to a single speaker as possible.
///
/// Order: (1) the min-turn-filtered ranges; (2) this speaker's full unfiltered
/// ranges, but only when the filter actually dropped turns, since those ranges
/// are still single-speaker and merely add back short turns; (3) the whole
/// chunk, which mixes every speaker in the window and is used only when even the
/// speaker's full audio is too short for the extractor to be ready.
///
/// Tiers (1) and (2) are tried via `try_embed`, which returns an error when the
/// extractor lacks enough audio; that error is swallowed so the next tier runs.
/// Only the whole-chunk tier's error propagates.
#[cfg(feature = "sherpa-onnx")]
fn compute_cluster_embedding_with_fallback<E>(
    all_samples: &[f32],
    chunk_start: usize,
    chunk_len: usize,
    filtered_ranges: &[(usize, usize)],
    speaker_ranges: &[(usize, usize)],
    mut try_embed: E,
) -> SpeakerAnalysisResult<(Vec<f32>, ClusterEmbeddingSource)>
where
    E: FnMut(&[f32]) -> SpeakerAnalysisResult<Vec<f32>>,
{
    if let Ok(embedding) = try_embed(&concatenate_ranges(all_samples, filtered_ranges)) {
        return Ok((embedding, ClusterEmbeddingSource::Filtered));
    }
    // `filtered_ranges` is a subset of `speaker_ranges`; a length difference
    // means the min-turn filter dropped turns, so the unfiltered ranges hold
    // strictly more of this same speaker's audio and are worth trying before we
    // fall back to the whole, speaker-mixed chunk.
    if filtered_ranges.len() != speaker_ranges.len() {
        if let Ok(embedding) = try_embed(&concatenate_ranges(all_samples, speaker_ranges)) {
            return Ok((embedding, ClusterEmbeddingSource::UnfilteredRanges));
        }
    }
    let fallback_samples = &all_samples[chunk_start..chunk_start + chunk_len];
    Ok((try_embed(fallback_samples)?, ClusterEmbeddingSource::WholeChunk))
}

#[cfg(feature = "sherpa-onnx")]
fn analyze_single_safe_chunk(
    all_samples: &[f32],
    chunk_start: usize,
    chunk_len: usize,
    trim_start: usize,
    trim_end: usize,
    segments: &[sherpa_onnx_runtime::OfflineSpeakerDiarizationSegment],
    extractor: &sherpa_onnx_runtime::SpeakerEmbeddingExtractor,
    next_local_cluster_key: usize,
    min_turn_ms: u64,
    warning_reasons: &mut Vec<String>,
) -> SpeakerAnalysisResult<(Vec<LocalSpeakerCluster>, Vec<PendingSpeakerTurn>)> {
    let mut ranges_by_speaker = BTreeMap::<i32, Vec<(usize, usize)>>::new();
    let mut raw_turns = Vec::<(i32, u64, u64)>::new();

    for segment in segments {
        let local_start_ms = seconds_to_ms(segment.start);
        let local_end_ms = seconds_to_ms(segment.end);
        let start = ms_to_sample_index(local_start_ms, chunk_len);
        let end = ms_to_sample_index(local_end_ms, chunk_len);
        let trimmed_start = start.max(trim_start);
        let trimmed_end = end.min(trim_end);
        if trimmed_end <= trimmed_start {
            continue;
        }
        let global_start_ms = chunk_start as u64 * 1000 / SAMPLE_RATE_HZ as u64 + local_start_ms;
        let global_end_ms = chunk_start as u64 * 1000 / SAMPLE_RATE_HZ as u64 + local_end_ms;
        let trim_global_start_ms =
            (chunk_start + trimmed_start) as u64 * 1000 / SAMPLE_RATE_HZ as u64;
        let trim_global_end_ms = (chunk_start + trimmed_end) as u64 * 1000 / SAMPLE_RATE_HZ as u64;
        raw_turns.push((
            segment.speaker,
            global_start_ms.max(trim_global_start_ms),
            global_end_ms.min(trim_global_end_ms),
        ));

        let start = chunk_start + trimmed_start;
        let end = chunk_start + trimmed_end;
        if end > start {
            ranges_by_speaker
                .entry(segment.speaker)
                .or_default()
                .push((start, end));
        }
    }

    let min_turn_samples = min_turn_samples(min_turn_ms);
    let mut local_clusters = Vec::<LocalSpeakerCluster>::new();
    let mut speaker_to_local_key = BTreeMap::<i32, usize>::new();
    for (index, (speaker, ranges)) in ranges_by_speaker.into_iter().enumerate() {
        // Accuracy #2: skip sub-second (per-model `min_turn_ms`) ranges when
        // forming the cluster embedding, since short turns carry noisy speaker
        // identity. Preserve current behavior if filtering would drop every
        // range for this speaker in the chunk: keep all ranges rather than emit
        // a zero-length embedding or drop the speaker entirely.
        let embedding_ranges: Vec<(usize, usize)> = {
            let filtered: Vec<(usize, usize)> = ranges
                .iter()
                .copied()
                .filter(|(start, end)| end.saturating_sub(*start) >= min_turn_samples)
                .collect();
            if filtered.is_empty() {
                warning_reasons.push("chunk_all_turns_sub_min".to_string());
                ranges.clone()
            } else {
                filtered
            }
        };
        // Tiered embedding fallback: prefer the min-turn-filtered ranges, then
        // this speaker's full unfiltered ranges (still single-speaker) before
        // resorting to the whole chunk, which mixes every speaker in the window
        // and would corrupt this cluster's identity.
        let (embedding, embedding_source) = compute_cluster_embedding_with_fallback(
            all_samples,
            chunk_start,
            chunk_len,
            &embedding_ranges,
            &ranges,
            |samples| compute_embedding(extractor, samples),
        )?;
        let key = next_local_cluster_key + index;
        // `total_samples` is the cross-chunk blending weight, so weight by the
        // ranges that actually fed the embedding.
        let total_samples = match embedding_source {
            ClusterEmbeddingSource::Filtered => range_sample_count(&embedding_ranges),
            ClusterEmbeddingSource::UnfilteredRanges => {
                warning_reasons.push("chunk_unfiltered_ranges_fallback".to_string());
                range_sample_count(&ranges)
            }
            ClusterEmbeddingSource::WholeChunk => {
                warning_reasons.push("chunk_embedding_fallback".to_string());
                range_sample_count(&embedding_ranges)
            }
        };
        speaker_to_local_key.insert(speaker, key);
        local_clusters.push(LocalSpeakerCluster {
            key,
            ranges,
            embedding,
            total_samples,
        });
    }

    let mut pending_turns = Vec::new();
    for (speaker, start_ms, end_ms) in raw_turns {
        let Some(local_cluster_key) = speaker_to_local_key.get(&speaker).copied() else {
            continue;
        };
        pending_turns.push(PendingSpeakerTurn {
            local_cluster_key,
            start_ms,
            end_ms,
        });
    }

    Ok((local_clusters, pending_turns))
}

/// Working node for the agglomerative cross-chunk clustering pass. Each node
/// starts as a single chunk-local cluster and accumulates the local cluster
/// keys it absorbs as merges happen.
#[cfg(feature = "sherpa-onnx")]
#[derive(Debug, Clone)]
struct AgglomerativeNode {
    member_keys: Vec<usize>,
    ranges: Vec<(usize, usize)>,
    /// `total_samples`-weighted centroid embedding (average linkage).
    representative_embedding: Vec<f32>,
    representative_weight: usize,
}

/// Accuracy #1: one order-independent agglomerative clustering pass over ALL
/// chunk-local cluster embeddings.
///
/// Starting from one node per local cluster, this repeatedly merges the single
/// most-similar pair of nodes (by cosine similarity of their
/// `total_samples`-weighted centroid embeddings) while the best pairwise
/// similarity is `>= cross_chunk_threshold`, then stops. Centroid (average)
/// linkage reuses the existing `blend_embeddings` weighting, so a merged node's
/// representative is the `total_samples`-weighted mean of its members.
///
/// Order independence: the merge order is driven purely by pairwise similarity
/// (the globally best pair each round), with deterministic tie-breaking by node
/// index, not by the order chunks were processed. The same set of local
/// clusters therefore always collapses to the same partition regardless of
/// chunk arrival order. Final global cluster ids are assigned by ascending
/// minimum member local-cluster key so ids are stable for a given partition.
///
/// Returns the finalized global clusters and a `local key -> global id` map.
/// Above this many local clusters the dense pairwise-similarity matrix is
/// skipped in favour of recomputing similarities on demand. The matrix is
/// `node_count^2 * 4` bytes, so the cap bounds its worst case at `4096^2 * 4 B
/// = 64 MiB`. Realistic inputs stay far below it — per-segment analysis is
/// capped well under a thousand clusters — so the dense fast path is what runs
/// in practice; the cap is a safety valve against a pathological cluster count
/// (e.g. a multi-hour single-call input) allocating gigabytes.
#[cfg(feature = "sherpa-onnx")]
const MAX_DENSE_SIMILARITY_NODES: usize = 4096;

#[cfg(feature = "sherpa-onnx")]
fn agglomerate_local_clusters(
    local_clusters: &[LocalSpeakerCluster],
    cross_chunk_threshold: f32,
) -> (Vec<GlobalSpeakerClusterState>, BTreeMap<usize, usize>) {
    agglomerate_local_clusters_impl(
        local_clusters,
        cross_chunk_threshold,
        MAX_DENSE_SIMILARITY_NODES,
    )
}

#[cfg(feature = "sherpa-onnx")]
fn agglomerate_local_clusters_impl(
    local_clusters: &[LocalSpeakerCluster],
    cross_chunk_threshold: f32,
    max_dense_nodes: usize,
) -> (Vec<GlobalSpeakerClusterState>, BTreeMap<usize, usize>) {
    let mut nodes: Vec<Option<AgglomerativeNode>> = local_clusters
        .iter()
        .map(|local| {
            Some(AgglomerativeNode {
                member_keys: vec![local.key],
                ranges: local.ranges.clone(),
                representative_embedding: local.embedding.clone(),
                representative_weight: local.total_samples,
            })
        })
        .collect();
    let node_count = nodes.len();

    // Cache pairwise cosine similarities in a flat upper-triangular matrix.
    // The naive approach recomputes `cosine_similarity` for every live pair on
    // every merge round, which is O(rounds * n^2 * embedding_dim) and, with up
    // to n-1 merge rounds, O(n^3 * embedding_dim) overall. A merge only changes
    // the centroid of the surviving (`left`) node, so we instead refresh just
    // that node's row/column after each merge. The scan order, the `>`
    // comparison, and the `None`-initialised tie-break below are unchanged, so
    // the pair chosen each round is bit-identical to recomputing fresh.
    //
    // The matrix costs O(n^2) memory, so above `max_dense_nodes` we leave it
    // `None` and recompute similarities from the live centroids on demand in the
    // scan below (O(n) memory, back to O(n^3) time). The partition is identical
    // either way — the recompute path reads the same centroids the matrix would
    // have cached — so this is a pure space/time trade keyed only on size.
    let sim_index = |left: usize, right: usize| left * node_count + right;
    let mut similarities: Option<Vec<f32>> = if node_count <= max_dense_nodes {
        let mut matrix = vec![0.0f32; node_count * node_count];
        for left in 0..node_count {
            let left_embedding = nodes[left]
                .as_ref()
                .expect("node is live during initial fill")
                .representative_embedding
                .clone();
            for right in (left + 1)..node_count {
                let right_node =
                    nodes[right].as_ref().expect("node is live during initial fill");
                matrix[sim_index(left, right)] =
                    cosine_similarity(&left_embedding, &right_node.representative_embedding);
            }
        }
        Some(matrix)
    } else {
        None
    };

    // Repeatedly find and merge the globally most-similar pair of live nodes.
    // Scanning all live pairs each round (rather than merging in input order)
    // is what makes the result independent of chunk processing order.
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for left in 0..node_count {
            if nodes[left].is_none() {
                continue;
            }
            for right in (left + 1)..node_count {
                if nodes[right].is_none() {
                    continue;
                }
                let score = match &similarities {
                    Some(matrix) => matrix[sim_index(left, right)],
                    None => cosine_similarity(
                        &nodes[left]
                            .as_ref()
                            .expect("left node is live")
                            .representative_embedding,
                        &nodes[right]
                            .as_ref()
                            .expect("right node is live")
                            .representative_embedding,
                    ),
                };
                // A non-finite similarity (NaN/inf, e.g. from a non-finite
                // embedding) must never be selectable. `score > best_score` is
                // always false for NaN, so a NaN latched as the first
                // `None`-tie-break `best` could never be displaced and would
                // then pass the `score < threshold` break check below (`NaN <
                // threshold` is also false), merging unrelated speakers and
                // potentially cascading into a collapsed partition. Skipping
                // non-finite scores keeps merges to real, >= threshold pairs and
                // is a no-op for the finite embeddings of the normal path.
                if !score.is_finite() {
                    continue;
                }
                let is_better = match best {
                    Some((_, _, best_score)) => score > best_score,
                    None => true,
                };
                if is_better {
                    best = Some((left, right, score));
                }
            }
        }

        let Some((left, right, score)) = best else {
            break;
        };
        if score < cross_chunk_threshold {
            break;
        }

        // Merge `right` into `left`: blend the weighted centroid, union the
        // ranges and member keys. `blend_embeddings` weights the existing
        // centroid by its accumulated weight and the incoming centroid by its
        // weight, yielding the `total_samples`-weighted mean of all members
        // (average linkage), so the merge is commutative in the members.
        let mut right_node = nodes[right].take().expect("right node is live");
        let left_node = nodes[left].as_mut().expect("left node is live");
        blend_embeddings(
            &mut left_node.representative_embedding,
            left_node.representative_weight,
            &right_node.representative_embedding,
            right_node.representative_weight,
        );
        left_node.representative_weight += right_node.representative_weight;
        left_node.ranges.append(&mut right_node.ranges);
        left_node.member_keys.append(&mut right_node.member_keys);

        // Only `left`'s centroid changed, so refresh just its cached
        // similarities against every still-live node; `right` is now dead and is
        // skipped by the scan above. In recompute mode (`None`) there is no cache
        // to maintain — the scan recomputes from the live centroids next round.
        if let Some(matrix) = similarities.as_mut() {
            let left_embedding = nodes[left]
                .as_ref()
                .expect("merged node is live")
                .representative_embedding
                .clone();
            for other in 0..node_count {
                if other == left {
                    continue;
                }
                let Some(other_node) = nodes[other].as_ref() else {
                    continue;
                };
                let score =
                    cosine_similarity(&left_embedding, &other_node.representative_embedding);
                let (low, high) = if left < other {
                    (left, other)
                } else {
                    (other, left)
                };
                matrix[sim_index(low, high)] = score;
            }
        }
    }

    // Collect surviving nodes and order them by their smallest member key so
    // global cluster ids are stable for a given partition (independent of which
    // input index happened to be the merge target).
    let mut surviving: Vec<AgglomerativeNode> = nodes.into_iter().flatten().collect();
    surviving.sort_by_key(|node| node.member_keys.iter().copied().min().unwrap_or(usize::MAX));

    let mut global_clusters = Vec::with_capacity(surviving.len());
    let mut local_to_global = BTreeMap::new();
    for (id, node) in surviving.into_iter().enumerate() {
        for key in &node.member_keys {
            local_to_global.insert(*key, id);
        }
        global_clusters.push(GlobalSpeakerClusterState {
            id,
            ranges: node.ranges,
            representative_embedding: node.representative_embedding,
        });
    }

    (global_clusters, local_to_global)
}

#[cfg(feature = "sherpa-onnx")]
fn blend_embeddings(
    current: &mut [f32],
    current_weight: usize,
    incoming: &[f32],
    incoming_weight: usize,
) {
    if current.len() != incoming.len() {
        return;
    }
    let total_weight = current_weight + incoming_weight;
    if total_weight == 0 {
        return;
    }
    for (current_value, incoming_value) in current.iter_mut().zip(incoming) {
        *current_value = (*current_value * current_weight as f32
            + *incoming_value * incoming_weight as f32)
            / total_weight as f32;
    }
}

fn merge_adjacent_turns(mut turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    turns.sort_by_key(|turn| (turn.start_ms, turn.end_ms));
    let mut merged = Vec::<SpeakerTurn>::new();
    for turn in turns {
        if let Some(last) = merged.last_mut() {
            if last.provider_cluster_id == turn.provider_cluster_id
                && turn.start_ms <= last.end_ms.saturating_add(MERGE_ADJACENT_TURN_GAP_MS)
            {
                last.end_ms = last.end_ms.max(turn.end_ms);
                continue;
            }
        }
        merged.push(turn);
    }
    merged
}

fn safe_single_chunk_sample_limit() -> usize {
    SAMPLE_RATE_HZ as usize * SAFE_SINGLE_CHUNK_DIARIZATION_MS as usize / 1000
}

/// Convert a per-model minimum turn duration (ms) into a sample count at the
/// fixed 16 kHz analysis rate, for accuracy #2 sub-second turn filtering.
fn min_turn_samples(min_turn_ms: u64) -> usize {
    (min_turn_ms as usize)
        .saturating_mul(SAMPLE_RATE_HZ as usize)
        / 1000
}

fn overlap_sample_limit() -> usize {
    SAMPLE_RATE_HZ as usize * SAFE_CHUNK_OVERLAP_MS as usize / 1000
}

fn mark_overlapping_turns(mut turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    for index in 0..turns.len() {
        let overlaps = turns.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.provider_cluster_id != turns[index].provider_cluster_id
                && other.end_ms > turns[index].start_ms
                && other.start_ms < turns[index].end_ms
        });
        if overlaps {
            turns[index].overlaps = true;
        }
    }
    turns
}

#[cfg(feature = "sherpa-onnx")]
fn diarization_config(
    request: &SpeakerAnalysisRequest,
    selection: &SherpaModelSelection,
) -> sherpa_onnx_runtime::OfflineSpeakerDiarizationConfig {
    let threshold = sanitize_threshold(
        request
            .options
            .get(CLUSTERING_THRESHOLD_OPTION)
            .and_then(serde_json::Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(selection.clustering_threshold),
    );
    let num_clusters = sanitize_num_clusters(
        request
            .options
            .get(NUM_CLUSTERS_OPTION)
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-1),
    );
    let min_duration_on = sanitize_min_duration(
        request
            .options
            .get(MIN_DURATION_ON_OPTION)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.3) as f32,
    );
    let min_duration_off = sanitize_min_duration(
        request
            .options
            .get(MIN_DURATION_OFF_OPTION)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.5) as f32,
    );

    sherpa_onnx_runtime::OfflineSpeakerDiarizationConfig {
        segmentation: sherpa_onnx_runtime::OfflineSpeakerSegmentationModelConfig {
            pyannote: sherpa_onnx_runtime::OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(selection.segmentation_model_path.display().to_string()),
            },
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
        embedding: sherpa_onnx_runtime::SpeakerEmbeddingExtractorConfig {
            model: Some(selection.embedding_model_path.display().to_string()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        },
        clustering: sherpa_onnx_runtime::FastClusteringConfig {
            num_clusters,
            threshold,
        },
        min_duration_on,
        min_duration_off,
    }
}

#[cfg(feature = "sherpa-onnx")]
fn speaker_output_for_request(
    request: &SpeakerAnalysisRequest,
    selection: &SherpaModelSelection,
    duration_ms: u64,
    audio_peak: f32,
) -> SpeakerAnalysisOutput {
    let mut output = SpeakerAnalysisOutput::new(SpeakerAnalysisMetadata::from_request(request));
    output.provider_version = Some("sherpa-onnx/1.13.1".to_string());
    output
        .metadata
        .provenance
        .insert("schemaVersion".to_string(), json!(1));
    output.metadata.provenance.insert(
        "segmentationModelPath".to_string(),
        json!(selection.segmentation_model_path.display().to_string()),
    );
    output.metadata.provenance.insert(
        "embeddingModelPath".to_string(),
        json!(selection.embedding_model_path.display().to_string()),
    );
    output
        .metadata
        .provenance
        .insert("audioDurationMs".to_string(), json!(duration_ms));
    output
        .metadata
        .provenance
        .insert("audioPeak".to_string(), json!(audio_peak));
    output
        .metadata
        .provenance
        .insert("skipReason".to_string(), serde_json::Value::Null);
    output
        .metadata
        .provenance
        .insert("chunkingMode".to_string(), json!("single"));
    output
        .metadata
        .provenance
        .insert("chunkCount".to_string(), json!(0));
    output
        .metadata
        .provenance
        .insert("turnCount".to_string(), json!(0));
    output
        .metadata
        .provenance
        .insert("clusterCount".to_string(), json!(0));
    output.metadata.provenance.insert(
        "recognitionEnabled".to_string(),
        json!(request.recognize_people),
    );
    output
        .metadata
        .provenance
        .insert("warningReasons".to_string(), json!(Vec::<String>::new()));
    output
}

#[cfg(feature = "sherpa-onnx")]
fn validate_decoded_samples(samples: &[f32]) -> SpeakerAnalysisResult<()> {
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(SpeakerAnalysisError::Runtime {
            stage: "validate_decoded_samples".to_string(),
            message: "decoded speaker-analysis audio contained non-finite samples".to_string(),
        });
    }
    Ok(())
}

#[cfg(feature = "sherpa-onnx")]
fn audio_peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max)
}

#[cfg(feature = "sherpa-onnx")]
fn speaker_skip_reason(audio_peak: f32, duration_ms: u64) -> Option<&'static str> {
    if duration_ms < MIN_DIARIZATION_AUDIO_MS {
        return Some("too_short");
    }

    if audio_peak < MIN_DIARIZATION_PEAK {
        return Some("silent");
    }

    None
}

#[cfg(feature = "sherpa-onnx")]
fn finalize_provenance_counts(output: &mut SpeakerAnalysisOutput) {
    output
        .metadata
        .provenance
        .insert("turnCount".to_string(), json!(output.turns.len()));
    output
        .metadata
        .provenance
        .insert("clusterCount".to_string(), json!(output.clusters.len()));
}

#[cfg(feature = "sherpa-onnx")]
fn add_warning_reason(output: &mut SpeakerAnalysisOutput, reason: &str) {
    let mut reasons = output
        .metadata
        .provenance
        .get("warningReasons")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    reasons.push(reason.to_string());
    output
        .metadata
        .provenance
        .insert("warningReasons".to_string(), json!(reasons));
}

fn sanitize_threshold(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.05, 0.95)
    } else {
        DEFAULT_CLUSTERING_THRESHOLD
    }
}

fn sanitize_num_clusters(value: i32) -> i32 {
    match value {
        -1 => -1,
        1..=16 => value,
        value if value <= 0 => -1,
        _ => 16,
    }
}

fn sanitize_min_duration(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 5.0)
    } else {
        0.0
    }
}

fn resolve_model_selection(
    request: &SpeakerAnalysisRequest,
    models_dir: &Path,
) -> SpeakerAnalysisResult<SherpaModelSelection> {
    if request.provider != SHERPA_ONNX_PROVIDER_ID {
        return Err(SpeakerAnalysisError::InvalidRequest(format!(
            "sherpa-onnx provider received request for {}",
            request.provider
        )));
    }
    let model_id = request
        .model_id
        .clone()
        .unwrap_or_else(|| DEFAULT_SHERPA_ONNX_MODEL_ID.to_string());
    let descriptor = crate::builtin_model_manifest()
        .models
        .into_iter()
        .find(|model| {
            model.provider == SHERPA_ONNX_PROVIDER_ID
                && model.model_id.as_deref() == Some(model_id.as_str())
        })
        .ok_or_else(|| {
            SpeakerAnalysisError::InvalidRequest(format!(
                "unknown sherpa-onnx speaker analysis model id '{model_id}'"
            ))
        })?;
    let install_dir = model_install_dir(models_dir, &descriptor)
        .map_err(|error| SpeakerAnalysisError::InvalidRequest(error.to_string()))?;
    let params = descriptor.sherpa_params.as_ref().ok_or_else(|| {
        SpeakerAnalysisError::InvalidRequest(format!(
            "sherpa-onnx model id '{model_id}' is missing sherpa_params in the manifest descriptor"
        ))
    })?;
    // The cross-chunk agglomeration threshold drives the global cluster count on
    // long, multi-chunk audio. It defaults to the per-model calibrated value but
    // is request-overridable (like clusteringThreshold) so it can be swept by the
    // DER benchmark harness without rebuilding.
    let cross_chunk_threshold = request
        .options
        .get(CROSS_CHUNK_THRESHOLD_OPTION)
        .and_then(serde_json::Value::as_f64)
        .map(|value| sanitize_threshold(value as f32))
        .unwrap_or(params.cross_chunk_threshold);
    Ok(SherpaModelSelection {
        model_id,
        segmentation_model_path: install_dir.join(&params.segmentation_relative_path),
        embedding_model_path: install_dir.join(&params.embedding_relative_path),
        clustering_threshold: params.clustering_threshold,
        cross_chunk_threshold,
        min_turn_ms: params.min_turn_ms,
    })
}

#[cfg(feature = "sherpa-onnx")]
fn compute_embedding(
    extractor: &sherpa_onnx_runtime::SpeakerEmbeddingExtractor,
    samples: &[f32],
) -> SpeakerAnalysisResult<Vec<f32>> {
    let stream = extractor
        .create_stream()
        .ok_or_else(|| SpeakerAnalysisError::Runtime {
            stage: "create_embedding_stream".to_string(),
            message: "failed to create speaker embedding stream".to_string(),
        })?;
    stream.accept_waveform(SAMPLE_RATE_HZ as i32, samples);
    if !extractor.is_ready(&stream) {
        return Err(SpeakerAnalysisError::Runtime {
            stage: "compute_embedding".to_string(),
            message: "not enough speaker audio to compute embedding".to_string(),
        });
    }
    extractor
        .compute(&stream)
        .ok_or_else(|| SpeakerAnalysisError::Runtime {
            stage: "compute_embedding".to_string(),
            message: "failed to compute embedding".to_string(),
        })
}

fn best_enrollment_match(
    request: &SpeakerAnalysisRequest,
    embedding: &[f32],
    model_id: &str,
) -> Option<SpeakerRecognitionSuggestion> {
    let mut matches = request
        .enrolled_people
        .iter()
        .filter(|person| person.embedding_model_id == model_id)
        .filter_map(|person| {
            let enrolled = f32_embedding_from_le_bytes(&person.embedding)?;
            let score = cosine_similarity(&enrolled, embedding);
            if score < MIN_RECOGNITION_SUGGESTION_SCORE
                || has_similar_rejection(request, person.person_id, embedding, model_id)
            {
                return None;
            }
            let confidence = if score >= HIGH_RECOGNITION_SUGGESTION_SCORE {
                crate::RecognitionConfidence::High
            } else {
                crate::RecognitionConfidence::Medium
            };
            Some(SpeakerRecognitionSuggestion {
                person_id: person.person_id,
                display_name: person.display_name.clone(),
                confidence,
                score,
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.person_id.cmp(&right.person_id))
    });

    let best = matches.first()?;
    if matches
        .iter()
        .find(|candidate| candidate.person_id != best.person_id)
        .is_some_and(|second| best.score - second.score < PERSON_AMBIGUITY_MARGIN)
    {
        return None;
    }
    Some(best.clone())
}

fn has_similar_rejection(
    request: &SpeakerAnalysisRequest,
    person_id: i64,
    embedding: &[f32],
    model_id: &str,
) -> bool {
    request
        .rejected_people
        .iter()
        .filter(|rejection| {
            rejection.person_id == person_id && rejection.embedding_model_id == model_id
        })
        .filter_map(|rejection| f32_embedding_from_le_bytes(&rejection.embedding))
        .any(|rejected| {
            cosine_similarity(&rejected, embedding) >= REJECTED_PERSON_SIMILARITY_THRESHOLD
        })
}

fn provider_cluster_id(speaker: i32) -> String {
    format!("speaker_{speaker:02}")
}

fn seconds_to_ms(seconds: f32) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn ms_to_sample_index(ms: u64, sample_len: usize) -> usize {
    ((ms as usize).saturating_mul(SAMPLE_RATE_HZ as usize) / 1000).min(sample_len)
}

fn concatenate_ranges(samples: &[f32], ranges: &[(usize, usize)]) -> Vec<f32> {
    let len = ranges.iter().map(|(start, end)| end - start).sum();
    let mut out = Vec::with_capacity(len);
    for (start, end) in ranges {
        out.extend_from_slice(&samples[*start..*end]);
    }
    out
}

/// Like [`concatenate_ranges`] but stops once `max_samples` have been collected,
/// truncating the final range as needed. Ranges are taken in order, so the result
/// is the speaker's earliest `max_samples` of audio — enough for a representative
/// embedding while staying within the extractor's input limit.
fn concatenate_ranges_capped(
    samples: &[f32],
    ranges: &[(usize, usize)],
    max_samples: usize,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(range_sample_count(ranges).min(max_samples));
    for (start, end) in ranges {
        if out.len() >= max_samples {
            break;
        }
        let take = (max_samples - out.len()).min(end - start);
        out.extend_from_slice(&samples[*start..*start + take]);
    }
    out
}

fn f32_embedding_to_le_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for value in embedding {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn f32_embedding_from_le_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let a_norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let b_norm = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (a_norm * b_norm).max(f32::EPSILON)
}

#[cfg(all(feature = "sherpa-onnx", target_os = "macos"))]
fn decode_audio_to_mono_16khz(path: &Path) -> SpeakerAnalysisResult<Vec<f32>> {
    let decoded = avfoundation_decode_audio_to_mono(path)?;
    Ok(resample_linear(
        &decoded.samples,
        decoded.sample_rate_hz,
        SAMPLE_RATE_HZ,
    ))
}

#[cfg(all(feature = "sherpa-onnx", not(target_os = "macos")))]
fn decode_audio_to_mono_16khz(_path: &Path) -> SpeakerAnalysisResult<Vec<f32>> {
    Err(SpeakerAnalysisError::ProviderUnavailable(
        "sherpa-onnx audio decoding is only implemented with AVFoundation on macOS in v1"
            .to_string(),
    ))
}

#[cfg(all(feature = "sherpa-onnx", target_os = "macos"))]
fn avfoundation_decode_audio_to_mono(path: &Path) -> SpeakerAnalysisResult<DecodedAudio> {
    decode_audio_to_mono_with_avassetreader_fallback(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PersonEnrollment, PersonRecognitionRejection, SpeakerAnalysisRequest};

    fn selection_for(model_id: &str) -> SherpaModelSelection {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SHERPA_ONNX_PROVIDER_ID,
            Some(model_id.to_string()),
            "session-a",
            7,
        );
        resolve_model_selection(&request, Path::new("/tmp/models")).expect("model")
    }

    #[test]
    fn resolves_model_paths_from_app_model_store() {
        let selection = selection_for(DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert_eq!(selection.model_id, DEFAULT_SHERPA_ONNX_MODEL_ID);
        assert_eq!(
            selection.segmentation_model_path,
            PathBuf::from("/tmp/models/sherpa_onnx/pyannote-3.0-nemo-titanet-small/pyannote-segmentation-3.0/model.onnx")
        );
        assert_eq!(
            selection.embedding_model_path,
            PathBuf::from(
                "/tmp/models/sherpa_onnx/pyannote-3.0-nemo-titanet-small/nemo_en_titanet_small.onnx"
            )
        );
        // Balanced preset: clustering threshold is the historical 0.65; the
        // cross-chunk threshold is the DER-benchmark optimum 0.60.
        assert_eq!(selection.clustering_threshold, 0.65_f32);
        assert_eq!(selection.cross_chunk_threshold, 0.60_f32);
        assert_eq!(selection.cross_chunk_threshold, crate::BALANCED_CROSS_CHUNK_THRESHOLD);
        assert_eq!(selection.min_turn_ms, crate::DEFAULT_MIN_TURN_MS);
    }

    #[test]
    fn resolves_multilingual_preset_paths() {
        let selection = selection_for(crate::MULTILINGUAL_SHERPA_ONNX_MODEL_ID);

        assert_eq!(selection.model_id, crate::MULTILINGUAL_SHERPA_ONNX_MODEL_ID);
        assert_eq!(
            selection.segmentation_model_path,
            PathBuf::from(
                "/tmp/models/sherpa_onnx/pyannote-3.0-campplus-zh-en/pyannote-segmentation-3.0/model.onnx"
            )
        );
        assert_eq!(
            selection.embedding_model_path,
            PathBuf::from(
                "/tmp/models/sherpa_onnx/pyannote-3.0-campplus-zh-en/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"
            )
        );
    }

    #[test]
    fn resolves_high_accuracy_preset_paths() {
        let selection = selection_for(crate::HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID);

        assert_eq!(selection.model_id, crate::HIGH_ACCURACY_SHERPA_ONNX_MODEL_ID);
        assert_eq!(
            selection.segmentation_model_path,
            PathBuf::from(
                "/tmp/models/sherpa_onnx/reverb-v1-nemo-titanet-large/reverb-diarization-v1/model.onnx"
            )
        );
        assert_eq!(
            selection.embedding_model_path,
            PathBuf::from(
                "/tmp/models/sherpa_onnx/reverb-v1-nemo-titanet-large/nemo_en_titanet_large.onnx"
            )
        );
    }

    #[test]
    fn rejects_unknown_model_id() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SHERPA_ONNX_PROVIDER_ID,
            Some("does-not-exist".to_string()),
            "session-a",
            7,
        );
        let error =
            resolve_model_selection(&request, Path::new("/tmp/models")).expect_err("unknown id");
        assert!(matches!(error, SpeakerAnalysisError::InvalidRequest(_)));
    }

    #[test]
    fn min_turn_samples_converts_ms_at_16khz() {
        assert_eq!(min_turn_samples(0), 0);
        assert_eq!(min_turn_samples(500), 8_000);
        assert_eq!(min_turn_samples(1_000), 16_000);
    }

    #[test]
    fn resamples_to_target_rate() {
        let samples = vec![0.0, 1.0, 0.0, -1.0];
        let out = resample_linear(&samples, 4, 2);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.0).abs() < 0.0001);
        assert!((out[1] - 0.0).abs() < 0.0001);
    }

    #[test]
    fn embedding_bytes_round_trip() {
        let embedding = vec![0.1, -0.2, 0.3];
        let bytes = f32_embedding_to_le_bytes(&embedding);
        assert_eq!(f32_embedding_from_le_bytes(&bytes), Some(embedding));
    }

    fn request_with_enrollment(score: f32) -> SpeakerAnalysisRequest {
        let mut request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        request.enrolled_people.push(PersonEnrollment {
            person_id: 1,
            display_name: "Jack".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(score)),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });
        request
    }

    fn unit_embedding_for_score(score: f32) -> [f32; 2] {
        [score, (1.0 - score.powi(2)).max(0.0).sqrt()]
    }

    #[test]
    fn recognition_skips_weak_best_match() {
        let request = request_with_enrollment(0.59);

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_maps_high_confidence_from_strict_threshold() {
        let request = request_with_enrollment(0.72);

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID)
            .expect("suggestion");

        assert_eq!(suggestion.confidence, crate::RecognitionConfidence::High);
        assert!(suggestion.score >= 0.72);
    }

    #[test]
    fn recognition_maps_medium_confidence_from_minimum_threshold() {
        let request = request_with_enrollment(0.60);

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID)
            .expect("suggestion");

        assert_eq!(suggestion.confidence, crate::RecognitionConfidence::Medium);
        assert!(suggestion.score >= 0.60);
        assert!(suggestion.score < 0.72);
    }

    #[test]
    fn recognition_skips_person_with_similar_rejection() {
        let mut request = request_with_enrollment(1.0);
        request.rejected_people.push(PersonRecognitionRejection {
            person_id: 1,
            embedding: f32_embedding_to_le_bytes(&[1.0, 0.0]),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_skips_ambiguous_top_two_people() {
        let mut request = request_with_enrollment(0.72);
        request.enrolled_people.push(PersonEnrollment {
            person_id: 2,
            display_name: "Jill".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(0.68)),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_keeps_close_same_person_enrollments_unambiguous() {
        let mut request = request_with_enrollment(0.72);
        request.enrolled_people.push(PersonEnrollment {
            person_id: 1,
            display_name: "Jack".to_string(),
            embedding: f32_embedding_to_le_bytes(&unit_embedding_for_score(0.71)),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID)
            .expect("same-person enrollments should not be ambiguous");

        assert_eq!(suggestion.person_id, 1);
        assert!(suggestion.score >= 0.72);
    }

    #[test]
    fn clamps_sherpa_runtime_options_before_native_call() {
        assert_eq!(sanitize_threshold(f32::NAN), DEFAULT_CLUSTERING_THRESHOLD);
        assert_eq!(sanitize_threshold(3.0), 0.95);
        assert_eq!(sanitize_threshold(-3.0), 0.05);
        assert_eq!(sanitize_num_clusters(-10), -1);
        assert_eq!(sanitize_num_clusters(0), -1);
        assert_eq!(sanitize_num_clusters(3), 3);
        assert_eq!(sanitize_num_clusters(100), 16);
        assert_eq!(sanitize_min_duration(f32::INFINITY), 0.0);
        assert_eq!(sanitize_min_duration(30.0), 5.0);
    }

    #[test]
    fn merges_adjacent_turns_for_same_cluster() {
        let turns = vec![
            SpeakerTurn {
                provider_cluster_id: "speaker_00".to_string(),
                start_ms: 0,
                end_ms: 1_000,
                transcript_text: None,
                overlaps: false,
            },
            SpeakerTurn {
                provider_cluster_id: "speaker_00".to_string(),
                start_ms: 1_050,
                end_ms: 2_000,
                transcript_text: None,
                overlaps: false,
            },
            SpeakerTurn {
                provider_cluster_id: "speaker_01".to_string(),
                start_ms: 2_500,
                end_ms: 3_000,
                transcript_text: None,
                overlaps: false,
            },
        ];

        let merged = merge_adjacent_turns(turns);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start_ms, 0);
        assert_eq!(merged[0].end_ms, 2_000);
    }

    #[cfg(feature = "sherpa-onnx")]
    fn local_cluster(key: usize, embedding: Vec<f32>) -> LocalSpeakerCluster {
        LocalSpeakerCluster {
            key,
            ranges: vec![(key * 100, key * 100 + 100)],
            embedding,
            total_samples: 100,
        }
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn agglomerative_merges_similar_local_clusters_and_splits_distinct() {
        let locals = vec![
            local_cluster(1, vec![1.0, 0.0]),
            local_cluster(2, vec![0.95, 0.05]),
            local_cluster(3, vec![0.0, 1.0]),
        ];

        let (global_clusters, local_to_global) =
            agglomerate_local_clusters(&locals, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);

        assert_eq!(global_clusters.len(), 2);
        assert_eq!(local_to_global[&1], local_to_global[&2]);
        assert_ne!(local_to_global[&1], local_to_global[&3]);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn agglomerative_is_order_independent() {
        // Three near-identical embeddings for one real speaker plus one
        // distinct speaker. Greedy chunk-order assignment could split the
        // first speaker; the agglomerative pass must collapse them regardless
        // of input order and produce the same partition either way.
        let forward = vec![
            local_cluster(0, vec![1.0, 0.0, 0.0]),
            local_cluster(1, vec![0.92, 0.10, 0.05]),
            local_cluster(2, vec![0.88, 0.12, 0.08]),
            local_cluster(3, vec![0.0, 0.0, 1.0]),
        ];
        let mut reversed = forward.clone();
        reversed.reverse();

        let (forward_clusters, forward_map) =
            agglomerate_local_clusters(&forward, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);
        let (reversed_clusters, reversed_map) =
            agglomerate_local_clusters(&reversed, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);

        // Same number of clusters and the same local->global partition.
        assert_eq!(forward_clusters.len(), reversed_clusters.len());
        assert_eq!(forward_clusters.len(), 2);
        assert_eq!(forward_map, reversed_map);
        // The three similar speakers collapse into one cluster.
        assert_eq!(forward_map[&0], forward_map[&1]);
        assert_eq!(forward_map[&0], forward_map[&2]);
        assert_ne!(forward_map[&0], forward_map[&3]);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn agglomerative_single_chunk_one_cluster_per_speaker() {
        // Single-chunk path: distinct speakers, nothing to merge across chunks.
        let locals = vec![
            local_cluster(0, vec![1.0, 0.0]),
            local_cluster(1, vec![0.0, 1.0]),
        ];

        let (global_clusters, local_to_global) =
            agglomerate_local_clusters(&locals, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);

        assert_eq!(global_clusters.len(), 2);
        assert_ne!(local_to_global[&0], local_to_global[&1]);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn agglomerative_does_not_merge_on_non_finite_similarity() {
        // A non-finite embedding makes `cosine_similarity` return NaN. With an
        // unguarded scan the first (0,1) pair latches as `best` via the
        // `None`-tie-break, can never be displaced (`x > NaN` is false), and
        // `NaN < threshold` does not break — so cluster 0 would be merged into 1
        // despite an undefined similarity. The guard must keep the NaN-producing
        // cluster separate while the genuinely similar finite clusters still merge.
        let locals = vec![
            local_cluster(0, vec![f32::NAN, 0.0]),
            local_cluster(1, vec![1.0, 0.0]),
            local_cluster(2, vec![0.95, 0.05]),
        ];

        let (_, map) =
            agglomerate_local_clusters(&locals, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);

        assert_eq!(map[&1], map[&2], "finite, near-identical clusters still merge");
        assert_ne!(map[&0], map[&1], "non-finite cluster must not be merged in");
    }

    // Reference reimplementation of the original O(n^3) agglomeration: recompute
    // the globally-best pair fresh every round and merge. The cached-similarity
    // optimisation in `agglomerate_local_clusters` must produce a bit-identical
    // partition to this.
    #[cfg(feature = "sherpa-onnx")]
    fn reference_partition(
        locals: &[LocalSpeakerCluster],
        threshold: f32,
    ) -> BTreeMap<usize, usize> {
        #[derive(Clone)]
        struct Node {
            keys: Vec<usize>,
            emb: Vec<f32>,
            weight: usize,
        }
        let mut nodes: Vec<Option<Node>> = locals
            .iter()
            .map(|l| {
                Some(Node {
                    keys: vec![l.key],
                    emb: l.embedding.clone(),
                    weight: l.total_samples,
                })
            })
            .collect();
        loop {
            let mut best: Option<(usize, usize, f32)> = None;
            for i in 0..nodes.len() {
                let Some(ni) = &nodes[i] else { continue };
                for j in (i + 1)..nodes.len() {
                    let Some(nj) = &nodes[j] else { continue };
                    let score = cosine_similarity(&ni.emb, &nj.emb);
                    if best.map_or(true, |(_, _, b)| score > b) {
                        best = Some((i, j, score));
                    }
                }
            }
            let Some((i, j, score)) = best else { break };
            if score < threshold {
                break;
            }
            let mut nj = nodes[j].take().unwrap();
            let ni = nodes[i].as_mut().unwrap();
            blend_embeddings(&mut ni.emb, ni.weight, &nj.emb, nj.weight);
            ni.weight += nj.weight;
            ni.keys.append(&mut nj.keys);
        }
        let mut surviving: Vec<Node> = nodes.into_iter().flatten().collect();
        surviving.sort_by_key(|n| n.keys.iter().copied().min().unwrap_or(usize::MAX));
        let mut map = BTreeMap::new();
        for (id, n) in surviving.into_iter().enumerate() {
            for k in n.keys {
                map.insert(k, id);
            }
        }
        map
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn agglomerative_matches_bruteforce_reference_on_random_embeddings() {
        // Deterministic xorshift so the differential check is reproducible.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u32 << 24) as f32
        };
        for trial in 0..300 {
            let n = 2 + (trial % 14);
            let dim = 4;
            let locals: Vec<LocalSpeakerCluster> = (0..n)
                .map(|k| LocalSpeakerCluster {
                    key: k,
                    ranges: vec![(k * 10, k * 10 + 5)],
                    embedding: (0..dim).map(|_| next() * 2.0 - 1.0).collect(),
                    total_samples: 1 + (k % 4),
                })
                .collect();
            let expected = reference_partition(&locals, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);
            // Dense matrix path (the default for any realistic node count).
            let (_, got_dense) =
                agglomerate_local_clusters(&locals, crate::DEFAULT_CROSS_CHUNK_THRESHOLD);
            assert_eq!(
                got_dense, expected,
                "dense partition mismatch at trial {trial} (n={n})"
            );
            // Recompute fallback: force it with max_dense_nodes = 0 and require
            // the same partition, so the memory-bounded large-n path stays
            // bit-identical to the matrix path.
            let (_, got_recompute) = agglomerate_local_clusters_impl(
                &locals,
                crate::DEFAULT_CROSS_CHUNK_THRESHOLD,
                0,
            );
            assert_eq!(
                got_recompute, expected,
                "recompute partition mismatch at trial {trial} (n={n})"
            );
        }
    }

    // --- P2: tiered cluster-embedding fallback ---------------------------------
    // `try_embed` models the extractor: it is only "ready" once it has at least
    // `min_ready` samples, mirroring `compute_embedding`'s `is_ready` gate.
    #[cfg(feature = "sherpa-onnx")]
    fn embed_with_min_ready(min_ready: usize) -> impl FnMut(&[f32]) -> SpeakerAnalysisResult<Vec<f32>> {
        move |samples: &[f32]| {
            if samples.len() >= min_ready {
                // Marker embedding: the sample count actually fed in.
                Ok(vec![samples.len() as f32])
            } else {
                Err(SpeakerAnalysisError::Runtime {
                    stage: "compute_embedding".to_string(),
                    message: "not enough speaker audio to compute embedding".to_string(),
                })
            }
        }
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn cluster_embedding_prefers_filtered_ranges() {
        let all_samples = vec![0.0f32; 1_000];
        let filtered = vec![(0usize, 200usize)];
        let speaker = vec![(0usize, 200usize), (300, 360)];
        let (embedding, source) = compute_cluster_embedding_with_fallback(
            &all_samples,
            0,
            1_000,
            &filtered,
            &speaker,
            embed_with_min_ready(100),
        )
        .expect("embedding");
        assert_eq!(source, ClusterEmbeddingSource::Filtered);
        assert_eq!(embedding, vec![200.0]); // only the filtered range was embedded
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn cluster_embedding_falls_back_to_unfiltered_before_whole_chunk() {
        // The bug: filtered ranges are too short for the extractor, but this
        // speaker's full ranges are long enough. We must use the speaker's own
        // audio, not the speaker-mixed whole chunk.
        let all_samples = vec![0.0f32; 1_000];
        let filtered = vec![(0usize, 30usize)]; // 30 samples, below min_ready
        let speaker = vec![(0usize, 30usize), (40, 200)]; // 190 samples total
        let (embedding, source) = compute_cluster_embedding_with_fallback(
            &all_samples,
            0,
            1_000,
            &filtered,
            &speaker,
            embed_with_min_ready(100),
        )
        .expect("embedding");
        assert_eq!(source, ClusterEmbeddingSource::UnfilteredRanges);
        assert_eq!(embedding, vec![190.0]); // speaker's full ranges, NOT 1000 (chunk)
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn cluster_embedding_uses_whole_chunk_only_when_even_unfiltered_too_short() {
        let all_samples = vec![0.0f32; 1_000];
        let filtered = vec![(0usize, 20usize)];
        let speaker = vec![(0usize, 20usize), (40, 70)]; // 50 samples total, still short
        let (embedding, source) = compute_cluster_embedding_with_fallback(
            &all_samples,
            0,
            1_000,
            &filtered,
            &speaker,
            embed_with_min_ready(100),
        )
        .expect("embedding");
        assert_eq!(source, ClusterEmbeddingSource::WholeChunk);
        assert_eq!(embedding, vec![1_000.0]); // the whole chunk
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn cluster_embedding_skips_unfiltered_when_no_turns_were_dropped() {
        // Filter dropped nothing (filtered == speaker ranges), so there is no
        // distinct unfiltered tier to try; go straight to the whole chunk.
        let all_samples = vec![0.0f32; 1_000];
        let ranges = vec![(0usize, 50usize)];
        let mut calls = 0usize;
        let (embedding, source) = compute_cluster_embedding_with_fallback(
            &all_samples,
            0,
            1_000,
            &ranges,
            &ranges,
            |samples: &[f32]| {
                calls += 1;
                embed_with_min_ready(100)(samples)
            },
        )
        .expect("embedding");
        assert_eq!(source, ClusterEmbeddingSource::WholeChunk);
        assert_eq!(embedding, vec![1_000.0]);
        // Only two attempts: filtered (fails) then whole chunk. No unfiltered tier.
        assert_eq!(calls, 2);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn skips_sherpa_for_short_or_silent_audio() {
        assert_eq!(speaker_skip_reason(0.1, 500), Some("too_short"));
        assert_eq!(speaker_skip_reason(0.0, 2_000), Some("silent"));
        assert_eq!(speaker_skip_reason(0.1, 2_000), None);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn empty_skip_output_includes_provenance() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        let selection = resolve_model_selection(&request, Path::new("/tmp/models")).expect("model");
        let mut output = speaker_output_for_request(&request, &selection, 500, 0.0);
        output
            .metadata
            .provenance
            .insert("skipReason".to_string(), json!("too_short"));
        finalize_provenance_counts(&mut output);

        let provenance = &output.metadata.provenance;
        assert_eq!(provenance.get("schemaVersion"), Some(&json!(1)));
        assert_eq!(provenance.get("audioDurationMs"), Some(&json!(500)));
        assert_eq!(provenance.get("audioPeak"), Some(&json!(0.0)));
        assert_eq!(provenance.get("skipReason"), Some(&json!("too_short")));
        assert_eq!(provenance.get("chunkingMode"), Some(&json!("single")));
        assert_eq!(provenance.get("chunkCount"), Some(&json!(0)));
        assert_eq!(provenance.get("turnCount"), Some(&json!(0)));
        assert_eq!(provenance.get("clusterCount"), Some(&json!(0)));
        assert_eq!(provenance.get("recognitionEnabled"), Some(&json!(false)));
        assert_eq!(
            provenance.get("warningReasons"),
            Some(&json!(Vec::<String>::new()))
        );
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn missing_models_return_typed_errors() {
        let temp = tempfile::tempdir().expect("tempdir");
        let audio_path = temp.path().join("audio.m4a");
        std::fs::write(&audio_path, b"not real audio").expect("audio fixture");
        let request = SpeakerAnalysisRequest::new(
            &audio_path,
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            7,
        );

        let error = analyze_sherpa_samples_blocking(
            request.clone(),
            temp.path(),
            vec![0.1; SAMPLE_RATE_HZ as usize * 2],
        )
        .expect_err("missing segmentation model should fail");

        assert!(matches!(
            error,
            SpeakerAnalysisError::MissingModel { ref model_kind, .. }
                if model_kind == "segmentation"
        ));

        let selection = resolve_model_selection(&request, temp.path()).expect("model");
        std::fs::create_dir_all(
            selection
                .segmentation_model_path
                .parent()
                .expect("segmentation parent"),
        )
        .expect("segmentation dir");
        std::fs::write(&selection.segmentation_model_path, b"model").expect("segmentation model");
        let error = analyze_sherpa_samples_blocking(
            request,
            temp.path(),
            vec![0.1; SAMPLE_RATE_HZ as usize * 2],
        )
        .expect_err("missing embedding model should fail");

        assert!(matches!(
            error,
            SpeakerAnalysisError::MissingModel { ref model_kind, .. }
                if model_kind == "embedding"
        ));
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    fn non_finite_samples_return_typed_runtime_error() {
        let error =
            validate_decoded_samples(&[0.0, f32::NAN]).expect_err("non-finite samples should fail");

        assert!(matches!(
            error,
            SpeakerAnalysisError::Runtime { ref stage, .. }
                if stage == "validate_decoded_samples"
        ));
    }

    #[test]
    fn concatenate_ranges_capped_truncates_in_order() {
        let samples: Vec<f32> = (0..100).map(|n| n as f32).collect();
        let ranges = [(0usize, 30usize), (50, 80), (90, 100)]; // 30 + 30 + 10 = 70 samples

        // Under the cap: identical to the uncapped concatenation.
        assert_eq!(
            concatenate_ranges_capped(&samples, &ranges, 1000),
            concatenate_ranges(&samples, &ranges)
        );

        // Over the cap: earliest `max_samples` only, truncating the spanning range.
        let capped = concatenate_ranges_capped(&samples, &ranges, 45);
        assert_eq!(capped.len(), 45);
        let mut expected: Vec<f32> = (0..30).map(|n| n as f32).collect();
        expected.extend((50..65).map(|n| n as f32)); // 15 of the second range
        assert_eq!(capped, expected);
    }

    #[cfg(feature = "sherpa-onnx")]
    #[test]
    #[ignore = "manual local repro; set MNEMA_SPEAKER_ANALYSIS_REPRO_AUDIO and MNEMA_SPEAKER_ANALYSIS_MODELS_DIR"]
    fn repro_local_sherpa_diarization() {
        let audio_path = std::env::var("MNEMA_SPEAKER_ANALYSIS_REPRO_AUDIO")
            .expect("MNEMA_SPEAKER_ANALYSIS_REPRO_AUDIO should point at an audio file");
        let models_dir = std::env::var("MNEMA_SPEAKER_ANALYSIS_MODELS_DIR")
            .expect("MNEMA_SPEAKER_ANALYSIS_MODELS_DIR should point at speaker-analysis-models");
        let request = SpeakerAnalysisRequest::new(
            audio_path,
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "manual-repro",
            1,
        );

        let output =
            analyze_sherpa_request_blocking(request, Path::new(&models_dir)).expect("analysis");

        assert!(!output.turns.is_empty(), "speaker turns should be returned");
    }

    /// Slice 4 cross-chunk clustering validation harness.
    ///
    /// Count-level smoke test (not a DER benchmark) that runs the real
    /// sherpa-onnx provider on two known clips and prints the resulting
    /// global cluster counts. Reads the models dir and clip paths from env
    /// vars so it stays reproducible across machines:
    /// - `MNEMA_SPEAKER_ANALYSIS_MODELS_DIR`
    /// - `MNEMA_SPEAKER_ANALYSIS_CLIP_3SPK` (chunked >10s path, 3 speakers)
    /// - `MNEMA_SPEAKER_ANALYSIS_CLIP_2SPK` (2 speakers)
    ///
    /// Run with:
    /// `cargo test -p speaker-analysis --features sherpa-onnx -- --ignored cross_chunk_cluster_count_validation_harness --nocapture`
    #[cfg(feature = "sherpa-onnx")]
    #[test]
    #[ignore = "integration: requires downloaded models + local clips; run with --ignored --nocapture"]
    fn cross_chunk_cluster_count_validation_harness() {
        let models_dir = std::env::var("MNEMA_SPEAKER_ANALYSIS_MODELS_DIR").unwrap_or_else(|_| {
            format!(
                "{}/Library/Application Support/com.shaikzeeshan.mnema/speaker-analysis-models",
                std::env::var("HOME").expect("HOME")
            )
        });
        let clip_3spk = std::env::var("MNEMA_SPEAKER_ANALYSIS_CLIP_3SPK").unwrap_or_else(|_| {
            format!(
                "{}/Downloads/test_1.wav",
                std::env::var("HOME").expect("HOME")
            )
        });
        let clip_2spk = std::env::var("MNEMA_SPEAKER_ANALYSIS_CLIP_2SPK").unwrap_or_else(|_| {
            format!(
                "{}/Downloads/test.wav",
                std::env::var("HOME").expect("HOME")
            )
        });

        let _provider = SherpaOnnxSpeakerAnalysisProvider::with_models_dir(&models_dir);
        let mut analyzed_any = false;
        for (label, clip, expected) in [
            ("test_1.wav (3 speakers, chunked)", clip_3spk, 3usize),
            ("test.wav (2 speakers)", clip_2spk, 2usize),
        ] {
            if !Path::new(&clip).is_file() {
                println!("[slice4-validation] {label}: SKIPPED (clip not found at {clip})");
                continue;
            }
            let request = SpeakerAnalysisRequest::new(
                &clip,
                SHERPA_ONNX_PROVIDER_ID,
                Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
                "slice4-validation",
                1,
            );
            let output = match analyze_sherpa_request_blocking(request, Path::new(&models_dir)) {
                Ok(output) => output,
                Err(error) => {
                    println!(
                        "[slice4-validation] {label}: SKIPPED (analysis error, e.g. unreadable file): {error:?}"
                    );
                    continue;
                }
            };
            analyzed_any = true;
            let chunking = output
                .metadata
                .provenance
                .get("chunkingMode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            let chunk_count = output
                .metadata
                .provenance
                .get("chunkCount")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            println!(
                "[slice4-validation] {label}: clusters={} turns={} chunkingMode={} chunkCount={} (expected~{expected})",
                output.clusters.len(),
                output.turns.len(),
                chunking,
                chunk_count,
            );
            assert!(
                !output.turns.is_empty(),
                "speaker turns should be returned for {label}"
            );
        }
        assert!(
            analyzed_any,
            "no validation clip could be analyzed; set MNEMA_SPEAKER_ANALYSIS_CLIP_3SPK / _2SPK"
        );
    }
}
