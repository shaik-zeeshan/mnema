//! The tray's update row: which row the menu carries for a status, and the memo
//! that keeps a per-chunk status emit from rebuilding the menu every time.

use std::sync::Mutex;

use app_core::status_bar_model::tray_update_menu_item;

use super::{current_status, AppUpdateRuntimeState};
use tauri::Manager;

/// Last update row pushed to the tray. `emit_current_status` fires once per
/// downloaded chunk, and a menu rebuild per chunk is both wasteful and visibly
/// flickery — the row only changes a handful of times per update, so rebuild
/// only when it actually differs. Starts as `None`, matching the row-less menu
/// `status_bar::initialize` builds before any check has run.
static LAST_TRAY_UPDATE_ITEM: Mutex<Option<(String, bool)>> = Mutex::new(None);

pub(crate) fn refresh_tray_update_item(app_handle: &tauri::AppHandle) {
    // The tray carries the same update row, and it's the only surface a user
    // with no window open can see. Callers always drop the runtime lock before
    // emitting, so the rebuild's own `current_status` read can't deadlock.
    let item = tray_update_item(app_handle);
    {
        let last = LAST_TRAY_UPDATE_ITEM
            .lock()
            .expect("tray update item state poisoned");
        if !tray_memo_needs_rebuild(&last, &item) {
            return;
        }
    }
    // The memo records what the tray ACTUALLY shows, so it is committed only
    // after a rebuild that landed. `status_bar::refresh` swallows a failed menu
    // build and the not-yet-initialized tray; recording those as delivered
    // would suppress every later rebuild for the same row.
    //
    // The lock is deliberately NOT held across the rebuild: `refresh` blocks on
    // main-thread round trips, so a main-thread emitter waiting on this lock
    // would deadlock against it.
    //
    // ponytail: `refresh` re-reads live state, so a state change racing the
    // rebuild can draw a newer row than `item`. That self-heals — the newer
    // state emits too, and its row differs from this memo, forcing another
    // rebuild. A duplicate rebuild is the cost; a missing row is not.
    if crate::status_bar::refresh(app_handle) {
        *LAST_TRAY_UPDATE_ITEM
            .lock()
            .expect("tray update item state poisoned") = item;
    }
}

pub(crate) fn tray_update_item(app_handle: &tauri::AppHandle) -> Option<(String, bool)> {
    let status = current_status(app_handle);
    let installable = {
        let runtime_state = app_handle.state::<AppUpdateRuntimeState>();
        let runtime = runtime_state
            .lock()
            .expect("app update runtime state poisoned");
        runtime.pending_update.is_some()
    };
    tray_update_menu_item(
        status.state,
        status.update.as_ref().map(|update| update.version.as_str()),
        installable,
    )
}

/// Whether a tray rebuild should be requested for `item`, given what the tray
/// last actually drew. Split out from [`refresh_tray_update_item`] because that
/// function is welded to an `AppHandle` and therefore untestable; this is the
/// part with the decision in it.
fn tray_memo_needs_rebuild(last: &Option<(String, bool)>, item: &Option<(String, bool)>) -> bool {
    last != item
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_memo_only_short_circuits_on_a_row_the_tray_actually_drew() {
        let row = Some(("Install Update 0.3.0\u{2026}".to_string(), true));

        // Nothing recorded yet (the row-less menu `initialize` builds): rebuild.
        assert!(tray_memo_needs_rebuild(&None, &row));
        // Recorded and unchanged: skip. `emit_current_status` fires once per
        // downloaded chunk, and a menu rebuild per chunk is visibly flickery.
        assert!(!tray_memo_needs_rebuild(&row, &row));
        // Enabled-ness alone is a real change (Available -> mid-install).
        let disabled = Some(("Install Update 0.3.0\u{2026}".to_string(), false));
        assert!(tray_memo_needs_rebuild(&row, &disabled));
        // Row going away is a change too, or a stale row outlives its state.
        assert!(tray_memo_needs_rebuild(&row, &None));
        assert!(!tray_memo_needs_rebuild(&None, &None));
    }
}
