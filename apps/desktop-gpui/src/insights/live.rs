//! Insights' live follow-ups: Re-read (a forced digest) and the refresh when
//! the derived context changes underneath the page.

use gpui_kit::{App, Context};

use super::{stats, Insights, Tab, G_OVERVIEW};
use crate::ui::{calendar::Unit, clock};

impl Insights {
    /// The shell's 60 s beat while Insights shows (`Overview.svelte`,
    /// `DayTimeline.svelte`): new capture moves tracked time without any event.
    /// While recording, Overview re-reads a period that includes now; the
    /// Journal re-reads today unless recording is paused. `true` = it re-read.
    pub fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        let (running, paused) = self.recording;
        if !running {
            return false;
        }
        let now = clock::now_ms(cx);
        let current = self.overview_window(cx).1 > now;
        let today = stats::window(self.day, Unit::Day, self.offset(cx)).1 > now;
        match self.tab {
            Tab::Overview if current => self.load_overview(cx),
            Tab::Journal if !paused && today => self.reload(cx),
            _ => return false,
        }
        true
    }

    /// Re-read: write this period's read again now (`regenerate_user_context_digest`,
    /// a model call), then show it or the reason it couldn't be written.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.rereading, true) {
            return;
        }
        let (start, end) = self.overview_window(cx);
        let unit = self.period.1.name();
        self.load(
            G_OVERVIEW,
            cx,
            move |b| b.digest(unit, start, end, true),
            |this, digest, _| {
                if let Err(e) = &digest {
                    log::warn!("insights: re-read failed: {e}");
                }
                this.rereading = false;
                if let Some(o) = this.overview.as_mut() {
                    o.digest = Some(digest);
                }
            },
        );
        cx.notify();
    }
}

/// `UserContextChanged` (a derivation pass, a dismissal elsewhere, a wipe):
/// re-read the open page.
pub fn on_event(event: &app_core::CoreEvent, cx: &mut App) {
    if !matches!(event, app_core::CoreEvent::UserContextChanged) {
        return;
    }
    if let Some(shell) = crate::window::shell(cx) {
        let insights = shell.read(cx).insights.clone();
        insights.update(cx, |i, cx| {
            i.rereading = false;
            i.reload(cx)
        });
    }
}
