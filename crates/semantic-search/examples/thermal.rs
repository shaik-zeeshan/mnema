//! Thermal / power harness for the **shipped** candle-Metal embedding path.
//!
//! Reproduces the backfill worker's loop shape from
//! `apps/desktop/src-tauri/src/semantic_search_worker.rs` (batch of
//! `SWEEP_BATCH_SIZE` anchors, then `clamp(multiplier * work, 150ms, 2000ms)`)
//! so the duty cycle and its GPU cost can be measured on the real path rather
//! than the Python/torch-MPS proxy in `scripts/semantic_bench/duty_cycle.py`,
//! whose absolute watts do not transfer.
//!
//! Sample power alongside it with `macmon pipe -i 500` (sudoless) and join on
//! wall-clock — this harness deliberately does no sampling of its own.
//!
//! ponytail: throwaway diagnostic for the #190 heat question, not shipped
//! behaviour. Delete once the thermal cases are settled.
//!
//! ```sh
//! cargo run --release -p semantic-search --features metal --example thermal -- \
//!     --mode duty --multiplier 1.0 --workers 1 --seconds 120
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use semantic_search::{
    model_install_dir, resolve_descriptor, semantic_search_models_dir, EmbedKind,
    SemanticSearchEmbedder,
};

/// Mirrors `SWEEP_BATCH_SIZE` in the worker.
const SWEEP_BATCH_SIZE: usize = 16;
/// Mirrors `BACKFILL_BATCH_COOLDOWN_MIN` / `_MAX` in the worker (retuned: MAX is a
/// safety bound now, the multiplier governs).
const COOLDOWN_MIN: Duration = Duration::from_millis(150);
const COOLDOWN_MAX: Duration = Duration::from_secs(30);

struct Args {
    mode: String,
    multiplier: f64,
    workers: usize,
    seconds: u64,
    batch: usize,
    corpus: Option<String>,
    /// Override for `BACKFILL_BATCH_COOLDOWN_MAX`. The measured multiplier sweep
    /// showed the multiplier is nearly inert because this cap binds first, so the
    /// cap is the knob worth sweeping.
    cooldown_max_s: f64,
    synth_chars: usize,
    synth_count: usize,
    provider: String,
    model_id: String,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: "duty".to_string(),
            multiplier: 3.0,
            workers: 1,
            seconds: 90,
            batch: SWEEP_BATCH_SIZE,
            corpus: None,
            cooldown_max_s: COOLDOWN_MAX.as_secs_f64(),
            // ~2800 chars ≈ ~700 tokens ≈ 2.9 chunks at the 256-token window,
            // matching the measured chunks-per-anchor on real capture text.
            synth_chars: 2800,
            synth_count: 512,
            provider: "local".to_string(),
            model_id: "nomic-embed-text-v1.5".to_string(),
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < raw.len() {
        let key = raw[index].as_str();
        let mut value = || {
            index += 1;
            raw.get(index).cloned().unwrap_or_else(|| {
                eprintln!("missing value for {key}");
                std::process::exit(2);
            })
        };
        match key {
            "--mode" => args.mode = value(),
            "--multiplier" => args.multiplier = value().parse().expect("multiplier"),
            "--workers" => args.workers = value().parse().expect("workers"),
            "--seconds" => args.seconds = value().parse().expect("seconds"),
            "--batch" => args.batch = value().parse().expect("batch"),
            "--corpus" => args.corpus = Some(value()),
            "--cooldown-max" => args.cooldown_max_s = value().parse().expect("cooldown-max"),
            "--synth-chars" => args.synth_chars = value().parse().expect("synth-chars"),
            "--synth-count" => args.synth_count = value().parse().expect("synth-count"),
            "--provider" => args.provider = value(),
            "--model-id" => args.model_id = value(),
            other => {
                eprintln!("unknown flag {other}");
                std::process::exit(2);
            }
        }
        index += 1;
    }
    args
}

