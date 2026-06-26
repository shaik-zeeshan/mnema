//! speakrs on-device diarization provider (Slice 2 + 3).
//!
//! speakrs is a pure-Rust pyannote community-1 pipeline run on a derived
//! Execution Backend (CoreML on macOS; CPU or — opt-in — CUDA on Windows; see
//! [`create_pipeline_for_backend`]), orthogonal to identity (ADR 0004/0005).
//! macOS chooses the backend at compile time; Windows chooses at RUNTIME from the
//! live Force-CPU override + GPU-pack presence, attempting `ExecutionMode::Cuda`
//! and init-falling-back to CPU. Segments at or below [`SPEAKRS_SAFE_CHUNK_SECONDS`]
//! are diarized
//! in a single whole-segment pass; longer segments are diarized in sequential
//! safe-chunks through the same pipeline and the per-chunk speaker clusters are
//! stitched back into segment-wide identities by centroid similarity
//! ([`stitch_chunk_mappings`]).
//!
//! Why chunk (measured, not assumed): whole-segment diarization trips a large
//! transient CoreML buffer past ~3min (~5GB physical footprint at 5min); the same
//! work in ≤180s chunks through the SAME pipeline peaks ~1.45GB and is *faster*,
//! because the per-run transients free between calls (the CoreML sessions stay
//! loaded; only the model weights persist). On the VoxConverse DER bench this is
//! accuracy-neutral at the tuned stitch threshold (7.56% vs 7.47% whole-segment).
//! This supersedes the earlier "subprocess exit is the only reclamation boundary,
//! so never chunk" rationale, which the footprint timeline disproved (the peak is
//! an upfront transient that frees mid-run, not retained graphics memory). See
//! ADR 0003.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use async_trait::async_trait;
use serde_json::json;

use crate::providers::shared::{
    add_warning_reason, audio_peak, best_enrollment_match, decode_audio_to_mono_16khz,
    f32_embedding_to_le_bytes, finalize_provenance_counts, mark_overlapping_turns,
    merge_adjacent_turns, speaker_skip_reason, validate_decoded_samples, SAMPLE_RATE_HZ,
};
use crate::providers::speakrs_mapping::{
    map_speakrs_result, plan_chunk_ranges, provider_cluster_id, stitch_chunk_mappings,
    SpeakerClusterCentroid, SpeakrsMapping,
};
use crate::{
    apply_execution_mode_provenance, model_install_dir, safe_path_component, SpeakerAnalysisError,
    SpeakerAnalysisMetadata, SpeakerAnalysisOutput, SpeakerAnalysisProvider, SpeakerAnalysisRequest,
    SpeakerAnalysisResult, SpeakerCluster, SPEAKRS_DEFAULT_MODEL_ID, SPEAKRS_EMBEDDING_MODEL_ID,
    SPEAKRS_PROVIDER_ID,
};

/// `provider_version` stamp; the crate version is pinned in Cargo.toml.
const SPEAKRS_PROVIDER_VERSION: &str = concat!("speakrs/", "0.4");

/// Safe-chunk window: segments longer than this are diarized in sequential chunks
/// of this length (then stitched). Whole-segment diarization spikes a large
/// transient CoreML buffer past ~3min (~5GB at 5min); chunking at 180s caps the
/// peak ~1.45GB and is *faster*, while staying DER-neutral on the VoxConverse
/// bench (7.56% vs 7.47% whole-segment). 180s keeps a max-length 5-minute segment
/// to two chunks (one stitch boundary). Segments at or below this run whole.
const SPEAKRS_SAFE_CHUNK_SECONDS: usize = 180;

/// Minimum trailing-chunk length: a final chunk shorter than this is folded into
/// the previous one so every `pipeline.run` gets at least a few segmentation
/// windows (the segmentation window is 10s).
const SPEAKRS_MIN_CHUNK_TAIL_SECONDS: usize = 20;

/// Cosine-similarity threshold for stitching per-chunk speaker clusters back into
/// segment-wide identities. Tuned on the VoxConverse bench: 0.5 over-merges
/// distinct speakers (+3.4pp DER), 0.8 over-splits; 0.6 is DER-neutral.
const SPEAKRS_STITCH_SIMILARITY: f32 = 0.6;

