//! The delayed-commit Dismiss (`dismissUndo.svelte.ts`). Dismissing calls no
//! backend at once: the row collapses to "Dismissed · Undo" and the commit fires
//! `DISMISS_UNDO` later unless undone. Undo just drops the pending entry (no
//! backend call was made). Leaving the view commits everything now: the user did
//! dismiss, and leaving shouldn't quietly undo it. Pure over explicit instants,
//! so `advance_clock` drives it through the view's one timer.

use std::time::{Duration, Instant};

/// `DISMISS_UNDO_MS`.
pub const DISMISS_UNDO: Duration = Duration::from_millis(5000);

#[derive(Default)]
pub struct DelayedDismiss {
    /// `(conclusion id, commit deadline)`, oldest first.
    pending: Vec<(i64, Instant)>,
}

impl DelayedDismiss {
    pub fn has(&self, id: i64) -> bool {
        self.pending.iter().any(|(p, _)| *p == id)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// A second dismiss of the same row keeps its first deadline.
    pub fn start(&mut self, id: i64, now: Instant) {
        if !self.has(id) {
            self.pending.push((id, now + DISMISS_UNDO));
        }
    }

    /// `true` when something was pending.
    pub fn undo(&mut self, id: i64) -> bool {
        let before = self.pending.len();
        self.pending.retain(|(p, _)| *p != id);
        before != self.pending.len()
    }

    /// The ids whose window has closed by `now` (removed; the caller commits them).
    pub fn due(&mut self, now: Instant) -> Vec<i64> {
        let (due, keep) = self.pending.iter().partition(|(_, at)| *at <= now);
        self.pending = keep;
        due.into_iter().map(|(id, _)| id).collect()
    }

    /// Everything pending, now (navigating away).
    pub fn flush(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|(_, at)| *at).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_at_the_window_not_before_and_undo_cancels() {
        let t0 = Instant::now();
        let mut d = DelayedDismiss::default();
        d.start(7, t0);
        d.start(7, t0 + Duration::from_millis(100));
        assert!(d.due(t0 + Duration::from_millis(4999)).is_empty());
        assert_eq!(d.due(t0 + DISMISS_UNDO), vec![7]);
        assert!(!d.has(7));
        d.start(8, t0);
        assert!(d.undo(8));
        assert!(d.due(t0 + Duration::from_secs(9)).is_empty());
        d.start(9, t0);
        assert_eq!(d.flush(), vec![9]);
        assert_eq!(d.next_deadline(), None);
    }
}