/// Synthetic anchor text sized to a target character count. Every text is salted
/// with its index so no two anchors are byte-identical (a real backlog never
/// repeats, and identical inputs could hit tokenizer/allocator fast paths).
fn synth_texts(count: usize, chars: usize) -> Vec<String> {
    const FILLER: &str = "The quarterly review meeting covered pipeline health, staffing \
        constraints and the migration timeline for the reporting service. Action items were \
        assigned to the platform team with a follow up scheduled for the following sprint. \
        Notes captured from the shared document include budget revisions and the updated \
        risk register entries discussed at length by the group. ";
    (0..count)
        .map(|index| {
            let mut text = format!("anchor {index} :: ");
            while text.len() < chars {
                text.push_str(FILLER);
            }
            text.truncate(chars);
            text
        })
        .collect()
}

/// Anchor text from a JSONL corpus (one object per line with a `text` field) —
/// the same shape `scripts/semantic_bench/` uses. Parsed with a minimal scan
/// rather than pulling serde_json's full machinery into an example.
fn corpus_texts(path: &str) -> Vec<String> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|error| {
        eprintln!("could not read corpus {path}: {error}");
        std::process::exit(2);
    });
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            value
                .get("text")?
                .as_str()
                .map(|text| text.to_string())
                .filter(|text| !text.trim().is_empty())
        })
        .collect()
}

struct Pass {
    work_seconds: f64,
    cooldown_seconds: f64,
    ok: usize,
    failed: usize,
}

