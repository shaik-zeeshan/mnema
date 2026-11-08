//! Throwaway S0 spike: proves each risky platform piece. S5 deletes this module.

pub mod fonts;
pub mod main_view;
pub mod native;
pub mod panel;
pub mod theme;

use std::io::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Call first thing in `main`: cold-start timings are measured from here.
pub fn mark_start() {
    START.get_or_init(Instant::now);
}

pub fn elapsed_ms() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Spike log: stderr, plus `$SPIKE_LOG` when set (a bundle launched by `open` has no stderr).
pub fn log(line: impl AsRef<str>) {
    let line = format!("[spike +{:.1}ms] {}", elapsed_ms(), line.as_ref());
    eprintln!("{line}");
    if let Ok(path) = std::env::var("SPIKE_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}
