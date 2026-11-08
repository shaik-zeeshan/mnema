//! The tray's update row: which row the menu carries for a status, and the memo
//! that keeps a per-chunk status emit from rebuilding the menu every time.

use std::sync::Mutex;

use app_core::updates::AppUpdateState;

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

/// Label + enabled state for the tray's update item, or `None` when the menu
/// should carry no update row at all. Out-of-window builds
/// (`AvailableOutOfWindow`) are deliberately absent: they aren't installable, so
/// the tray would offer a button that can only fail — Settings does the directing.
///
/// `Failed` DOES keep a row. A failed download or bundle swap leaves
/// `pending_update` intact and is retryable, which is why the update window and
/// Settings → About both keep their install button live for it; dropping the
/// tray row instead stranded the windowless user the row exists for.
///
/// `installable` is `pending_update.is_some()`, NOT the presence of a version:
/// they are different fields and they come apart. A failed *check* clears
/// `update` (no version, no row), but `install_app_update`'s no-pending branch
/// and `restart_after_app_update`'s error path both set `Failed` while LEAVING
/// `update` populated — keying the row on the version alone would draw an
/// enabled "Retry" whose only possible outcome is the failure alert. That is
/// exactly the "button that can only fail" reasoning that keeps
/// `AvailableOutOfWindow` off the tray.
pub(crate) fn tray_update_menu_item(
    state: AppUpdateState,
    version: Option<&str>,
    installable: bool,
) -> Option<(String, bool)> {
    match state {
        AppUpdateState::RestartRequired => Some(("Restart to Finish Update".to_string(), true)),
        AppUpdateState::Downloading | AppUpdateState::Installing => {
            Some(("Updating\u{2026}".to_string(), false))
        }
        AppUpdateState::Available if installable => {
            version.map(|version| (format!("Install Update {version}\u{2026}"), true))
        }
        AppUpdateState::Failed if installable => {
            version.map(|version| (format!("Retry Update {version}\u{2026}"), true))
        }
        _ => None,
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
    fn tray_update_item_only_appears_for_installable_states() {
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Available, Some("0.3.0"), true),
            Some(("Install Update 0.3.0\u{2026}".to_string(), true))
        );
        assert_eq!(
            tray_update_menu_item(AppUpdateState::RestartRequired, None, false),
            Some(("Restart to Finish Update".to_string(), true))
        );
        // Mid-install the row shows progress but must not be clickable again.
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Downloading, Some("0.3.0"), true),
            Some(("Updating\u{2026}".to_string(), false))
        );
        // A failed install is retryable and keeps its pending update, so the
        // tray keeps a row — matching the update window and Settings, which
        // both keep `failed` installable.
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Failed, Some("0.3.0"), true),
            Some(("Retry Update 0.3.0\u{2026}".to_string(), true))
        );
        // A failed CHECK carries no version (run_update_check clears it), so
        // there is nothing to retry and no row.
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Failed, None, false),
            None
        );
        // The dangerous shape: `install_app_update`'s no-pending branch and
        // `restart_after_app_update`'s error path both set Failed while leaving
        // `update` populated. A version-keyed row would draw an enabled "Retry"
        // whose only outcome is the failure alert.
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Failed, Some("0.3.0"), false),
            None
        );
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Available, Some("0.3.0"), false),
            None
        );
        for state in [
            AppUpdateState::Idle,
            AppUpdateState::Checking,
            AppUpdateState::UpToDate,
            AppUpdateState::AvailableOutOfWindow,
            AppUpdateState::Incompatible,
        ] {
            assert_eq!(
                tray_update_menu_item(state, Some("0.3.0"), true),
                None,
                "{state:?}"
            );
        }
        // Available with no version in the feed is a broken row, not a row.
        assert_eq!(
            tray_update_menu_item(AppUpdateState::Available, None, true),
            None
        );
    }


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
