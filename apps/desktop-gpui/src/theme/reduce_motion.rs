//! GPUI never reads the system's Reduce Motion (its flag defaults to false and is
//! set by nobody), so we read `NSWorkspace` at start and follow
//! `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`.

use std::ptr::NonNull;

use block2::RcBlock;
use gpui_kit::App;
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
use objc2_foundation::NSNotification;

fn system_reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Apply the current value and keep following it for the app's lifetime.
pub fn observe(cx: &mut App) {
    cx.set_reduce_motion(system_reduce_motion());
    let (tx, rx) = async_channel::unbounded::<()>();
    let block = RcBlock::new(move |_: NonNull<NSNotification>| {
        let _ = tx.try_send(());
    });
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: a static notification name, no sender filter, delivered on the posting
    // (main) thread; the block only sends on a channel.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
            None,
            &block,
        )
    };
    cx.spawn(async move |cx| {
        let _token = token; // keeps the observer registered while the loop runs
        while rx.recv().await.is_ok() {
            let value = system_reduce_motion();
            log::info!("reduce motion changed: {value}");
            let _ = cx.update(|cx| cx.set_reduce_motion(value));
        }
    })
    .detach();
}
