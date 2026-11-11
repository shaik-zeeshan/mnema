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

use gpui_kit::Context;

use super::{subjects, Insights};
use crate::ui::toast::Toast;

impl Insights {
    /// Collapse the row to "Dismissed · Undo" and commit in `DISMISS_UNDO`.
    pub fn dismiss(&mut self, id: i64, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        // Dismissing the open conclusion moves the drawer to its neighbour.
        let flat = subjects::flat_conclusions(self);
        let next = flat.iter().position(|c| c.id == id).and_then(|i| {
            flat.get(i + 1)
                .or(i.checked_sub(1).and_then(|j| flat.get(j)))
        });
        let next = next
            .map(|c| c.id)
            .filter(|_| self.open_conclusion == Some(id));
        self.undo.start(id, now);
        if self.open_conclusion == Some(id) {
            self.open_conclusion = next;
            if let Some(n) = next {
                subjects::reveal_conclusion(self, n);
            }
        }
        let this = cx.entity().downgrade();
        let toast =
            Toast::ok("Dismissed — it won't be re-inferred").action("Undo", true, move |_, cx| {
                this.update(cx, |i, cx| i.undo_dismiss(id, cx)).ok();
            });
        let toast = Toast {
            ttl: DISMISS_UNDO,
            ..toast
        };
        self.undo_toast = Some(self.toasts.update(cx, |t, cx| t.push(toast, cx)));
        self.arm_undo(cx);
        cx.notify();
    }

    pub fn undo_dismiss(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.undo.undo(id) {
            if let Some(key) = self.undo_toast.take() {
                self.toasts.update(cx, |t, cx| t.dismiss(key, cx));
            }
            self.open_conclusion = Some(id);
            subjects::reveal_conclusion(self, id);
            self.arm_undo(cx);
            cx.notify();
        }
    }

    /// One timer for the earliest pending deadline.
    fn arm_undo(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.undo.next_deadline() else {
            self.undo_timer = None;
            return;
        };
        let wait = at.saturating_duration_since(cx.background_executor().now());
        self.undo_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                let due = this.undo.due(cx.background_executor().now());
                this.commit_dismiss(due, cx);
                this.arm_undo(cx);
            })
            .ok();
        }));
    }

    pub(super) fn commit_dismiss(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        for id in ids {
            self.write(
                cx,
                move |b| b.dismiss(id),
                |this, r, cx| match r {
                    Ok(()) => {
                        if this.subject.is_some() {
                            this.load_subject(cx);
                        }
                    }
                    Err(e) => {
                        log::warn!("insights: dismiss failed");
                        this.toasts.update(cx, |t, cx| {
                            t.push(Toast::danger(format!("Couldn't dismiss — {e}")), cx)
                        });
                    }
                },
            );
        }
    }

    pub fn toggle_pin(&mut self, id: i64, pinned: bool, cx: &mut Context<Self>) {
        self.write(
            cx,
            move |b| b.set_pinned(id, !pinned),
            move |this, r, cx| {
                let toast = match r {
                    Ok(()) if !pinned => Some(Toast::ok("Pinned — it won't fade")),
                    Ok(()) => None,
                    Err(e) => Some(Toast::danger(format!(
                        "Couldn't {} — {e}",
                        if pinned { "unpin" } else { "pin" }
                    ))),
                };
                if let Some(t) = toast {
                    this.toasts.update(cx, |ts, cx| ts.push(t, cx));
                }
                this.load_subject(cx);
            },
        );
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