/// Execution-time backend inputs threaded from the helper into the blocking
/// entry. Backend is an *execution-time* decision read live at each spawn (ADR
/// 0005), never frozen at admission like provider/model/timeout — so it rides in
/// as a parameter rather than on the request.
///
/// `Default` (`force_cpu = false`, `pack_dir = None`) reproduces the pre-#137
/// behavior exactly: plain CPU on Windows / CoreML on macOS, no CUDA attempt, no
/// fallback diagnostics. Slice 3 populates these from the helper's
/// `--force-cpu` / `--gpu-acceleration-pack-dir` args.
#[derive(Debug, Clone, Default)]
pub struct ExecutionBackendConfig {
    /// The Windows-only "Use GPU acceleration" override is OFF ⇒ cap selection at
    /// CPU (identity-safe; CPU and CUDA share `model_id` / Voiceprint Space /
    /// Continuity keying — it changes speed, never identity).
    pub force_cpu: bool,
    /// The GPU Acceleration Pack dir. CUDA is attempted only when this is set AND
    /// holds the install marker ([`crate::gpu_pack_present`]); `None` ⇒ never
    /// attempt CUDA (plain CPU, no fallback noise — "not provisioned" is not a
    /// failure).
    pub pack_dir: Option<PathBuf>,
}

/// The DEFAULT **Execution Backend** provenance string seeded into a job's base
/// output, so the too-short/silent SKIP path (which never creates a pipeline)
/// still carries an honest `executionMode`. Compile-time per platform: `"coreml"`
/// on macOS, `"cpu"` elsewhere (Windows CPU is the baseline; the full run upgrades
/// it to `"cuda"` — or stamps a fallback — via
/// [`crate::apply_execution_mode_provenance`] once the backend is actually
/// known). Orthogonal to identity (ADR 0004/0005).
fn default_execution_mode_provenance() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "coreml"
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows + the always-type-check fallback target both baseline to CPU.
        "cpu"
    }
}

/// `from_dir` with the crate's typed pipeline-load error. Used for every direct
/// load and for the CPU leg of a CUDA-init fallback.
fn load_pipeline(
    install_dir: &Path,
    mode: speakrs::ExecutionMode,
) -> SpeakerAnalysisResult<speakrs::OwnedDiarizationPipeline> {
    speakrs::OwnedDiarizationPipeline::from_dir(install_dir.to_path_buf(), mode).map_err(|error| {
        SpeakerAnalysisError::Runtime {
            stage: "create_pipeline".to_string(),
            message: format!(
                "failed to load speakrs pipeline from {}: {error}",
                install_dir.display()
            ),
        }
    })
}

