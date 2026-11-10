//! A second launch hands over to the running Mnema, as the Tauri app's
//! single-instance plugin does: the new process asks the running one to show
//! its main window (a distributed notification), activates it, and exits. Only
//! when no running instance can be found does it fall back to the alert.

use std::ptr::NonNull;

use block2::RcBlock;
use gpui_kit::App;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};

/// The per-identity request name: a dev bundle never wakes a release one.
fn request_name(bundle_id: &str) -> String {
    format!("{bundle_id}.show-main")
}

/// From the second process: ask the running instance to show main and bring it
/// forward. `false` = no other instance of this bundle id runs (the caller shows
/// the alert instead).
pub fn hand_over(bundle_id: &str) -> bool {
    let me = std::process::id() as i32;
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
        bundle_id,
    ));
    let Some(other) = apps.iter().find(|app| app.processIdentifier() != me) else {
        return false;
    };
    let name = NSString::from_str(&request_name(bundle_id));
    // SAFETY: a plain name, no object or user info; `deliverImmediately` reaches
    // an app that is inactive (AppKit suspends delivery to those otherwise).
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object_userInfo_deliverImmediately(&name, None, None, true);
    }
    other.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    log::info!("another Mnema instance is running; handed this launch over to it");
    true
}

/// In the running instance: a second launch's request shows main (onboarding
/// while it isn't done), like a Dock reopen.
pub fn observe(bundle_id: &str, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<()>();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let _ = tx.try_send(());
    });
    let name = NSString::from_str(&request_name(bundle_id));
    // SAFETY: our own name, no sender filter, delivered on the main run loop; the
    // block only sends on a channel.
    let token =
        unsafe {
            NSDistributedNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
        };
    cx.spawn(async move |cx| {
        let _token = token; // keeps the observer registered while the loop runs
        while rx.recv().await.is_ok() {
            let _ = cx.update(|cx| crate::window::ensure_main(cx));
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    #[test]
    fn requests_are_per_identity() {
        assert_eq!(
            super::request_name("day.mnema.gpui-dev"),
            "day.mnema.gpui-dev.show-main"
        );
        assert_ne!(
            super::request_name("day.mnema"),
            super::request_name("day.mnema.gpui-dev")
        );
    }
}
