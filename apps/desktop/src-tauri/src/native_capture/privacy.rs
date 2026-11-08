//! The privacy refresh's host half: requesting one and spawning its collector
//! read and wake the segment loop through `NativeCaptureState`, so they stay in
//! the desktop crate until S3 part c. The rest is
//! `app_core::native_capture::privacy`.

#[cfg(target_os = "macos")]
use app_core::native_capture::privacy::{
    collect_privacy_filter_update, privacy_refresh_debug_log_enabled, CollectedPrivacyFilterUpdate,
    PrivacyFilterRefreshState, PrivacyRefreshReason,
};
#[cfg(target_os = "macos")]
use app_core::AppCore;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use tauri::Manager;

#[cfg(target_os = "macos")]
pub(crate) fn request_privacy_filter_refresh(
    app_handle: &tauri::AppHandle,
    reason: PrivacyRefreshReason,
) {
    let capture_state = app_handle.state::<crate::native_capture::NativeCaptureState>();
    let control = {
        let Ok(runtime) = capture_state.lock() else {
            return;
        };
        let runtime = runtime.runtime();
        if !runtime.is_running || runtime.segment_loop_control.is_none() {
            if privacy_refresh_debug_log_enabled(reason) {
                app_core::debug_log::log(format!(
                    "privacy refresh skipped because recording is stopped (reason={reason:?})"
                ));
            }
            return;
        }
        runtime.segment_loop_control.clone()
    };

    let Some(refresh_state) = app_handle
        .state::<Arc<AppCore>>()
        .try_state::<PrivacyFilterRefreshState>()
    else {
        return;
    };
    let mut state = refresh_state
        .lock()
        .expect("privacy filter refresh state poisoned");
    if reason != PrivacyRefreshReason::FallbackPoll {
        state.static_fallback_suppressed = false;
    }
    let metadata_enabled = app_handle
        .state::<app_core::recording_settings::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .metadata
        .enabled;
    if reason == PrivacyRefreshReason::FallbackPoll
        && state.static_fallback_suppressed
        && !metadata_enabled
    {
        return;
    }
    state.requested_generation = state.requested_generation.saturating_add(1);
    state.latest_reason = Some(reason);
    if privacy_refresh_debug_log_enabled(reason) {
        app_core::debug_log::log(format!(
            "privacy refresh requested (reason={reason:?}, generation={})",
            state.requested_generation
        ));
    }
    drop(state);
    if let Some(control) = control {
        control.notify();
    }
}

#[cfg(target_os = "macos")]
pub(super) fn maybe_start_privacy_filter_collection(app_handle: &tauri::AppHandle) {
    let Some(refresh_state) = app_handle
        .state::<Arc<AppCore>>()
        .try_state::<PrivacyFilterRefreshState>()
    else {
        return;
    };
    let Some((generation, token, reason)) = ({
        let mut state = refresh_state
            .lock()
            .expect("privacy filter refresh state poisoned");
        state.begin_collection()
    }) else {
        return;
    };
    if privacy_refresh_debug_log_enabled(reason) {
        app_core::debug_log::log(format!(
            "privacy refresh collector started (reason={reason:?}, generation={generation})"
        ));
    }
    let app_handle = app_handle.clone();
    std::thread::spawn(move || {
        // A Rust-spawned thread has no autorelease pool; without this every
        // autoreleased object the frontmost-app and browser-URL reads below
        // create would leak for the life of the process. Scoped to the whole
        // closure — the thread is one-shot, so the pool drains when it exits.
        let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        // This collection runs on its own background thread, off every capture
        // lock, so a live active-tab URL read here is safe (and is the one path
        // that keeps the Gecko AX URL fresh).
        //
        // On a focus change, publish the fresh frontmost app identity with the
        // *cached* URL first. The live read below can cost up to ~1.4s (Gecko AX
        // on Zen/Firefox). `on_frame_exported` stamps each frame by *capture* time
        // via `snapshot_in_effect_at` (the history ring), so the new app has to be
        // in the ring — with a `published_at` close to the switch — before those
        // frames are captured. This cheap pre-pass publishes it within ~ms; without
        // it only the ~1.4s-late live pass would, leaving frames captured in that
        // window resolving to the *previous* app (e.g. a Zen frame labelled
        // "Hitch"). The live pass then upgrades only the browser URL for the same app.
        if reason == PrivacyRefreshReason::WorkspaceFocusChanged {
            let _ = collect_privacy_filter_update(
                &app_handle.state::<Arc<AppCore>>(),
                reason,
                app_core::native_capture::metadata::BrowserUrlReadMode::Cached,
            );
        }
        let (mode, update) = collect_privacy_filter_update(
            &app_handle.state::<Arc<AppCore>>(),
            reason,
            app_core::native_capture::metadata::BrowserUrlReadMode::Live,
        );
        if let Some(refresh_state) = app_handle
            .state::<Arc<AppCore>>()
            .try_state::<PrivacyFilterRefreshState>()
        {
            {
                let mut state = refresh_state
                    .lock()
                    .expect("privacy filter refresh state poisoned");
                // Only apply the result if this collector is still the in-flight
                // one. A collector spawned for a previous capture session can
                // finish (the Gecko AX live read can take ~1.4s) after the new
                // session has reset the refresh state; applying its stale
                // generation would poison `last_completed_generation` and stall
                // the new session's privacy/metadata refresh for many ticks.
                state.complete_collection(
                    token,
                    generation,
                    CollectedPrivacyFilterUpdate {
                        generation,
                        reason,
                        mode,
                        update,
                    },
                );
            }
        }
        if let Some(control) = app_handle
            .state::<crate::native_capture::NativeCaptureState>()
            .lock()
            .ok()
            .and_then(|runtime| runtime.runtime().segment_loop_control.clone())
        {
            control.notify();
        }
    });
}
