use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use async_trait::async_trait;
use serde_json::json;

use crate::{
    macos_audio_decode::{
        decode_audio_to_mono_with_avassetreader_fallback, resample_linear, DecodedAudio,
    },
    model_install_dir, SpeakerAnalysisError, SpeakerAnalysisMetadata, SpeakerAnalysisOutput,
    SpeakerAnalysisProvider, SpeakerAnalysisRequest, SpeakerAnalysisResult, SpeakerCluster,
    SpeakerRecognitionSuggestion, SpeakerTurn, DEFAULT_SHERPA_ONNX_MODEL_ID,
    SHERPA_ONNX_PROVIDER_ID,
};

const SAMPLE_RATE_HZ: u32 = 16_000;
const SEGMENTATION_MODEL_RELATIVE_PATH: &str = "pyannote-segmentation-3.0/model.onnx";
const EMBEDDING_MODEL_RELATIVE_PATH: &str = "nemo_en_titanet_small.onnx";
const CLUSTERING_THRESHOLD_OPTION: &str = "clusteringThreshold";
const NUM_CLUSTERS_OPTION: &str = "numClusters";
const MIN_DURATION_ON_OPTION: &str = "minDurationOn";
const MIN_DURATION_OFF_OPTION: &str = "minDurationOff";
const MIN_DIARIZATION_AUDIO_MS: u64 = 1_000;
const MIN_DIARIZATION_PEAK: f32 = 1.0e-5;
const DEFAULT_CLUSTERING_THRESHOLD: f32 = 0.65;
const SAFE_SINGLE_CHUNK_DIARIZATION_MS: u64 = 10_000;
const SAFE_CHUNK_OVERLAP_MS: u64 = 1_000;
const CROSS_CHUNK_CLUSTER_SIMILARITY_THRESHOLD: f32 = 0.60;
const MERGE_ADJACENT_TURN_GAP_MS: u64 = 250;
const MIN_RECOGNITION_SUGGESTION_SCORE: f32 = 0.50;
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
    representative_weight: usize,
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
    let samples = decode_audio_to_mono_16khz(&request.audio_path)?;
    run_sherpa_on_samples(request, models_dir, samples)
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

    let mut global_clusters = Vec::<GlobalSpeakerClusterState>::new();
    let mut local_to_global = BTreeMap::<usize, usize>::new();
    for local in &local_clusters {
        let global_id = assign_global_cluster(&mut global_clusters, local);
        local_to_global.insert(local.key, global_id);
    }

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
        let cluster_samples = concatenate_ranges(samples, &cluster.ranges);
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

    let mut local_clusters = Vec::<LocalSpeakerCluster>::new();
    let mut speaker_to_local_key = BTreeMap::<i32, usize>::new();
    for (index, (speaker, ranges)) in ranges_by_speaker.into_iter().enumerate() {
        let cluster_samples = concatenate_ranges(all_samples, &ranges);
        let embedding = match compute_embedding(extractor, &cluster_samples) {
            Ok(embedding) => embedding,
            Err(_) => {
                warning_reasons.push("chunk_embedding_fallback".to_string());
                let fallback_samples = &all_samples[chunk_start..chunk_start + chunk_len];
                compute_embedding(extractor, fallback_samples)?
            }
        };
        let key = next_local_cluster_key + index;
        let total_samples = ranges.iter().map(|(start, end)| end - start).sum();
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

#[cfg(feature = "sherpa-onnx")]
fn assign_global_cluster(
    global_clusters: &mut Vec<GlobalSpeakerClusterState>,
    local_cluster: &LocalSpeakerCluster,
) -> usize {
    let best_match = global_clusters
        .iter()
        .map(|cluster| {
            (
                cluster.id,
                cosine_similarity(&cluster.representative_embedding, &local_cluster.embedding),
            )
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    if let Some((cluster_id, score)) = best_match {
        if score >= CROSS_CHUNK_CLUSTER_SIMILARITY_THRESHOLD {
            if let Some(cluster) = global_clusters
                .iter_mut()
                .find(|cluster| cluster.id == cluster_id)
            {
                cluster.ranges.extend(local_cluster.ranges.iter().copied());
                blend_embeddings(
                    &mut cluster.representative_embedding,
                    cluster.representative_weight,
                    &local_cluster.embedding,
                    local_cluster.total_samples,
                );
                cluster.representative_weight += local_cluster.total_samples;
            }
            return cluster_id;
        }
    }

    let cluster_id = global_clusters.len();
    global_clusters.push(GlobalSpeakerClusterState {
        id: cluster_id,
        ranges: local_cluster.ranges.clone(),
        representative_embedding: local_cluster.embedding.clone(),
        representative_weight: local_cluster.total_samples,
    });
    cluster_id
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
            .unwrap_or(DEFAULT_CLUSTERING_THRESHOLD as f64) as f32,
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
    Ok(SherpaModelSelection {
        model_id,
        segmentation_model_path: install_dir.join(SEGMENTATION_MODEL_RELATIVE_PATH),
        embedding_model_path: install_dir.join(EMBEDDING_MODEL_RELATIVE_PATH),
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
    request
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
            let confidence = if score >= 0.65 {
                crate::RecognitionConfidence::High
            } else if score >= 0.50 {
                crate::RecognitionConfidence::Medium
            } else {
                crate::RecognitionConfidence::Low
            };
            Some(SpeakerRecognitionSuggestion {
                person_id: person.person_id,
                display_name: person.display_name.clone(),
                confidence,
                score,
            })
        })
        .max_by(|left, right| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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

    #[test]
    fn resolves_model_paths_from_app_model_store() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SHERPA_ONNX_PROVIDER_ID,
            Some(DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-a",
            7,
        );

        let selection = resolve_model_selection(&request, Path::new("/tmp/models")).expect("model");

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

    #[test]
    fn recognition_skips_weak_best_match() {
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
            embedding: f32_embedding_to_le_bytes(&[1.0, 0.0]),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });

        let suggestion = best_enrollment_match(&request, &[0.0, 1.0], DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert!(suggestion.is_none());
    }

    #[test]
    fn recognition_skips_person_with_similar_rejection() {
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
            embedding: f32_embedding_to_le_bytes(&[1.0, 0.0]),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });
        request.rejected_people.push(PersonRecognitionRejection {
            person_id: 1,
            embedding: f32_embedding_to_le_bytes(&[1.0, 0.0]),
            embedding_model_id: DEFAULT_SHERPA_ONNX_MODEL_ID.to_string(),
        });

        let suggestion = best_enrollment_match(&request, &[1.0, 0.0], DEFAULT_SHERPA_ONNX_MODEL_ID);

        assert!(suggestion.is_none());
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
    #[test]
    fn assigns_similar_local_clusters_to_the_same_global_cluster() {
        let mut global_clusters = Vec::new();
        let first = LocalSpeakerCluster {
            key: 1,
            ranges: vec![(0, 100)],
            embedding: vec![1.0, 0.0],
            total_samples: 100,
        };
        let second = LocalSpeakerCluster {
            key: 2,
            ranges: vec![(100, 200)],
            embedding: vec![0.95, 0.05],
            total_samples: 100,
        };
        let third = LocalSpeakerCluster {
            key: 3,
            ranges: vec![(200, 300)],
            embedding: vec![0.0, 1.0],
            total_samples: 100,
        };

        let first_id = assign_global_cluster(&mut global_clusters, &first);
        let second_id = assign_global_cluster(&mut global_clusters, &second);
        let third_id = assign_global_cluster(&mut global_clusters, &third);

        assert_eq!(first_id, second_id);
        assert_ne!(first_id, third_id);
        assert_eq!(global_clusters.len(), 2);
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
}
