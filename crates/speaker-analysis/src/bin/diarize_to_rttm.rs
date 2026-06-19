//! Diarize a single audio file and emit hypothesis RTTM on stdout.
//!
//! This is the Rust side of the DER benchmark harness (see
//! `scripts/diarization_bench/`). It runs the real sherpa-onnx provider through
//! [`analyze_sherpa_request_blocking`] — the same in-process entry point the
//! desktop app and the existing repro tests use — so the RTTM it emits reflects
//! production diarization behavior, including chunking and agglomerative
//! cross-chunk clustering.
//!
//! stdout is pure RTTM (so it can be piped/redirected); progress and provenance
//! go to stderr.
//!
//! ## Usage
//! ```text
//! cargo run -p speaker-analysis --features sherpa-onnx --release --bin diarize_to_rttm -- \
//!     --audio /path/to/clip.wav --uri clip-0001 [--out hyp.rttm]
//! ```
//!
//! `--models-dir` defaults to the desktop app's installed model store
//! (`~/Library/Application Support/com.shaikzeeshan.mnema/speaker-analysis-models`).

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use speaker_analysis::{
    providers::sherpa_onnx::{analyze_sherpa_request_blocking, dump_pre_clustering_locals},
    SpeakerAnalysisOutput, SpeakerAnalysisRequest, DEFAULT_SHERPA_ONNX_MODEL_ID,
    MODEL_STORE_DIR_NAME, SHERPA_ONNX_PROVIDER_ID,
};

struct Args {
    audio: PathBuf,
    models_dir: PathBuf,
    model_id: String,
    uri: String,
    out: Option<PathBuf>,
    clustering_threshold: Option<f64>,
    cross_chunk_threshold: Option<f64>,
    num_clusters: Option<i64>,
    min_duration_on: Option<f64>,
    min_duration_off: Option<f64>,
    /// EXPERIMENT (chunk-size sweep): override the safe-chunk diarization window
    /// in milliseconds. Additive/opt-in — absent leaves the production default
    /// (60s) untouched. Clamped server-side to <= 60s. See
    /// `scripts/diarization_bench/`.
    safe_chunk_ms: Option<u64>,
    /// PROTOTYPE (NME-SC experiment): when set, write the pre-global-clustering
    /// local-cluster embeddings + pending turns to this JSON path and skip RTTM.
    /// Additive/opt-in — does not affect normal RTTM output. See
    /// `scripts/diarization_bench/nme_sc.py`.
    dump_embeddings: Option<PathBuf>,
}

const USAGE: &str = "diarize_to_rttm — emit hypothesis RTTM for one audio file

USAGE:
  diarize_to_rttm --audio <path> [options]

OPTIONS:
  --audio <path>              Audio file to diarize (required).
  --uri <name>                RTTM URI/recording id (default: audio file stem).
  --models-dir <path>         Speaker-analysis model store (default: app store).
  --model-id <id>             Preset id (default: pyannote-3.0-nemo-titanet-small).
  --out <path>                Write RTTM here instead of stdout.
  --clustering-threshold <f>  Override per-chunk fast-clustering threshold.
  --cross-chunk-threshold <f> Override cross-chunk agglomeration threshold
                              (drives global cluster count on long audio).
  --num-clusters <n>          Force speaker count (-1 = automatic).
  --min-duration-on <s>       pyannote segmentation min-duration-on (seconds).
  --min-duration-off <s>      pyannote segmentation min-duration-off (seconds).
  --safe-chunk-ms <ms>        EXPERIMENT (chunk-size sweep): override the
                              safe-chunk diarization window (default 60000;
                              clamped to <= 60000). Additive/opt-in. See
                              scripts/diarization_bench/.
  --dump-embeddings <path>    PROTOTYPE (NME-SC experiment): write the
                              pre-global-clustering local-cluster embeddings +
                              pending turns to <path> as JSON and exit without
                              emitting RTTM. Additive/opt-in. See
                              scripts/diarization_bench/.
  -h, --help                  Print this help.";

fn default_models_dir() -> PathBuf {
    // Mirrors the path the desktop app installs models to and the existing
    // repro harnesses default to.
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library/Application Support/com.shaikzeeshan.mnema")
        .join(MODEL_STORE_DIR_NAME)
}