/// Create the speakrs pipeline on the chosen **Execution Backend** and report
/// what to stamp into provenance: the backend that ACTUALLY ran, plus — only on a
/// CUDA-init fallback — the reason CUDA initialization failed.
///
/// macOS is compile-time CoreML (unchanged, ADR 0004): the Force-CPU override and
/// the pack are Windows-only (macOS always runs CoreML). Windows is a RUNTIME
/// decision (ADR 0005): [`crate::select_execution_mode`] chooses
/// whether to ATTEMPT `ExecutionMode::Cuda` from the live Force-CPU override +
/// pack presence. On a `from_dir(.., Cuda)` INIT `Err` — speakrs's CUDA arm uses
/// `.error_on_failure()`, so a missing/incompatible CUDA runtime surfaces *here*
/// as `Err`, not a silent CPU run — we re-run on `Cpu` and record the fallback. A
/// CUDA failure AFTER successful init is a later `pipeline.run` error and
/// propagates as a job failure ([`SpeakerAnalysisError::Runtime`]); it never
/// reaches this fallback (CONTEXT.md / ADR 0005).
///
/// The `ExecutionMode::Cuda` reference lives ONLY in the `#[cfg(target_os =
/// "windows")]` arm: the `cuda` speakrs feature is Windows-target-only, and only
/// the Windows build attempts CUDA, so non-Windows builds never name the variant.
fn create_pipeline_for_backend(
    install_dir: &Path,
    exec_config: &ExecutionBackendConfig,
) -> SpeakerAnalysisResult<(speakrs::OwnedDiarizationPipeline, &'static str, Option<String>)> {
    #[cfg(target_os = "macos")]
    {
        // Compile-time CoreML; the override + pack are Windows-only (ADR 0005).
        let _ = exec_config;
        let pipeline = load_pipeline(install_dir, speakrs::ExecutionMode::CoreMl)?;
        Ok((pipeline, "coreml", None))
    }

    #[cfg(target_os = "windows")]
    {
        // CUDA is attempted only when the pack is present AND the user has not
        // forced CPU; pack presence is a filesystem marker check (NVML detection
        // drives the Settings offer, never the attempt — the try/fallback subsumes
        // it). `gpu_detected` is therefore passed as `false` here.
        let pack_present = exec_config
            .pack_dir
            .as_deref()
            .map(crate::gpu_pack_present)
            .unwrap_or(false);
        match crate::select_execution_mode(exec_config.force_cpu, pack_present, false) {
            crate::ExecutionModeSelection::Cpu => {
                // No pack OR Force-CPU: plain CPU, no CUDA attempt, no fallback noise.
                let pipeline = load_pipeline(install_dir, speakrs::ExecutionMode::Cpu)?;
                Ok((pipeline, "cpu", None))
            }
            crate::ExecutionModeSelection::AttemptCuda => {
                // Attempt CUDA. The `cuda` feature (Cargo.toml) + Slice-1
                // load-dynamic ORT + the operator-provided provider DLL + the GPU
                // pack make this real on-device; with any of those missing, init
                // fails and we fall back to CPU below.
                match speakrs::OwnedDiarizationPipeline::from_dir(
                    install_dir.to_path_buf(),
                    speakrs::ExecutionMode::Cuda,
                ) {
                    Ok(pipeline) => Ok((pipeline, "cuda", None)),
                    Err(cuda_error) => {
                        // INIT-only fallback (ADR 0005): re-run on CPU. If CPU ALSO
                        // fails to load, THAT error propagates as a real job failure
                        // (we do not swallow it). The CUDA error becomes the
                        // recorded `cudaFallbackReason` on the successful CPU run.
                        let reason = cuda_error.to_string();
                        let pipeline = load_pipeline(install_dir, speakrs::ExecutionMode::Cpu)?;
                        Ok((pipeline, "cpu", Some(reason)))
                    }
                }
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // speakrs.rs only compiles under the `speakrs` feature (macOS/Windows
        // only), but keep a CPU fallback so the module always type-checks.
        let _ = exec_config;
        let pipeline = load_pipeline(install_dir, speakrs::ExecutionMode::Cpu)?;
        Ok((pipeline, "cpu", None))
    }
}

#[derive(Debug, Clone)]
pub struct SpeakrsSpeakerAnalysisProvider {
    models_dir: PathBuf,
}

impl SpeakrsSpeakerAnalysisProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }
}

#[async_trait]
impl SpeakerAnalysisProvider for SpeakrsSpeakerAnalysisProvider {
    fn provider(&self) -> &'static str {
        SPEAKRS_PROVIDER_ID
    }

    async fn analyze(
        &self,
        request: SpeakerAnalysisRequest,
    ) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
        let models_dir = self.models_dir.clone();
        // The in-process provider has no helper args to source a backend config
        // from, so it runs the default backend (plain CPU on Windows / CoreML on
        // macOS). The CUDA path is driven only through the subprocess helper, which
        // threads a populated `ExecutionBackendConfig` (Slice 3).
        tokio::task::spawn_blocking(move || {
            run_speakrs_blocking(request, &models_dir, &ExecutionBackendConfig::default())
        })
        .await
        .map_err(|error| {
            SpeakerAnalysisError::Analysis(format!("speakrs worker failed to join: {error}"))
        })?
    }
}

/// Blocking entry the Slice 4 subprocess helper calls. Keep this name EXACTLY.
///
/// `exec_config` carries the execution-time **Execution Backend** inputs (live
/// Force-CPU override + GPU-pack dir). Pass `&ExecutionBackendConfig::default()`
/// for the pre-#137 behavior (plain CPU / CoreML).
pub fn analyze_speakrs_request_blocking(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
    exec_config: &ExecutionBackendConfig,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    run_speakrs_blocking(request, models_dir, exec_config)
}

