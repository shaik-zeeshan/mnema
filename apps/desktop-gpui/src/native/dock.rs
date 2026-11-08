//! The Dock icon shows while any window other than Quick Recall is visible
//! (BUILD-PROMPT §3.4); with nothing open Mnema lives in the menu bar only.
//! `on_reopen` (Dock click or Finder relaunch with nothing visible) opens main.

use std::collections::HashSet;

use gpui_kit::{ActivationPolicy, App, Global, WindowId};

#[derive(Default)]
struct Dock {
    regular: Option<bool>,
    /// Windows that never show the Dock icon (the Quick Recall panel, S10).
    excluded: HashSet<WindowId>,
}

impl Global for Dock {}

/// Keep `window` out of the Dock decision (Quick Recall).
pub fn exclude(window: WindowId, cx: &mut App) {
    cx.default_global::<Dock>().excluded.insert(window);
}

/// Re-decide the activation policy from what is visible. Call after a window
/// shows, hides or closes. Deferred: a caller inside a window update would
/// otherwise read that window as unreachable.
pub fn refresh(cx: &mut App) {
    cx.defer(refresh_now);
}

fn refresh_now(cx: &mut App) {
    let excluded = cx.default_global::<Dock>().excluded.clone();
    let mut visible = false;
    for handle in cx.windows() {
        if excluded.contains(&handle.window_id()) {
            continue;
        }
        visible |= handle
            .update(cx, |_, window, _| super::is_visible(window))
            .unwrap_or(false);
    }
    set_regular(visible, cx);
}

/// `true` = Dock icon + menu bar (Regular), `false` = menu-bar only (Accessory).
pub fn set_regular(regular: bool, cx: &mut App) {
    let dock = cx.default_global::<Dock>();
    if dock.regular == Some(regular) {
        return;
    }
    dock.regular = Some(regular);
    cx.set_activation_policy(if regular {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    });
}
