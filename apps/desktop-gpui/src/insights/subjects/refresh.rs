//! Subjects never reflow under the reader (`Subjects.svelte` `onContextChanged`):
//! a context change re-reads the list after 500 ms, and when the subjects or
//! their order changed while the reader is scrolled down, the new list waits
//! behind a "N views updated · Refresh" pill; back at the top it swaps in on
//! its own. And the filter's 200 ms wait before it re-ranks (`applySearch`).

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Task};

use super::tiers::subject_rows;
use crate::insights::model::SubjectsData;
use crate::insights::{data, Insights, Tab};
use crate::theme::{space, Palette};
use crate::ui::{clock, Button};

/// `debounce(onContextChanged, 500)`.
pub const REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
/// `debounce(applySearch, 200)`.
pub const FILTER_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct Ui {
    /// A fresh list held behind the pill, and how many subjects came or went.
    pub staged: Option<(SubjectsData, usize)>,
    /// The filter text the list ranks by (the field's value, 200 ms later).
    pub applied: String,
    filter_timer: Option<Task<()>>,
    refresh_timer: Option<Task<()>>,
}

/// `subjectsDiff`: `(changed, added + removed)` between two display orders.
pub fn diff(shown: &[String], staged: &[String]) -> (bool, usize) {
    let added = staged.iter().filter(|s| !shown.contains(s)).count();
    let removed = shown.iter().filter(|s| !staged.contains(s)).count();
    let n = added + removed;
    (n > 0 || shown != staged, n)
}

fn order(d: &SubjectsData, now: i64) -> Vec<String> {
    subject_rows(&d.conclusions, &d.histories, now)
        .into_iter()
        .map(|r| r.subject)
        .collect()
}

impl Insights {
    /// The list's top is in view (`atTop`: within 8 px).
    pub fn at_top(&self) -> bool {
        self.scroll.offset().y > px(-8.)
    }

    /// A context change on Subjects: re-read in 500 ms, then apply or stage.
    pub fn refresh_subjects(&mut self, cx: &mut Context<Self>) {
        self.subj_ui.refresh_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.write(cx, data::subjects, |this, next, cx| {
                    if next.error.is_some() {
                        return; // a failed refresh keeps what's shown
                    }
                    let now = clock::now_ms(cx);
                    let shown = this
                        .subjects
                        .as_ref()
                        .map(|d| order(d, now))
                        .unwrap_or_default();
                    let (changed, n) = diff(&shown, &order(&next, now));
                    if changed && !this.at_top() {
                        this.subj_ui.staged = Some((next, n));
                    } else {
                        this.subjects = Some(next);
                        if changed {
                            this.subj_ui.staged = None;
                        }
                    }
                });
            });
        }));
    }

    /// The pill, or the reader scrolling back to the top.
    pub fn apply_staged(&mut self, cx: &mut Context<Self>) {
        if let Some((d, _)) = self.subj_ui.staged.take() {
            self.subjects = Some(d);
            cx.notify();
        }
    }

    /// The filter changed: rank by it 200 ms later; the matches start on page 1.
    pub fn schedule_filter(&mut self, cx: &mut Context<Self>) {
        self.subj_ui.filter_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.subj_ui.applied = this.filter.read(cx).value().trim().to_string();
                this.pages.remove("search");
                cx.notify();
            });
        }));
    }

    /// Clear filter: at once, no wait.
    pub fn clear_filter(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        self.subj_ui.filter_timer = None;
        self.subj_ui.applied.clear();
        self.pages.remove("search");
        cx.notify();
    }
}

/// "N views updated · Refresh", centred under the bar.
pub fn pill(this: &Insights, _: &Palette, cx: &mut Context<Insights>) -> Option<AnyElement> {
    let (_, n) = this
        .subj_ui
        .staged
        .as_ref()
        .filter(|_| this.tab == Tab::Subjects && this.subject.is_none())?;
    let label = if *n > 0 {
        format!(
            "{n} {} updated · Refresh",
            if *n == 1 { "view" } else { "views" }
        )
    } else {
        "Views updated · Refresh".to_string()
    };
    Some(
        div()
            .absolute()
            .top(space::S2)
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                Button::new("insights-subjects-refresh", label)
                    .sm()
                    .icon(IconName::RotateCcw.path())
                    .on_click(cx.listener(|this, _, _, cx| this.apply_staged(cx))),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn diff_counts_membership_and_sees_order() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(super::diff(&s(&["a", "b"]), &s(&["a", "b"])), (false, 0));
        assert_eq!(super::diff(&s(&["a", "b"]), &s(&["b", "a"])), (true, 0));
        assert_eq!(super::diff(&s(&["a", "b"]), &s(&["a", "c"])), (true, 2));
    }
}