/// Resolve the speakrs model install dir for this request.
///
/// Prefers the manifest descriptor (added in Slice 5). Until that lands, falls
/// back to the same `models_dir/<provider>/<model_id>` layout `model_install_dir`
/// would produce via `safe_path_component`, so this compiles and works before
/// Slice 5. The dir is passed FLAT to `OwnedDiarizationPipeline::from_dir`, which
/// loads `segmentation-3.0.onnx`, `wespeaker-voxceleb-resnet34.onnx`, the PLDA
/// `*.npy` files, and the compiled `*.mlmodelc` bundles directly from it.
fn resolve_install_dir(
    request: &SpeakerAnalysisRequest,
    models_dir: &Path,
) -> SpeakerAnalysisResult<PathBuf> {
    if request.provider != SPEAKRS_PROVIDER_ID {
        return Err(SpeakerAnalysisError::InvalidRequest(format!(
            "speakrs provider received request for {}",
            request.provider
        )));
    }
    let model_id = request
        .model_id
        .clone()
        .unwrap_or_else(|| SPEAKRS_DEFAULT_MODEL_ID.to_string());

    // Prefer the manifest descriptor when present (Slice 5).
    if let Some(descriptor) = crate::find_model_descriptor(
        &crate::builtin_model_manifest(),
        SPEAKRS_PROVIDER_ID,
        Some(model_id.as_str()),
    ) {
        return model_install_dir(models_dir, descriptor)
            .map_err(|error| SpeakerAnalysisError::InvalidRequest(error.to_string()));
    }

    // Pre-Slice-5 fallback: mirror `model_install_dir`'s safe layout by hand.
    let provider_component = safe_path_component("provider", SPEAKRS_PROVIDER_ID)
        .map_err(|error| SpeakerAnalysisError::InvalidRequest(error.to_string()))?;
    let model_component = safe_path_component("modelId", &model_id)
        .map_err(|error| SpeakerAnalysisError::InvalidRequest(error.to_string()))?;
    Ok(models_dir.join(provider_component).join(model_component))
}

