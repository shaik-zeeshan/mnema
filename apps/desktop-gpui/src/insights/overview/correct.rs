//! Overview's corrections (`correct` in `Overview.svelte`): tag an activity's
//! category (Needs attention, the category modal's adjust) or its focus. The
//! row is busy while it saves, the change shows once it lands, and a failure
//! says so in Svelte's alert ("Couldn't update category") rather than a no-op.

use std::collections::HashSet;

use capture_types::{ActivityCategory, FocusLevel};
use gpui_kit::Context;

use crate::insights::Insights;

/// Overview's own view state beyond the loaded data.
#[derive(Default)]
pub struct Ui {
    /// Activities whose correction is saving.
    pub correcting: HashSet<i64>,
    /// Needs attention shows every untagged activity, not the first five.
    pub attn_open: bool,
    /// The category modal's expanded threads.
    pub expanded: Vec<Option<ActivityCategory>>,
    /// The category modal row whose "adjust" popover is open.
    pub adjusting: Option<i64>,
    /// App name → icon PNG (`resolve_app_icons`), and the names already asked.
    pub icons: std::collections::HashMap<String, std::path::PathBuf>,
    pub icons_asked: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fix {
    Category(Option<ActivityCategory>),
    Focus(Option<FocusLevel>),
}

impl Insights {
    pub fn correct(&mut self, id: i64, fix: Fix, cx: &mut Context<Self>) {
        if !self.ov_ui.correcting.insert(id) {
            return;
        }
        self.write(
            cx,
            move |b| match fix {
                Fix::Category(c) => b.correct_category(id, c),
                Fix::Focus(f) => b.correct_focus(id, f),
            },
            move |this, r, cx| {
                this.ov_ui.correcting.remove(&id);
                match r {
                    Ok(()) => {
                        let acts = this
                            .overview
                            .iter_mut()
                            .flat_map(|o| o.activities.iter_mut());
                        for a in acts.filter(|a| a.id == id) {
                            match fix {
                                Fix::Category(c) => a.category = c,
                                Fix::Focus(f) => a.focus = f,
                            }
                        }
                    }
                    Err(e) => {
                        let field = match fix {
                            Fix::Category(_) => "category",
                            Fix::Focus(_) => "focus",
                        };
                        log::warn!("insights: {field} correction failed");
                        if let Some(core) = crate::chat::core(cx) {
                            core.host().message(app_core::MessageDialog {
                                title: format!("Couldn't update {field}"),
                                message: e,
                                kind: app_core::DialogKind::Error,
                            });
                        }
                    }
                }
            },
        );
        cx.notify();
    }
}
