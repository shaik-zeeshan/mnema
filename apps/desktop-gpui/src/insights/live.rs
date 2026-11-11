//! Insights' live follow-ups: the minute beat while recording, and the
//! refreshes when the derived context or the engine's settings change
//! underneath the page.

use gpui_kit::{App, Context};

use super::{stats, Insights, Tab, G_STATUS};
use crate::ui::{calendar::Unit, clock};

impl Insights {
    /// The shell's 60 s beat while Insights shows (`Overview.svelte`,
    /// `DayTimeline.svelte`): new capture moves tracked time without any event.
    /// While recording, Overview re-reads a period that includes now; the
    /// Journal re-reads today unless recording is paused. Neither re-reads the
    /// digest (a model call). `true` = it re-read.
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
            Tab::Journal if !paused && today => self.load(
                G_STATUS,
                cx,
                |b| b.status(),
                |this, s, cx| {
                    this.status = s;
                    this.load_journal(cx);
                },
            ),
            _ => return false,
        }
        true
    }

    /// `loadEngineStatus` alone: which notice shows, and the tab counts.
    pub fn reload_status(&mut self, cx: &mut Context<Self>) {
        self.write(cx, |b| b.status(), |this, s, _| this.status = s);
    }
}

/// `user_context_changed` (a derivation pass, a dismissal elsewhere, a wipe)
/// re-reads the open page; a Settings save (`recording_settings_changed`: the
/// engine on/off, the default model) and a ChatGPT sign-in change which engine
/// notice shows, so they re-read the status (`+page.svelte`).
pub fn on_event(event: &app_core::CoreEvent, cx: &mut App) {
    use app_core::CoreEvent as E;
    let full = match event {
        E::UserContextChanged => true,
        E::RecordingSettingsChanged(_) | E::ChatgptLoginUpdate(_) => false,
        _ => return,
    };
    if let Some(shell) = crate::window::shell(cx) {
        let insights = shell.read(cx).insights.clone();
        insights.update(cx, |i, cx| {
            if full && i.tab == Tab::Subjects && i.subject.is_none() && i.subjects.is_some() {
                // Never reflow the list under the reader.
                i.reload_status(cx);
                i.refresh_subjects(cx);
            } else if full {
                i.reload(cx)
            } else {
                i.reload_status(cx)
            }
        });
    }
}