fn run_speakrs_blocking(
    request: SpeakerAnalysisRequest,
    models_dir: &Path,
    exec_config: &ExecutionBackendConfig,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    // 1. Validate provider + resolve the install dir.
    let install_dir = resolve_install_dir(&request, models_dir)?;
    let model_id = request
        .model_id
        .clone()
        .unwrap_or_else(|| SPEAKRS_DEFAULT_MODEL_ID.to_string());

    // 2. Confirm the install dir exists (required files are checked by Slice 5's
    //    status detection; here we surface a typed MissingModel so the job fails
    //    cleanly rather than panicking inside the native loader).
    if !install_dir.is_dir() {
        return Err(SpeakerAnalysisError::MissingModel {
            model_kind: "speakrs-bundle".to_string(),
            path: install_dir,
        });
    }

    // 3. Decode whole-segment audio to mono 16k, validate, compute peak/duration.
    let analysis_started = Instant::now();
    let samples = decode_audio_to_mono_16khz(&request.audio_path)?;
    validate_decoded_samples(&samples)?;
    let duration_ms = samples.len() as u64 * 1000 / SAMPLE_RATE_HZ as u64;
    let audio_peak = audio_peak(&samples);

    // 4. Build the base output + provenance (mirrors sherpa's keys for a uniform
    //    downstream). On a too-short/silent skip, return the SUCCESSFUL EMPTY
    //    output (CONTEXT.md invariant), still carrying the Slice 3 keys.
    let mut output = speaker_output_for_request(&request, &install_dir, &model_id, duration_ms, audio_peak);
    if let Some(skip_reason) = speaker_skip_reason(audio_peak, duration_ms) {
        output
            .metadata
            .provenance
            .insert("skipReason".to_string(), json!(skip_reason));
        finalize_provenance_counts(&mut output, analysis_started.elapsed().as_millis() as u64);
        return Ok(output);
    }

    // 5. Create the pipeline on the chosen Execution Backend. macOS is compile-time
    //    CoreML; Windows decides at RUNTIME (live Force-CPU override + GPU-pack
    //    presence → maybe attempt CUDA, init-fall-back to CPU). The backend is
    //    orthogonal to identity and recorded only in provenance. On macOS the
    //    compute units are left at speakrs's default; the GPU-vs-ANE choice was
    //    measured not to affect the memory peak. The returned tuple is the backend
    //    that ACTUALLY ran plus, on a CUDA-init fallback, the reason CUDA failed.
    let (mut pipeline, actual_execution_mode, cuda_fallback_reason) =
        create_pipeline_for_backend(&install_dir, exec_config)?;

    // Stamp the ACTUAL Execution Backend over the seeded default, and — only on a
    // CUDA-init fallback — the `executionModeRequested` + `cudaFallbackReason`
    // diagnostics. A plain CPU run (no pack / Force-CPU) and a successful CUDA or
    // CoreML run get `executionMode` only, no extra keys (ADR 0005). The skip/empty
    // path above never reaches here and keeps the seeded default executionMode.
    apply_execution_mode_provenance(
        &mut output.metadata.provenance,
        actual_execution_mode,
        cuda_fallback_reason.as_deref(),
    );

    // 6. Run + map into the provider-neutral turns/centroids contract. Segments
    //    longer than the safe-chunk window are diarized in sequential chunks through
    //    the SAME pipeline (CoreML sessions stay loaded) and the per-chunk clusters
    //    are stitched back into segment-wide identities; shorter segments — the
    //    common case, since default segments are well under the window — run whole
    //    with no stitch overhead. Chunking bounds the CoreML memory peak (see
    //    SPEAKRS_SAFE_CHUNK_SECONDS) and is DER-neutral with the tuned stitch sim.
    let chunk_samples = SPEAKRS_SAFE_CHUNK_SECONDS * SAMPLE_RATE_HZ as usize;
    let min_tail_samples = SPEAKRS_MIN_CHUNK_TAIL_SECONDS * SAMPLE_RATE_HZ as usize;
    // Plan the safe-chunk ranges with the pure (always-compiled) helper. It
    // rebalances a too-short trailing chunk against the previous one rather than
    // folding into a single >window range, so every chunk stays inside the CoreML
    // memory window (see plan_chunk_ranges / SPEAKRS_SAFE_CHUNK_SECONDS).
    let ranges = plan_chunk_ranges(samples.len(), chunk_samples, min_tail_samples);

    let chunk_count = ranges.len();
    let mapping = if chunk_count <= 1 {
        let result = pipeline
            .run(&samples)
            .map_err(|error| SpeakerAnalysisError::Runtime {
                stage: "diarize".to_string(),
                message: format!("speakrs diarization failed: {error}"),
            })?;
        map_run_result(result)?
    } else {
        let mut chunk_mappings: Vec<(u64, SpeakrsMapping)> = Vec::with_capacity(chunk_count);
        for (start, end) in ranges {
            let result =
                pipeline
                    .run(&samples[start..end])
                    .map_err(|error| SpeakerAnalysisError::Runtime {
                        stage: "diarize".to_string(),
                        message: format!("speakrs diarization failed: {error}"),
                    })?;
            let offset_ms = start as u64 * 1000 / SAMPLE_RATE_HZ as u64;
            chunk_mappings.push((offset_ms, map_run_result(result)?));
        }
        stitch_chunk_mappings(chunk_mappings, SPEAKRS_STITCH_SIMILARITY)
    };

    // Record the actual chunking in provenance (the base output is stamped
    // "single"; override it when the segment was safe-chunked).
    if chunk_count > 1 {
        let provenance = &mut output.metadata.provenance;
        provenance.insert("chunkingMode".to_string(), json!("safe_chunked"));
        provenance.insert("chunkCount".to_string(), json!(chunk_count));
        provenance.insert(
            "safeChunkSeconds".to_string(),
            json!(SPEAKRS_SAFE_CHUNK_SECONDS),
        );
    }

    // 7. Build turns (post-process to match sherpa: merge adjacent same-cluster
    //    turns, then mark cross-cluster overlaps).
    output.turns = mark_overlapping_turns(merge_adjacent_turns(mapping.turns));

    // 8. Build clusters from the centroids, attaching cautious recognition when
    //    requested. The centroid is already mean-pooled + L2-normalized.
    output.clusters = speakrs_clusters_from_centroids(&request, mapping.clusters, &model_id);

    // 9. Finalize provenance counts (turnCount/clusterCount + Slice 3 keys).
    //
    // Clusterless turns are no longer lossy — a turn whose centroid was skipped now
    // keeps a placeholder (empty-embedding) cluster so it survives persistence — but
    // we still surface the condition. A placeholder cluster encodes to an empty
    // embedding byte string, so count those: any > 0 means some turns had no usable
    // centroid. The all-empty case (turns but ZERO usable centroids) still warns,
    // preserving the spirit of the old `speakrs_no_cluster_centroids` reason.
    let placeholder_cluster_count = output
        .clusters
        .iter()
        .filter(|cluster| cluster.embedding.is_empty())
        .count();
    if placeholder_cluster_count > 0 {
        add_warning_reason(&mut output, "speakrs_clusterless_turns");
        output.metadata.provenance.insert(
            "placeholderClusterCount".to_string(),
            json!(placeholder_cluster_count),
        );
    }
    if !output.turns.is_empty() && placeholder_cluster_count == output.clusters.len() {
        // Turns present but EVERY cluster is a placeholder (no usable centroid at
        // all): the all-empty case the old warning flagged. Record it too.
        add_warning_reason(&mut output, "speakrs_no_cluster_centroids");
    }
    finalize_provenance_counts(&mut output, analysis_started.elapsed().as_millis() as u64);

    Ok(output)
}

