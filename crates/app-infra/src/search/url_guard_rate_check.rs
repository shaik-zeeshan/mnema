//! Micro-benchmark for the projection-path claim in `projection.rs`: "Measured at
//! 2.3us/call release ... frames with no browser_url short-circuit for free".
//!
//! ```text
//! cargo test -p app-infra --release --lib url_guard_rate -- --ignored --nocapture
//! ```

use std::time::Instant;

const ITERATIONS: u32 = 20_000;

fn bench(label: &str, url: &str) -> f64 {
    // Warm the process-wide `Lazy` detectors before timing.
    let _ = crate::guard_browser_url(url);
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(crate::guard_browser_url(std::hint::black_box(url)));
    }
    let per_call = started.elapsed().as_secs_f64() * 1e6 / f64::from(ITERATIONS);
    println!("{label}: {per_call:.2} us/call (path len {})", url.len());
    per_call
}

#[test]
#[ignore = "timing check; run with --release --ignored"]
fn url_guard_cost_on_the_capture_rate_projection_path() {
    let typical = bench(
        "typical browser url",
        "https://github.com/mnema/mnema/pull/189/files?diff=split#r12345",
    );
    let deep = bench(
        "deep path (32 segments)",
        &format!("https://example.com/{}", vec!["segment"; 32].join("/")),
    );
    let capped = bench(
        "8KB path (the guard's own cap)",
        &format!("https://example.com/{}", "a".repeat(9_000)),
    );

    // A `None` browser_url never reaches the guard at all (`Option::and_then`),
    // so the only rate that matters is "frames that carry a url".
    println!(
        "at 1 projected document/sec the worst of these costs {:.4}% of one core",
        capped.max(deep).max(typical) / 10_000.0
    );

    assert!(
        typical < 25.0,
        "guarding a typical url on the capture-rate projection path costs {typical:.2} us/call"
    );
}
