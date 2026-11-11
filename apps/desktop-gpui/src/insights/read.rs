//! The read (`loadDigest` / `scheduleDigestLoad` / the Re-read in
//! `Overview.svelte` and `DayTimeline.svelte`): one period's digest, loaded on
//! its own so the page never waits on a model call. A range step drops the old
//! read at once and debounces the new one (`DIGEST_DEBOUNCE`, a fresh range can
//! cost a paid call); a Re-read forces a fresh one past the cache, and a failed
//! Re-read keeps the read already shown (OV-06/07) with its reason beside it.

use capture_types::{DigestAbsentReason, UserContextDigest};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Task};

use super::{overview, stats, Insights, DIGEST_DEBOUNCE};
use crate::ui::calendar::Unit;

/// Which page's read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Of {
    Overview,
    Journal,
}

#[derive(Default)]
pub struct Read {
    pub digest: Option<UserContextDigest>,
    /// Why the command served no read (`None` while loading / on a read).
    pub reason: Option<DigestAbsentReason>,
    /// A plain read is in flight, or waiting out the debounce.
    pub loading: bool,
    /// A Re-read is in flight (`Re-reading…`).
    pub regenerating: bool,
    /// The last failure, as the backend said it.
    pub error: Option<String>,
    gen: u64,
    regen: u64,
    timer: Option<Task<()>>,
}

impl Insights {
    pub fn read_of(&self, of: Of) -> &Read {
        match of {
            Of::Overview => &self.ov_read,
            Of::Journal => &self.day_read,
        }
    }

    fn read_mut(&mut self, of: Of) -> &mut Read {
        match of {
            Of::Overview => &mut self.ov_read,
            Of::Journal => &mut self.day_read,
        }
    }

    /// `(rangeKind, start, end)` of the page's period.
    fn read_range(&self, of: Of, cx: &Context<Self>) -> (&'static str, i64, i64) {
        match of {
            Of::Overview => {
                let (s, e) = self.overview_window(cx);
                (self.period.1.name(), s, e)
            }
            Of::Journal => {
                let (s, e) = stats::window(self.day, Unit::Day, crate::ui::clock::now(cx).offset());
                ("day", s, e)
            }
        }
    }

    /// `loadDigest`: the cached read (a cheap hit for an unchanged range).
    pub fn load_read(&mut self, of: Of, cx: &mut Context<Self>) {
        let ready = self.status.loaded && overview::engine_on(&self.status);
        let r = self.read_mut(of);
        r.timer = None;
        if !ready {
            (r.digest, r.reason, r.loading) = (None, None, false);
            return;
        }
        r.gen += 1;
        let gen = r.gen;
        r.loading = true;
        r.error = None;
        let (kind, s, e) = self.read_range(of, cx);
        self.spawn_read(of, gen, None, move |b| b.digest(kind, s, e, false), cx);
    }

    /// `scheduleDigestLoad`: a range step. The old read goes at once (last
    /// week's lede never sits over this week's cards), any pending Re-read and
    /// its error with it; the new read waits out `DIGEST_DEBOUNCE`.
    pub fn schedule_read(&mut self, of: Of, cx: &mut Context<Self>) {
        let ready = self.status.loaded && overview::engine_on(&self.status);
        let r = self.read_mut(of);
        r.gen += 1;
        (r.digest, r.reason, r.error) = (None, None, None);
        r.regenerating = false;
        r.loading = ready;
        r.timer = ready.then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DIGEST_DEBOUNCE).await;
                let _ = this.update(cx, |this, cx| {
                    this.load_read(of, cx);
                    cx.notify();
                });
            })
        });
        cx.notify();
    }

    /// Re-read (`regenerate_user_context_digest`): past the cache and the
    /// freshness floor. The busy flag has its own sequence, so a plain read
    /// landing meanwhile can't leave the button stuck on "Re-reading…".
    pub fn reread(&mut self, of: Of, cx: &mut Context<Self>) {
        if !overview::engine_on(&self.status) || self.read_of(of).regenerating {
            return;
        }
        let (kind, s, e) = self.read_range(of, cx);
        let r = self.read_mut(of);
        r.timer = None;
        r.gen += 1;
        r.regen += 1;
        let (gen, regen) = (r.gen, r.regen);
        r.regenerating = true;
        r.loading = false;
        r.error = None;
        self.spawn_read(
            of,
            gen,
            Some(regen),
            move |b| b.digest(kind, s, e, true),
            cx,
        );
        cx.notify();
    }

    fn spawn_read(
        &mut self,
        of: Of,
        gen: u64,
        regen: Option<u64>,
        f: impl FnOnce(
                &dyn super::data::Backend,
            ) -> super::data::Res<capture_types::UserContextDigestResult>
            + Send
            + 'static,
        cx: &mut Context<Self>,
    ) {
        let b = self.backend.clone();
        let task = cx.background_spawn(async move { f(&*b) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |this, cx| {
                let r = this.read_mut(of);
                if regen.is_some_and(|q| q == r.regen) {
                    r.regenerating = false;
                }
                if r.gen != gen {
                    return cx.notify(); // the range moved on: stale
                }
                r.loading = false;
                match got {
                    Ok(v) => (r.digest, r.reason) = (v.digest, v.reason),
                    Err(e) => {
                        log::warn!("insights: the read failed ({} chars)", e.len());
                        r.error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// `SENSITIVE_HOLD` (mirrors `digest::DIGEST_SENSITIVE_HOLD`).
pub const SENSITIVE_HOLD: &str = "digest_sensitive_hold";

/// `turnErrorCopy(..).text`: never a raw reason code.
pub fn fail_text(e: &str) -> String {
    crate::chat::turn_error::error_copy(Some(e), None, None).text
}