/// Decompose one speakrs [`speakrs::DiarizationResult`] into the provider-neutral
/// [`SpeakrsMapping`] (turns + per-cluster centroids). No speakrs/ndarray type
/// crosses out of this function; it reads array shape + flat row-major data
/// through the arrays' own public methods so our signatures stay decoupled from
/// speakrs's ndarray version.
fn map_run_result(
    result: speakrs::DiarizationResult,
) -> SpeakerAnalysisResult<SpeakrsMapping> {
    let segments: Vec<(f64, f64, String)> = result
        .segments
        .iter()
        .map(|segment| (segment.start, segment.end, segment.speaker.clone()))
        .collect();

    // Fail loud on an unexpected embedding rank. speakrs always returns an Array3,
    // so this is unreachable today; if it ever changes shape, silently collapsing
    // to (0,0,0) would drop EVERY centroid while still emitting turns (clusterless
    // output). Surfacing a typed error keeps that invariant honest.
    let emb_shape = result.embeddings.0.shape();
    if emb_shape.len() != 3 {
        return Err(SpeakerAnalysisError::Runtime {
            stage: "map_run_result".to_string(),
            message: format!(
                "unexpected speakrs embedding rank {} (expected 3)",
                emb_shape.len()
            ),
        });
    }
    let (chunks, speakers, dim) = (emb_shape[0], emb_shape[1], emb_shape[2]);
    let embeddings: Vec<f32> = match result.embeddings.0.as_slice() {
        Some(slice) => slice.to_vec(),
        None => result.embeddings.0.iter().copied().collect(),
    };
    let hard_clusters: Vec<i32> = match result.hard_clusters.0.as_slice() {
        Some(slice) => slice.to_vec(),
        None => result.hard_clusters.0.iter().copied().collect(),
    };

    Ok(map_speakrs_result(
        &segments,
        chunks,
        speakers,
        dim,
        &embeddings,
        &hard_clusters,
    ))
}

/// Build the [`SpeakerCluster`]s for a speakrs result from the mapped centroids,
/// attaching cautious recognition when `request.recognize_people` is set.
///
/// `voiceprint_model_id` is the preset's Voiceprint Space id — the resolved request
/// `model_id`. That is the value persisted to `recording_speaker_clusters.model_id`,
/// and therefore the `person_voice_embeddings.model_id` that the recognition fetch
/// filters on and `best_enrollment_match` compares against. It is DISTINCT from
/// `SPEAKRS_EMBEDDING_MODEL_ID`, which only labels which embedding model produced the
/// vector (provenance, stamped on `embedding_model_id`). Recognition MUST key on the
/// preset id; keying on the embedding id silently drops every enrollment.
fn speakrs_clusters_from_centroids(
    request: &SpeakerAnalysisRequest,
    centroids: Vec<SpeakerClusterCentroid>,
    voiceprint_model_id: &str,
) -> Vec<SpeakerCluster> {
    centroids
        .into_iter()
        .map(|centroid| {
            let global_id = centroid.global_id;
            let suggestion = if request.recognize_people {
                best_enrollment_match(request, &centroid.embedding, voiceprint_model_id)
            } else {
                None
            };
            SpeakerCluster {
                provider_cluster_id: provider_cluster_id(global_id as i32),
                stable_label: format!("Unknown Speaker {}", global_id + 1),
                embedding: f32_embedding_to_le_bytes(&centroid.embedding),
                embedding_model_id: SPEAKRS_EMBEDDING_MODEL_ID.to_string(),
                suggestion,
            }
        })
        .collect()
}

