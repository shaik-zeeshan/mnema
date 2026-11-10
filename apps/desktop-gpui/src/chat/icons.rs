//! App icons for the tool rows' app chips and the timeline block's chips, as
//! `Timeline.svelte` resolves them: every name not yet asked goes out in one
//! `resolve_app_icons` batch, and each name is asked once (a miss is a fact
//! too: the initial stays). Names are compared trimmed and lower-cased
//! (`canonicalBundleIdForComparison`). An answer is a fact about an app, never
//! about a chat, so one landing after the chat changed is still right.

use std::path::PathBuf;

use capture_types::AnswerBlock;
use gpui_kit::prelude::*;
use gpui_kit::Context;

use super::Chat;
use crate::insights::charts::timeline::app_label;

fn key(app: &str) -> String {
    app.trim().to_lowercase()
}

impl Chat {
    /// Ask once, off the main thread, for every shown app with no icon yet. A
    /// tool row the backend already gave an `app_icon_path` is never asked.
    pub(super) fn request_icons(&mut self, cx: &mut Context<Self>) {
        let mut ask = Vec::new();
        for t in &self.turns {
            let tools = t
                .view
                .tool_activities
                .iter()
                .filter(|a| a.app_icon_path.is_none())
                .filter_map(|a| a.app.clone());
            let lanes = t
                .view
                .blocks
                .iter()
                .flat_map(|b| match b {
                    AnswerBlock::Timeline { items, .. } => items.as_slice(),
                    _ => &[],
                })
                .filter_map(|it| app_label(it.app.as_deref()));
            for app in tools.chain(lanes) {
                let k = key(&app);
                if !k.is_empty() && self.icons_asked.insert(k) {
                    ask.push(app.trim().to_string());
                }
            }
        }
        if ask.is_empty() {
            return;
        }
        let b = self.sources.clone();
        let task = cx.background_spawn(async move { b.app_icons(&ask) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            if got.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                for (app, path) in got {
                    this.icons.insert(key(&app), path);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `app`'s resolved icon, if one has landed.
    pub fn icon_for(&self, app: &str) -> Option<PathBuf> {
        self.icons.get(&key(app)).cloned()
    }
}
