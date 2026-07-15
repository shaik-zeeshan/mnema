//! Manual candle-CPU measurement harness (NOT a CI gate, NOT a perf gate).
//!
//! ADR 0037 gates claiming Windows Semantic Search support on a candle-CPU
//! measurement: CPU%, throughput, and RSS from a real build/run of the candle
//! CPU path. This test loads the SAME production wrapper the app uses
//! ([`SemanticSearchEmbedder`] — chunking / length-bucketed sub-batching /
//! cross-chunk mean-pool included, not just the raw backend), embeds a
//! realistic mixed-length document workload plus a handful of queries, and
//! PRINTS the numbers. It asserts only correctness invariants (vector
//! dimension + finiteness) — it is a measurement, not a performance gate.
//!
//! On a build without the `metal` feature (the default, and the only option on
//! Windows) `SemanticSearchEmbedder::load_from_dir` always lands on the CPU
//! device, so running this test WITHOUT `--features metal` measures the CPU
//! path everywhere.
//!
//! It is `#[ignore]` and additionally gated on
//! `MNEMA_SEMANTIC_CPU_MEASURE_MODEL_DIR` (falling back to the parity gate's
//! `MNEMA_SEMANTIC_PARITY_MODEL_DIR`) pointing at an installed nomic model dir
//! (CI lacks the ~550 MB weights), so it skips cleanly when unset. Run it
//! manually, e.g. on Windows PowerShell:
//!
//! ```text
//! $env:MNEMA_SEMANTIC_CPU_MEASURE_MODEL_DIR = "$env:APPDATA\com.shaikzeeshan.mnema\semantic_search_models\local\nomic-embed-text-v1.5"
//! cargo test -p semantic-search --test cpu_measurement -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use semantic_search::{
    resolve_descriptor, EmbedKind, SemanticSearchEmbedder, SEMANTIC_SEARCH_PROVIDER_ID,
};

/// Vocabulary for the deterministic synthetic workload: capture/OCR-flavored
/// words so token statistics roughly resemble real anchor `body_text`.
const WORDS: &[&str] = &[
    "screen", "capture", "semantic", "search", "vector", "anchor", "window", "meeting",
    "transcript", "notes", "browser", "editor", "terminal", "dashboard", "invoice", "quarterly",
    "report", "review", "deploy", "pipeline", "error", "timeout", "retry", "database",
    "migration", "index", "storage", "encrypted", "workspace", "session", "keyboard", "focus",
];

/// Deterministic pseudo-text: `word_count` words picked by a simple index mix,
/// with sentence punctuation every 12 words. No RNG dependency, identical
/// workload on every run/machine so measurements are comparable.
fn synth_text(seed: usize, word_count: usize) -> String {
    let mut out = String::with_capacity(word_count * 8);
    for i in 0..word_count {
        if i > 0 {
            out.push(if i % 12 == 0 { '.' } else { ' ' });
            if i % 12 == 0 {
                out.push(' ');
            }
        }
        out.push_str(WORDS[(seed.wrapping_mul(31).wrapping_add(i.wrapping_mul(17))) % WORDS.len()]);
    }
    out.push('.');
    out
}

/// The mixed-length document workload: 20 short titles, 20 medium paragraphs,
/// 20 long OCR-ish paragraphs (~400 words — these overflow the 256-token embed
/// window, so the wrapper's chunk-split + mean-pool path is exercised too).
fn document_workload() -> Vec<String> {
    let mut docs = Vec::with_capacity(60);
    for seed in 0..20 {
        docs.push(synth_text(seed, 4 + seed % 6)); // short titles: 4-9 words
    }
    for seed in 20..40 {
        docs.push(synth_text(seed, 80 + (seed % 5) * 10)); // medium: 80-120 words
    }
    for seed in 40..60 {
        docs.push(synth_text(seed, 350 + (seed % 4) * 50)); // long: 350-500 words
    }
    docs
}

const QUERIES: &[&str] = &[
    "that invoice from the quarterly review",
    "database migration timeout error",
    "meeting notes about the deploy pipeline",
    "encrypted workspace storage",
    "what did I read about semantic search",
];

// ---------------------------------------------------------------------------
// Process CPU time (user + kernel), for CPU% = cpu_time / wall_time.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn process_cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return None;
    }
    // FILETIME is 100-ns ticks.
    let to_duration = |ft: &FILETIME| {
        let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        Duration::from_nanos(ticks.saturating_mul(100))
    };
    Some(to_duration(&kernel) + to_duration(&user))
}

#[cfg(unix)]
fn process_cpu_time() -> Option<Duration> {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let timeval = |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32) * 1000);
    Some(timeval(usage.ru_utime) + timeval(usage.ru_stime))
}

// ---------------------------------------------------------------------------
// Memory: (current working set, peak working set) in bytes.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn memory_snapshot() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok == 0 {
        return None;
    }
    Some((
        counters.WorkingSetSize as u64,
        counters.PeakWorkingSetSize as u64,
    ))
}