/// Build the base output + provenance for a speakrs job. Mirrors sherpa's
/// `speaker_output_for_request` provenance keys so downstream is uniform. The
/// `chunkingMode` is stamped `"single"` here and overridden to `"safe_chunked"`
/// by the caller when a long segment was diarized in chunks.
fn speaker_output_for_request(
    request: &SpeakerAnalysisRequest,
    install_dir: &Path,
    model_id: &str,
    duration_ms: u64,
    audio_peak: f32,
) -> SpeakerAnalysisOutput {
    let mut output = SpeakerAnalysisOutput::new(SpeakerAnalysisMetadata::from_request(request));
    output.provider_version = Some(SPEAKRS_PROVIDER_VERSION.to_string());
    let provenance = &mut output.metadata.provenance;
    provenance.insert("schemaVersion".to_string(), json!(1));
    provenance.insert("modelId".to_string(), json!(model_id));
    provenance.insert(
        "modelInstallDir".to_string(),
        json!(install_dir.display().to_string()),
    );
    provenance.insert(
        "embeddingModelId".to_string(),
        json!(SPEAKRS_EMBEDDING_MODEL_ID),
    );
    provenance.insert("audioDurationMs".to_string(), json!(duration_ms));
    provenance.insert("audioPeak".to_string(), json!(audio_peak));
    provenance.insert("skipReason".to_string(), serde_json::Value::Null);
    // Default; overridden to "safe_chunked" when a long segment is chunked.
    provenance.insert("chunkingMode".to_string(), json!("single"));
    // Seed the DEFAULT Execution Backend (provenance-only; CoreML on macOS, CPU
    // elsewhere). The full diarization path overrides this with the backend that
    // actually ran (and any CUDA-fallback diagnostics) via
    // `apply_execution_mode_provenance`; the too-short/silent skip path keeps this
    // honest default. Orthogonal to identity (ADR 0004/0005).
    provenance.insert(
        "executionMode".to_string(),
        json!(default_execution_mode_provenance()),
    );
    provenance.insert("turnCount".to_string(), json!(0));
    provenance.insert("clusterCount".to_string(), json!(0));
    provenance.insert(
        "recognitionEnabled".to_string(),
        json!(request.recognize_people),
    );
    provenance.insert("warningReasons".to_string(), json!(Vec::<String>::new()));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_request_for_other_provider() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            "some_other_provider",
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        let error =
            resolve_install_dir(&request, Path::new("/tmp/models")).expect_err("wrong provider");
        assert!(matches!(error, SpeakerAnalysisError::InvalidRequest(_)));
    }

    #[test]
    fn resolves_fallback_install_dir_before_manifest_descriptor() {
        // Pre-Slice-5: no speakrs descriptor in the manifest, so the install dir
        // is the safe `models_dir/speakrs/<model_id>` layout.
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            None,
            "session-a",
            7,
        );
        let dir = resolve_install_dir(&request, Path::new("/tmp/models")).expect("install dir");
        assert_eq!(
            dir,
            PathBuf::from(format!("/tmp/models/speakrs/{SPEAKRS_DEFAULT_MODEL_ID}"))
        );
    }

    #[test]
    fn missing_install_dir_returns_missing_model() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        let error = run_speakrs_blocking(request, temp.path(), &ExecutionBackendConfig::default())
            .expect_err("missing speakrs bundle should fail");
        assert!(matches!(
            error,
            SpeakerAnalysisError::MissingModel { ref model_kind, .. }
                if model_kind == "speakrs-bundle"
        ));
    }

    #[test]
    fn base_output_carries_uniform_provenance() {
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        let mut output = speaker_output_for_request(
            &request,
            Path::new("/tmp/models/speakrs/x"),
            SPEAKRS_DEFAULT_MODEL_ID,
            500,
            0.0,
        );
        output
            .metadata
            .provenance
            .insert("skipReason".to_string(), json!("too_short"));
        finalize_provenance_counts(&mut output, 5);

        let provenance = &output.metadata.provenance;
        assert_eq!(provenance.get("schemaVersion"), Some(&json!(1)));
        assert_eq!(provenance.get("chunkingMode"), Some(&json!("single")));
        assert_eq!(provenance.get("skipReason"), Some(&json!("too_short")));
        assert_eq!(provenance.get("turnCount"), Some(&json!(0)));
        assert_eq!(provenance.get("clusterCount"), Some(&json!(0)));
        // Slice 3 keys present even on the skip/empty path.
        assert_eq!(provenance.get("clustersPerSegment"), Some(&json!(0)));
        assert_eq!(provenance.get("analysisDurationMs"), Some(&json!(5)));
        assert_eq!(
            output.provider_version.as_deref(),
            Some(SPEAKRS_PROVIDER_VERSION)
        );
    }

    #[test]
    fn base_output_seeds_default_execution_mode() {
        // The base output (which the too-short/silent SKIP path returns as-is)
        // carries an honest default executionMode and NO CUDA-fallback diagnostics.
        // The full diarization path overrides executionMode via
        // `apply_execution_mode_provenance` once the real backend is known.
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        let output = speaker_output_for_request(
            &request,
            Path::new("/tmp/models/speakrs/x"),
            SPEAKRS_DEFAULT_MODEL_ID,
            500,
            0.0,
        );
        let expected = if cfg!(target_os = "macos") { "coreml" } else { "cpu" };
        assert_eq!(
            output.metadata.provenance.get("executionMode"),
            Some(&json!(expected))
        );
        // No fallback diagnostics on the seed/skip path (ADR 0005).
        assert!(!output
            .metadata
            .provenance
            .contains_key("executionModeRequested"));
        assert!(!output
            .metadata
            .provenance
            .contains_key("cudaFallbackReason"));
    }

    #[test]
    fn recognition_keys_on_preset_voiceprint_id_not_embedding_id() {
        // A speakrs cluster is enrolled under the preset's Voiceprint Space id — the
        // value `recording_speaker_clusters.model_id` and `person_voice_embeddings
        // .model_id` store (NOT `SPEAKRS_EMBEDDING_MODEL_ID`). Recognition must
        // compare against that same preset id; keying on the embedding id silently
        // drops every enrollment (the bug this guards against).
        use crate::PersonEnrollment;

        let embedding = vec![1.0_f32, 0.0];
        let mut request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            SPEAKRS_PROVIDER_ID,
            Some(SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-a",
            7,
        );
        request.recognize_people = true;
        request.enrolled_people.push(PersonEnrollment {
            person_id: 42,
            display_name: "Ada".to_string(),
            embedding: f32_embedding_to_le_bytes(&embedding),
            embedding_model_id: SPEAKRS_DEFAULT_MODEL_ID.to_string(),
        });
        let centroids = vec![SpeakerClusterCentroid {
            global_id: 0,
            embedding: embedding.clone(),
        }];

        // Keyed on the preset id (what `run_speakrs_blocking` passes): recognized.
        let clusters =
            speakrs_clusters_from_centroids(&request, centroids.clone(), SPEAKRS_DEFAULT_MODEL_ID);
        let suggestion = clusters[0]
            .suggestion
            .as_ref()
            .expect("enrolled speaker should be recognized when keyed on the preset id");
        assert_eq!(suggestion.person_id, 42);
        // Provenance still labels the embedding model, independent of the match key.
        assert_eq!(clusters[0].embedding_model_id, SPEAKRS_EMBEDDING_MODEL_ID);

        // Keyed on the embedding id (the prior bug): the enrollment is filtered out.
        let regressed =
            speakrs_clusters_from_centroids(&request, centroids, SPEAKRS_EMBEDDING_MODEL_ID);
        assert!(
            regressed[0].suggestion.is_none(),
            "keying recognition on the embedding id must not match preset-id enrollments"
        );
    }
}