fn main() {
    let args = parse_args();
    // `--mode peak` = back-to-back forwards with no rest, isolating the GPU's
    // saturated draw from the worker's pacing. Any other mode runs the worker's
    // real clamp so the measured duty is the shipped one.
    let peak = args.mode == "peak";

    let app_data_dir = dirs_app_data();
    let models_dir = semantic_search_models_dir(&app_data_dir);
    let descriptor = resolve_descriptor(&args.provider, &args.model_id).unwrap_or_else(|| {
        eprintln!("unknown model {}/{}", args.provider, args.model_id);
        std::process::exit(2);
    });
    let model_dir =
        model_install_dir(&models_dir, &args.provider, &args.model_id).expect("model install dir");
    if !model_dir.is_dir() {
        eprintln!("model not installed at {}", model_dir.display());
        std::process::exit(2);
    }

    let texts = match &args.corpus {
        Some(path) => corpus_texts(path),
        None => synth_texts(args.synth_count, args.synth_chars),
    };
    assert!(texts.len() > args.batch, "corpus smaller than one batch");
    let mean_chars = texts.iter().map(|t| t.len()).sum::<usize>() / texts.len();

    eprintln!(
        "mode={} multiplier={} workers={} seconds={} batch={} texts={} mean_chars={} source={}",
        args.mode,
        args.multiplier,
        args.workers,
        args.seconds,
        args.batch,
        texts.len(),
        mean_chars,
        args.corpus.as_deref().unwrap_or("synthetic"),
    );

    // One embedder PER worker, matching the app: the backfill worker, the
    // subject-vector worker and the query cache each hold their own instance, so
    // a shared `Arc` would serialise on the backend mutex and understate the
    // concurrent case (Case 3).
    let texts = Arc::new(texts);
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    let mut handles = Vec::new();
    for worker in 0..args.workers {
        let (texts, stop, barrier) = (texts.clone(), stop.clone(), barrier.clone());
        let (model_dir, descriptor) = (model_dir.clone(), descriptor.clone());
        let (batch, multiplier) = (args.batch, args.multiplier);
        let cooldown_max = Duration::from_secs_f64(args.cooldown_max_s);
        handles.push(std::thread::spawn(move || {
            let load_started = Instant::now();
            let embedder = SemanticSearchEmbedder::load_from_dir(&model_dir, &descriptor)
                .expect("load embedder");
            let load_seconds = load_started.elapsed().as_secs_f64();
            // One unmeasured pass: the first Metal forward pays shader
            // compilation, which is not the steady state the worker lives in.
            let warmup: Vec<&str> = texts[..batch].iter().map(String::as_str).collect();
            let _ = embedder.embed_texts(&warmup, EmbedKind::Document);

            barrier.wait();
            let mut passes: Vec<Pass> = Vec::new();
            let mut cursor = worker * batch;
            while !stop.load(Ordering::Relaxed) {
                let start = cursor % (texts.len() - batch);
                let slice: Vec<&str> = texts[start..start + batch]
                    .iter()
                    .map(String::as_str)
                    .collect();
                let began = Instant::now();
                let results = embedder.embed_texts(&slice, EmbedKind::Document);
                let work = began.elapsed();
                let failed = results.iter().filter(|r| r.is_err()).count();
                let cooldown = if peak {
                    Duration::ZERO
                } else {
                    work.mul_f64(multiplier).clamp(COOLDOWN_MIN, cooldown_max)
                };
                passes.push(Pass {
                    work_seconds: work.as_secs_f64(),
                    cooldown_seconds: cooldown.as_secs_f64(),
                    ok: results.len() - failed,
                    failed,
                });
                if !cooldown.is_zero() {
                    std::thread::sleep(cooldown);
                }
                cursor += batch;
            }
            (load_seconds, passes)
        }));
    }

    barrier.wait();
    let run_started = Instant::now();
    std::thread::sleep(Duration::from_secs(args.seconds));
    stop.store(true, Ordering::Relaxed);
    let wall = run_started.elapsed().as_secs_f64();

    let mut report = Vec::new();
    for handle in handles {
        let (load_seconds, passes) = handle.join().expect("worker panicked");
        let work: f64 = passes.iter().map(|p| p.work_seconds).sum();
        let cooldown: f64 = passes.iter().map(|p| p.cooldown_seconds).sum();
        let ok: usize = passes.iter().map(|p| p.ok).sum();
        let failed: usize = passes.iter().map(|p| p.failed).sum();
        let mut per_pass: Vec<f64> = passes.iter().map(|p| p.work_seconds).collect();
        per_pass.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = per_pass.get(per_pass.len() / 2).copied().unwrap_or(0.0);
        report.push(serde_json::json!({
            "load_seconds": round(load_seconds, 2),
            "passes": passes.len(),
            "anchors_ok": ok,
            "anchors_failed": failed,
            "work_seconds": round(work, 2),
            "cooldown_seconds": round(cooldown, 2),
            // Duty against the harness wall clock, so a worker that spent time
            // blocked behind a sibling on the GPU is not flattered.
            "duty_ratio": round(work / wall, 3),
            "pass_p50_seconds": round(p50, 3),
            "anchors_per_sec_active": round(ok as f64 / work, 2),
            "anchors_per_sec_wall": round(ok as f64 / wall, 2),
        }));
    }

    let total_duty: f64 = report
        .iter()
        .map(|w| w["duty_ratio"].as_f64().unwrap_or(0.0))
        .sum();
    let summary = serde_json::json!({
        "mode": args.mode,
        "multiplier": args.multiplier,
        "cooldown_max_s": args.cooldown_max_s,
        "workers": args.workers,
        "batch": args.batch,
        "wall_seconds": round(wall, 2),
        "text_source": args.corpus.as_deref().unwrap_or("synthetic"),
        "mean_chars_per_anchor": mean_chars,
        "model": format!("{}/{}", args.provider, args.model_id),
        // Sum across workers: >1.0 means the GPU was oversubscribed.
        "summed_duty_ratio": round(total_duty, 3),
        "per_worker": report,
    });
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}

fn round(value: f64, places: u32) -> f64 {
    let scale = 10_f64.powi(places as i32);
    (value * scale).round() / scale
}

/// `~/Library/Application Support/day.mnema` — where the desktop app installs
/// **Semantic Search Model** weights.
fn dirs_app_data() -> std::path::PathBuf {
    let home = std::env::var("HOME").expect("HOME");
    std::path::PathBuf::from(home)
        .join("Library/Application Support")
        .join(std::env::var("MNEMA_APP_ID").unwrap_or_else(|_| "day.mnema".to_string()))
}
