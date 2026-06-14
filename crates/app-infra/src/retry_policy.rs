/// Bounded failure-retry policy: how many genuine failures are tolerated before a unit of work
/// is left permanently failed, and the per-attempt backoff before it becomes eligible again.
///
/// Used by both the processing-job queue (`processing/store.rs`) and the user-context
/// derivation worker (`user_context/worker.rs`) so retry semantics are expressed in one place.
pub struct RetryPolicy {
    /// Maximum number of genuine failures before the work item is left terminally failed.
    pub max_attempts: i64,
    /// Per-attempt backoff in seconds, indexed by failure count (0-based). Saturates at the last
    /// entry — a single-element slice gives flat backoff for all failure counts.
    pub backoff_seconds: &'static [i64],
}

impl RetryPolicy {
    /// Backoff (seconds) before the next attempt given how many failures have already been
    /// recorded. Saturates at the last configured step.
    pub fn backoff_seconds(&self, failures_recorded: i64) -> i64 {
        let index = failures_recorded.max(1).saturating_sub(1) as usize;
        self.backoff_seconds
            .get(index)
            .copied()
            .or_else(|| self.backoff_seconds.last().copied())
            .unwrap_or(0)
    }

    /// Whether the failure count has hit or exceeded the cap.
    pub fn is_exhausted(&self, failure_count: i64) -> bool {
        failure_count >= self.max_attempts
    }
}