fn parse_args() -> Result<Args, String> {
    let mut audio: Option<PathBuf> = None;
    let mut models_dir: Option<PathBuf> = None;
    let mut model_id = DEFAULT_SHERPA_ONNX_MODEL_ID.to_string();
    let mut uri: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut clustering_threshold: Option<f64> = None;
    let mut cross_chunk_threshold: Option<f64> = None;
    let mut num_clusters: Option<i64> = None;
    let mut min_duration_on: Option<f64> = None;
    let mut min_duration_off: Option<f64> = None;
    let mut safe_chunk_ms: Option<u64> = None;
    let mut dump_embeddings: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--audio" => audio = Some(PathBuf::from(value()?)),
            "--models-dir" => models_dir = Some(PathBuf::from(value()?)),
            "--model-id" => model_id = value()?,
            "--uri" => uri = Some(value()?),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--clustering-threshold" => {
                clustering_threshold = Some(value()?.parse().map_err(|e| format!("{e}"))?)
            }
            "--cross-chunk-threshold" => {
                cross_chunk_threshold = Some(value()?.parse().map_err(|e| format!("{e}"))?)
            }
            "--num-clusters" => num_clusters = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--min-duration-on" => {
                min_duration_on = Some(value()?.parse().map_err(|e| format!("{e}"))?)
            }
            "--min-duration-off" => {
                min_duration_off = Some(value()?.parse().map_err(|e| format!("{e}"))?)
            }
            "--safe-chunk-ms" => safe_chunk_ms = Some(value()?.parse().map_err(|e| format!("{e}"))?),
            "--dump-embeddings" => dump_embeddings = Some(PathBuf::from(value()?)),
            other if audio.is_none() && !other.starts_with('-') => {
                audio = Some(PathBuf::from(other))
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let audio = audio.ok_or("--audio is required")?;
    let uri = uri.unwrap_or_else(|| {
        audio
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "audio".to_string())
    });

    Ok(Args {
        audio,
        models_dir: models_dir.unwrap_or_else(default_models_dir),
        model_id,
        uri,
        out,
        clustering_threshold,
        cross_chunk_threshold,
        num_clusters,
        min_duration_on,
        min_duration_off,
        safe_chunk_ms,
        dump_embeddings,
    })
}

/// Render diarization turns as RTTM `SPEAKER` lines.
///
/// Field layout (NIST RT): `SPEAKER <uri> 1 <onset> <dur> <NA> <NA> <spk> <NA> <NA>`.
fn output_to_rttm(output: &SpeakerAnalysisOutput, uri: &str) -> String {
    let mut rttm = String::new();
    for turn in &output.turns {
        if turn.end_ms <= turn.start_ms {
            continue;
        }
        let onset = turn.start_ms as f64 / 1000.0;
        let duration = (turn.end_ms - turn.start_ms) as f64 / 1000.0;
        rttm.push_str(&format!(
            "SPEAKER {uri} 1 {onset:.3} {duration:.3} <NA> <NA> {speaker} <NA> <NA>\n",
            speaker = turn.provider_cluster_id,
        ));
    }
    rttm
}

fn run(args: &Args) -> Result<(), String> {
    if !args.audio.is_file() {
        return Err(format!("audio file not found: {}", args.audio.display()));
    }
    if !args.models_dir.is_dir() {
        return Err(format!(
            "models dir not found: {} (pass --models-dir, or install models via the app)",
            args.models_dir.display()
        ));
    }

    let mut request = SpeakerAnalysisRequest::new(
        &args.audio,
        SHERPA_ONNX_PROVIDER_ID,
        Some(args.model_id.clone()),
        format!("der-bench:{}", args.uri),
        1,
    );
    if let Some(t) = args.clustering_threshold {
        request
            .options
            .insert("clusteringThreshold".to_string(), t.into());
    }
    if let Some(t) = args.cross_chunk_threshold {
        request
            .options
            .insert("crossChunkThreshold".to_string(), t.into());
    }
    if let Some(n) = args.num_clusters {
        request.options.insert("numClusters".to_string(), n.into());
    }
    if let Some(v) = args.min_duration_on {
        request
            .options
            .insert("minDurationOn".to_string(), v.into());
    }
    if let Some(v) = args.min_duration_off {
        request
            .options
            .insert("minDurationOff".to_string(), v.into());
    }
    if let Some(ms) = args.safe_chunk_ms {
        request.options.insert("safeChunkMs".to_string(), ms.into());
    }

    // PROTOTYPE (NME-SC experiment): dump pre-global-clustering local clusters +
    // pending turns as JSON and exit, leaving normal RTTM output untouched.
    if let Some(dump_path) = &args.dump_embeddings {
        let dump = dump_pre_clustering_locals(request, Path::new(&args.models_dir), &args.uri)
            .map_err(|e| format!("embedding dump failed: {e}"))?;
        let json = serde_json::to_string(&dump)
            .map_err(|e| format!("failed to serialize embedding dump: {e}"))?;
        fs::write(dump_path, json)
            .map_err(|e| format!("failed to write {}: {e}", dump_path.display()))?;
        eprintln!(
            "[diarize_to_rttm] uri={} dumped {} local clusters, {} turns (dim={}, chunks={}) -> {}",
            args.uri,
            dump.local_clusters.len(),
            dump.turns.len(),
            dump.embedding_dim,
            dump.chunk_count,
            dump_path.display(),
        );
        return Ok(());
    }

    let output = analyze_sherpa_request_blocking(request, Path::new(&args.models_dir))
        .map_err(|e| format!("diarization failed: {e}"))?;

    let chunking = output
        .metadata
        .provenance
        .get("chunkingMode")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    eprintln!(
        "[diarize_to_rttm] uri={} clusters={} turns={} chunkingMode={} model={}",
        args.uri,
        output.clusters.len(),
        output.turns.len(),
        chunking,
        args.model_id,
    );

    let rttm = output_to_rttm(&output, &args.uri);
    match &args.out {
        Some(path) => {
            fs::write(path, rttm)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            eprintln!("[diarize_to_rttm] wrote {}", path.display());
        }
        None => print!("{rttm}"),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
