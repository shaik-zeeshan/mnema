//! Diarize a single audio file with the **speakrs** provider and emit
//! hypothesis RTTM on stdout.
//!
//! This is the speakrs sibling of `diarize_to_rttm.rs` (the sherpa-onnx bench
//! bin). It exists as a separate `[[bin]]` with `required-features = ["speakrs"]`
//! so the heavy speakrs/OpenBLAS/CoreML build stays opt-in and the default
//! `cargo check`/`cargo test` (no features) stay clean. It runs the real
//! in-process speakrs provider through [`analyze_speakrs_request_blocking`] — the
//! same entry the Slice 4 subprocess helper calls — so the RTTM it emits reflects
//! production diarization behavior.
//!
//! It speaks the SAME CLI/stdout/RTTM contract `scripts/diarization_bench/run_der.py`
//! expects for `--binary`: `--audio <path> --uri <name>`, optional `--models-dir`,
//! pure RTTM on stdout (progress/provenance to stderr), and the identical NIST RT
//! `SPEAKER` line layout as the sherpa bin so DER scoring is apples-to-apples.
//!
//! The sherpa-specific tuning flags `run_der.py` may pass (`--model-id`,
//! `--clustering-threshold`, `--cross-chunk-threshold`, `--num-clusters`,
//! `--min-duration-on`, `--min-duration-off`, `--safe-chunk-ms`) are accepted but
//! ignored here (except `--model-id`, which is honored): speakrs uses a single
//! curated preset and its own fixed safe-chunk window (see `providers/speakrs.rs`
//! — segments past ~180s are chunked and stitched internally; the flag does not
//! tune it). Accepting-and-ignoring them keeps `run_der.py`'s shared `extra` args
//! from breaking the speakrs `--binary` path.
//!
//! stdout is pure RTTM (so it can be piped/redirected); progress and provenance
//! go to stderr.
//!
//! ## Usage
//! ```text
//! # OpenBLAS is required to build speakrs:
//! brew install openblas pkgconf
//! export PKG_CONFIG_PATH=$(brew --prefix openblas)/lib/pkgconfig
//!
//! cargo build -p speaker-analysis --features speakrs --release \
//!     --bin diarize_to_rttm_speakrs
//!
//! # Then drive the DER harness at the speakrs binary to reconfirm the VoxConverse
//! # ~8.35% DER win:
//! python scripts/diarization_bench/run_der.py \
//!     --binary target/release/diarize_to_rttm_speakrs --all
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
    providers::speakrs::analyze_speakrs_request_blocking, SpeakerAnalysisOutput,
    SpeakerAnalysisRequest, MODEL_STORE_DIR_NAME, SPEAKRS_DEFAULT_MODEL_ID, SPEAKRS_PROVIDER_ID,
};

struct Args {
    audio: PathBuf,
    models_dir: PathBuf,
    model_id: String,
    uri: String,
    out: Option<PathBuf>,
}

const USAGE: &str = "diarize_to_rttm_speakrs — emit hypothesis RTTM for one audio file (speakrs provider)

USAGE:
  diarize_to_rttm_speakrs --audio <path> [options]

OPTIONS:
  --audio <path>              Audio file to diarize (required).
  --uri <name>                RTTM URI/recording id (default: audio file stem).
  --models-dir <path>         Speaker-analysis model store (default: app store).
  --model-id <id>             Preset id (default: pyannote-community-1-wespeaker).
  --out <path>                Write RTTM here instead of stdout.
  -h, --help                  Print this help.

NOTE: For drop-in parity with run_der.py's shared --binary args, the
sherpa-only tuning flags (--clustering-threshold, --cross-chunk-threshold,
--num-clusters, --min-duration-on, --min-duration-off, --safe-chunk-ms,
--dump-embeddings) are accepted but ignored — speakrs runs with a single
curated preset; segments past 180s are safe-chunked and stitched internally.";

fn default_models_dir() -> PathBuf {
    // Mirrors the path the desktop app installs models to (the shared
    // speaker-analysis model store).
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library/Application Support/com.shaikzeeshan.mnema")
        .join(MODEL_STORE_DIR_NAME)
}

fn parse_args() -> Result<Args, String> {
    let mut audio: Option<PathBuf> = None;
    let mut models_dir: Option<PathBuf> = None;
    let mut model_id = SPEAKRS_DEFAULT_MODEL_ID.to_string();
    let mut uri: Option<String> = None;
    let mut out: Option<PathBuf> = None;

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
            // Accept-and-ignore the sherpa-only tuning flags so the shared
            // `run_der.py --binary` arg list (which is engine-agnostic) does not
            // break the speakrs path. These have no speakrs equivalent.
            "--clustering-threshold"
            | "--cross-chunk-threshold"
            | "--num-clusters"
            | "--min-duration-on"
            | "--min-duration-off"
            | "--safe-chunk-ms"
            | "--dump-embeddings" => {
                let value = value()?;
                eprintln!("[diarize_to_rttm_speakrs] ignoring sherpa-only flag {flag} {value}");
            }
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
    })
}

/// Render diarization turns as RTTM `SPEAKER` lines.
///
/// Field layout (NIST RT): `SPEAKER <uri> 1 <onset> <dur> <NA> <NA> <spk> <NA> <NA>`.
/// Kept byte-for-byte identical to `diarize_to_rttm.rs::output_to_rttm` so the
/// two engines' RTTM is parsed and DER-scored identically by `run_der.py`.
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

    let request = SpeakerAnalysisRequest::new(
        &args.audio,
        SPEAKRS_PROVIDER_ID,
        Some(args.model_id.clone()),
        format!("der-bench:{}", args.uri),
        1,
    );

    let output = analyze_speakrs_request_blocking(request, Path::new(&args.models_dir))
        .map_err(|e| format!("diarization failed: {e}"))?;

    eprintln!(
        "[diarize_to_rttm_speakrs] uri={} clusters={} turns={} model={}",
        args.uri,
        output.clusters.len(),
        output.turns.len(),
        args.model_id,
    );

    let rttm = output_to_rttm(&output, &args.uri);
    match &args.out {
        Some(path) => {
            fs::write(path, rttm)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            eprintln!("[diarize_to_rttm_speakrs] wrote {}", path.display());
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