#[cfg(unix)]
fn memory_snapshot() -> Option<(u64, u64)> {
    // getrusage only exposes the PEAK RSS portably (Linux: KB, macOS: bytes);
    // report it for both slots — good enough for a manual measurement.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let raw = usage.ru_maxrss as u64;
    let peak = if cfg!(target_os = "macos") { raw } else { raw.saturating_mul(1024) };
    Some((peak, peak))
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn print_memory(label: &str) {
    match memory_snapshot() {
        Some((working_set, peak)) => println!(
            "  RSS {label}: working set {:.1} MiB, peak working set {:.1} MiB",
            mib(working_set),
            mib(peak)
        ),
        None => println!("  RSS {label}: unavailable on this platform"),
    }
}

/// Correctness invariant (the only assertions in this harness): the vector has
/// the stored dimension and every component is finite.
fn assert_valid_vector(context: &str, vector: &[f32], dimension: usize) {
    assert_eq!(
        vector.len(),
        dimension,
        "{context}: vector dimension must match the embedder's stored dimension"
    );
    assert!(
        vector.iter().all(|v| v.is_finite()),
        "{context}: every vector component must be finite"
    );
}

#[test]
#[ignore = "manual measurement: needs the ~550 MB nomic model; set MNEMA_SEMANTIC_CPU_MEASURE_MODEL_DIR (or MNEMA_SEMANTIC_PARITY_MODEL_DIR) and run with --nocapture"]
fn candle_cpu_measurement() {
    let model_dir = std::env::var("MNEMA_SEMANTIC_CPU_MEASURE_MODEL_DIR")
        .or_else(|_| std::env::var("MNEMA_SEMANTIC_PARITY_MODEL_DIR"));
    let Ok(model_dir) = model_dir else {
        eprintln!(
            "MNEMA_SEMANTIC_CPU_MEASURE_MODEL_DIR / MNEMA_SEMANTIC_PARITY_MODEL_DIR unset; \
             skipping candle-CPU measurement harness"
        );
        return;
    };

    let descriptor = resolve_descriptor(SEMANTIC_SEARCH_PROVIDER_ID, "nomic-embed-text-v1.5")
        .expect("nomic descriptor resolves");

    println!("== candle-CPU Semantic Search measurement (model dir: {model_dir}) ==");
    print_memory("before model load");

    // Model load, through the production wrapper (on a non-`metal` build this
    // is the CPU device).
    let load_started = Instant::now();
    let embedder = SemanticSearchEmbedder::load_from_dir(&model_dir, &descriptor)
        .expect("embedder loads the nomic model");
    let load_elapsed = load_started.elapsed();
    let dimension = embedder.dimension();
    println!("  model load: {:.2}s (stored dimension {dimension})", load_elapsed.as_secs_f64());
    print_memory("after model load");

    let documents = document_workload();
    let document_refs: Vec<&str> = documents.iter().map(String::as_str).collect();
    let total_chars: usize = documents.iter().map(String::len).sum();

    // -- Phase A: batched document throughput (the production backfill shape) --
    let cpu_before = process_cpu_time();
    let wall_started = Instant::now();
    let results = embedder.embed_texts(&document_refs, EmbedKind::Document);
    let wall_elapsed = wall_started.elapsed();
    let cpu_after = process_cpu_time();

    assert_eq!(results.len(), documents.len());
    for (text, result) in documents.iter().zip(&results) {
        let vector = result
            .as_ref()
            .unwrap_or_else(|error| panic!("document embed failed: {error} ({text:.40})"));
        assert_valid_vector("document", vector, dimension);
    }

    let wall_secs = wall_elapsed.as_secs_f64();
    println!(
        "  batched documents: {} texts ({} chars) in {:.2}s = {:.2} texts/s, {:.0} chars/s",
        documents.len(),
        total_chars,
        wall_secs,
        documents.len() as f64 / wall_secs,
        total_chars as f64 / wall_secs,
    );
    match (cpu_before, cpu_after) {
        (Some(before), Some(after)) => {
            let cpu_secs = (after - before).as_secs_f64();
            println!(
                "  CPU during batched embed: {:.2}s CPU / {:.2}s wall = {:.0}% (~{:.1} cores)",
                cpu_secs,
                wall_secs,
                100.0 * cpu_secs / wall_secs,
                cpu_secs / wall_secs,
            );
        }
        _ => println!("  CPU during batched embed: unavailable on this platform"),
    }
    print_memory("after batched documents");

    // -- Phase B: per-text latency distribution (single-text embeds, the query/
    //    interactive shape), over a length-spanning subset --
    let mut latencies: Vec<Duration> = Vec::new();
    for text in documents.iter().step_by(3) {
        let started = Instant::now();
        let vector = embedder
            .embed_text(text, EmbedKind::Document)
            .expect("single document embed");
        latencies.push(started.elapsed());
        assert_valid_vector("single document", &vector, dimension);
    }
    let min = latencies.iter().min().expect("nonempty");
    let max = latencies.iter().max().expect("nonempty");
    let mean = latencies.iter().sum::<Duration>() / latencies.len() as u32;
    println!(
        "  per-text latency ({} single embeds, mixed lengths): min {:.0}ms / mean {:.0}ms / max {:.0}ms",
        latencies.len(),
        min.as_secs_f64() * 1000.0,
        mean.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0,
    );

    // -- Phase C: query-side embeds --
    let query_started = Instant::now();
    for query in QUERIES {
        let vector = embedder
            .embed_text(query, EmbedKind::Query)
            .expect("query embed");
        assert_valid_vector("query", &vector, dimension);
    }
    let query_elapsed = query_started.elapsed();
    println!(
        "  queries: {} embeds in {:.2}s = {:.0}ms/query",
        QUERIES.len(),
        query_elapsed.as_secs_f64(),
        query_elapsed.as_secs_f64() * 1000.0 / QUERIES.len() as f64,
    );

    print_memory("at end");
    match process_cpu_time() {
        Some(total) => println!("  total process CPU time: {:.2}s", total.as_secs_f64()),
        None => println!("  total process CPU time: unavailable on this platform"),
    }
    println!("== measurement complete ==");
}
