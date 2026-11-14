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

/// A bare `cargo run` binary has no bundle `Info.plist` to name its icon, so set it by hand.
pub fn set_dev_icon() {
    #[cfg(debug_assertions)]
    dev_icon::set();
}

#[cfg(debug_assertions)]
mod dev_icon {
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::NSString;

    pub(super) const ICON: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../desktop/src-tauri/icons/icon.icns"
    );

    pub(super) fn set() {
        let (Some(bundle), Some(app)) =
            (AnyClass::get(c"NSBundle"), AnyClass::get(c"NSApplication"))
        else {
            return;
        };
        // SAFETY: documented NSBundle / NSApplication messages; nil answers are `None`.
        unsafe {
            let main: Option<Retained<AnyObject>> = msg_send![bundle, mainBundle];
            let id: Option<Retained<AnyObject>> =
                main.and_then(|m| msg_send![&*m, bundleIdentifier]);
            if id.is_some() {
                return;
            }
            let Some(image) = load() else { return };
            let app: Option<Retained<AnyObject>> = msg_send![app, sharedApplication];
            if let Some(app) = app {
                let _: () = msg_send![&*app, setApplicationIconImage: &*image];
            }
        }
    }

    /// The icon as a valid `NSImage`, or `None` when it is missing or unreadable.
    pub(super) fn load() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"NSImage")?;
        // SAFETY: alloc + initWithContentsOfFile: hands back +1 (or nil), owned by `Retained`.
        unsafe {
            let alloc: Allocated<AnyObject> = msg_send![class, alloc];
            let image: Option<Retained<AnyObject>> =
                msg_send![alloc, initWithContentsOfFile: &*NSString::from_str(ICON)];
            let image = image?;
            let valid: Bool = msg_send![&*image, isValid];
            valid.as_bool().then_some(image)
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn dock_dev_icon_loads() {
            assert!(
                super::load().is_some(),
                "missing or unreadable: {}",
                super::ICON
            );
        }
    }
}
