#[cfg(target_os = "macos")]
use super::runtime::NativeCaptureRuntime;
#[cfg(target_os = "macos")]
use capture_types::CaptureErrorResponse;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use tauri::Manager;

/// Never-reset source of process-globally-unique collector tokens. Unlike the
/// per-runtime generation counter (which restarts at 0 on every capture-session
/// reset), this only ever increments, so a token minted for one collector can
/// never equal a token minted for another — even across a reset that re-uses the
/// same generation number.
#[cfg(target_os = "macos")]
static COLLECTION_TOKEN_SOURCE: AtomicU64 = AtomicU64::new(0);

/// Error code reported when a privacy-filter apply fails specifically because no
/// capture display is available (display sleep, screen lock, lid close, monitor
/// disconnect). The segment loop treats this as a transient liveness condition
/// to recover from, distinct from a genuine privacy-filter failure.
#[cfg(target_os = "macos")]
pub(super) const PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE: &str =
    "privacy_filter_display_unavailable";

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub(super) struct InitialPrivacyFilter {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
}

#[cfg(target_os = "macos")]
impl InitialPrivacyFilter {
    pub(super) fn screen_capture_filter(&self) -> Option<capture_screen::PrivacyContentFilter> {
        self.filter.clone()
    }

    /// The same excluded apps the screen filter hides, for the system-audio tap's
    /// exclude list. Parity, not a feature: ScreenCaptureKit's content filter
    /// already silenced these apps' audio (ADR 0052).
    pub(super) fn excluded_bundle_ids(&self) -> Vec<String> {
        self.decision.excluded_bundle_ids.clone()
    }

    pub(super) fn mark_applied(self, app_handle: &tauri::AppHandle) {
        mark_privacy_decision_applied(app_handle, self.decision);
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) struct PrivacyFilterUpdate {
    decision: capture_metadata::PrivacyFilterDecision,
    filter: Option<capture_screen::PrivacyContentFilter>,
}

#[cfg(target_os = "macos")]
impl PrivacyFilterUpdate {
    /// The apps this update excludes, for the system-audio tap's exclude list.
    pub(super) fn excluded_bundle_ids(&self) -> &[String] {
        &self.decision.excluded_bundle_ids
    }
}

#[cfg(target_os = "macos")]
pub type PrivacyFilterRefreshState = Mutex<PrivacyFilterRefreshRuntime>;

#[cfg(not(target_os = "macos"))]
pub type PrivacyFilterRefreshState = std::sync::Mutex<()>;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyRefreshReason {
    StaticAppRuleMutation,
    MetadataSettingsMutation,
    WorkspaceAppChanged,
    WorkspaceFocusChanged,
    FallbackPoll,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivacyRefreshMode {
    StaticExcludedAppsOnly,
    MetadataAndStaticApps,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) struct CollectedPrivacyFilterUpdate {
    pub generation: u64,
    pub reason: PrivacyRefreshReason,
    pub mode: PrivacyRefreshMode,
    pub update: PrivacyFilterUpdate,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct PrivacyFilterRefreshRuntime {
    requested_generation: u64,
    latest_reason: Option<PrivacyRefreshReason>,
    collecting_generation: Option<u64>,
    /// Process-globally-unique id of the in-flight collector. The per-runtime
    /// `generation` counter restarts from 0 on every `reset_privacy_filter_refresh_state`
    /// (the runtime is replaced with `default()`), so a stale collector from a
    /// previous session can hold the SAME generation number a new session later
    /// re-claims — making `generation` alone unsafe to match a completion on.
    /// This token is minted from a never-reset global, so it can never collide
    /// across a reset.
    collecting_token: Option<u64>,
    last_completed_generation: u64,
    completed_update: Option<CollectedPrivacyFilterUpdate>,
    static_fallback_suppressed: bool,
}

#[cfg(target_os = "macos")]
impl PrivacyFilterRefreshRuntime {
    /// Claims the next collection generation, or `None` when a collector is
    /// already in flight or there is nothing newer to collect. The returned
    /// generation marks this runtime as collecting; the collector must hand it
    /// back to [`Self::complete_collection`].
    fn begin_collection(&mut self) -> Option<(u64, u64, PrivacyRefreshReason)> {
        if self.collecting_generation.is_some()
            || self.requested_generation <= self.last_completed_generation
        {
            return None;
        }
        let generation = self.requested_generation;
        let reason = self.latest_reason.unwrap_or(PrivacyRefreshReason::FallbackPoll);
        let token = COLLECTION_TOKEN_SOURCE.fetch_add(1, Ordering::Relaxed);
        self.collecting_generation = Some(generation);
        self.collecting_token = Some(token);
        Some((generation, token, reason))
    }

    /// Records the result of a finished collection — but only if `token` is still
    /// the in-flight one. A collector spawned for a *previous* session can land
    /// its write-back after [`reset_privacy_filter_refresh_state`] (or a later
    /// collection) has replaced the runtime; matching on the per-runtime
    /// `generation` is NOT enough, because the reset restarts that counter at 0
    /// and the new session can re-claim the SAME generation the stale collector
    /// still holds (the equal-generation collision). Applying that stale write
    /// would clobber the live session's snapshot with the previous session's data
    /// and poison `last_completed_generation`. The process-global `token` cannot
    /// collide across a reset, so it is the safe match key. Returns whether the
    /// result was applied.
    fn complete_collection(
        &mut self,
        token: u64,
        generation: u64,
        completed: CollectedPrivacyFilterUpdate,
    ) -> bool {
        if self.collecting_token != Some(token) {
            return false;
        }
        self.collecting_generation = None;
        self.collecting_token = None;
        self.last_completed_generation = generation;
        self.completed_update = Some(completed);
        true
    }
}

#[cfg(target_os = "macos")]
pub(super) fn privacy_filter_from_decision(
    decision: capture_metadata::PrivacyFilterDecision,
) -> Option<capture_screen::PrivacyContentFilter> {
    decision
        .privacy_filter_applied
        .then_some(capture_screen::PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: decision.excluded_bundle_ids,
        })
}

#[cfg(target_os = "macos")]
pub(super) fn privacy_refresh_debug_log_enabled(reason: PrivacyRefreshReason) -> bool {
    cfg!(feature = "privacy-refresh-trace") && reason != PrivacyRefreshReason::FallbackPoll
}

#[cfg(target_os = "macos")]
pub(super) fn collect_initial_privacy_filter(
    app_handle: &tauri::AppHandle,
) -> InitialPrivacyFilter {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = collect_initial_privacy_filter_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings,
    );
    let filter = privacy_filter_from_decision(decision.clone());
    InitialPrivacyFilter { decision, filter }
}

#[cfg(target_os = "macos")]
fn collect_initial_privacy_filter_decision(
    metadata_state: &crate::native_capture::CaptureMetadataState,
    settings: &capture_types::RecordingSettings,
) -> capture_metadata::PrivacyFilterDecision {
    if settings.metadata.enabled {
        // `collect_initial_privacy_filter` is only ever called from synchronous
        // segment-start / resume / recovery paths that hold the
        // `NativeCaptureState` mutex, so the active-tab URL must come from the
        // cache (no live AX/AppleScript read) to avoid stalling under the lock.
        crate::native_capture::metadata::refresh_metadata_state(
            metadata_state,
            &settings.metadata,
            &settings.privacy,
            crate::native_capture::metadata::BrowserUrlReadMode::Cached,
        )
    } else {
        crate::native_capture::metadata::refresh_static_excluded_app_privacy_state(
            metadata_state,
            &settings.privacy,
        )
    }
}

#[cfg(target_os = "macos")]
fn collect_static_privacy_filter_update(app_handle: &tauri::AppHandle) -> PrivacyFilterUpdate {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = crate::native_capture::metadata::refresh_static_excluded_app_privacy_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.privacy,
    );
    let latest_applied = crate::native_capture::metadata::latest_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate { decision, filter }
}

#[cfg(target_os = "macos")]
fn collect_metadata_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    browser_url_read_mode: crate::native_capture::metadata::BrowserUrlReadMode,
) -> PrivacyFilterUpdate {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let decision = crate::native_capture::metadata::refresh_metadata_state(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        &settings.metadata,
        &settings.privacy,
        browser_url_read_mode,
    );
    let latest_applied = crate::native_capture::metadata::latest_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
    );
    let filter = privacy_filter_from_decision(decision.clone()).or_else(|| {
        latest_applied
            .privacy_filter_applied
            .then_some(empty_privacy_filter())
    });
    PrivacyFilterUpdate { decision, filter }
}

#[cfg(target_os = "macos")]
fn privacy_refresh_mode(
    settings: &capture_types::RecordingSettings,
    reason: PrivacyRefreshReason,
) -> PrivacyRefreshMode {
    if reason == PrivacyRefreshReason::MetadataSettingsMutation {
        PrivacyRefreshMode::MetadataAndStaticApps
    } else if settings.metadata.enabled && reason != PrivacyRefreshReason::StaticAppRuleMutation {
        PrivacyRefreshMode::MetadataAndStaticApps
    } else {
        PrivacyRefreshMode::StaticExcludedAppsOnly
    }
}

#[cfg(target_os = "macos")]
pub(super) fn collect_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    reason: PrivacyRefreshReason,
    browser_url_read_mode: crate::native_capture::metadata::BrowserUrlReadMode,
) -> (PrivacyRefreshMode, PrivacyFilterUpdate) {
    let settings = app_handle
        .state::<crate::native_capture::RecordingSettingsState>()
        .lock()
        .expect("recording settings state poisoned")
        .settings
        .clone();
    let mode = privacy_refresh_mode(&settings, reason);
    let update = match mode {
        PrivacyRefreshMode::StaticExcludedAppsOnly => {
            collect_static_privacy_filter_update(app_handle)
        }
        PrivacyRefreshMode::MetadataAndStaticApps => {
            collect_metadata_privacy_filter_update(app_handle, browser_url_read_mode)
        }
    };
    (mode, update)
}

#[cfg(target_os = "macos")]
pub(crate) fn reset_privacy_filter_refresh_state(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    {
        *state.lock().expect("privacy filter refresh state poisoned") =
            PrivacyFilterRefreshRuntime::default();
    }
}

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
                super::debug_log::log(format!(
                    "privacy refresh skipped because recording is stopped (reason={reason:?})"
                ));
            }
            return;
        }
        runtime.segment_loop_control.clone()
    };

    let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
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
        .state::<crate::native_capture::RecordingSettingsState>()
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
        super::debug_log::log(format!(
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
    let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
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
        super::debug_log::log(format!(
            "privacy refresh collector started (reason={reason:?}, generation={generation})"
        ));
    }
    let app_handle = app_handle.clone();
    std::thread::spawn(move || {
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
                &app_handle,
                reason,
                crate::native_capture::metadata::BrowserUrlReadMode::Cached,
            );
        }
        let (mode, update) = collect_privacy_filter_update(
            &app_handle,
            reason,
            crate::native_capture::metadata::BrowserUrlReadMode::Live,
        );
        if let Some(refresh_state) =
            app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
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

#[cfg(target_os = "macos")]
pub(super) fn take_completed_privacy_filter_update(
    app_handle: &tauri::AppHandle,
) -> Option<CollectedPrivacyFilterUpdate> {
    let refresh_state =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()?;
    let mut state = refresh_state
        .lock()
        .expect("privacy filter refresh state poisoned");
    let completed = state.completed_update.take()?;
    if completed.generation == state.requested_generation {
        Some(completed)
    } else {
        if privacy_refresh_debug_log_enabled(completed.reason) {
            super::debug_log::log(format!(
                "stale privacy refresh skipped (reason={:?}, completed_generation={}, requested_generation={})",
                completed.reason, completed.generation, state.requested_generation
            ));
        }
        None
    }
}

#[cfg(target_os = "macos")]
pub(super) fn record_privacy_filter_apply_outcome(
    app_handle: &tauri::AppHandle,
    mode: PrivacyRefreshMode,
    outcome: capture_screen::PrivacyFilterApplyOutcome,
) {
    if mode != PrivacyRefreshMode::StaticExcludedAppsOnly {
        return;
    }
    if let Some(refresh_state) =
        app_handle.try_state::<crate::native_capture::PrivacyFilterRefreshState>()
    {
        let mut state = refresh_state
            .lock()
            .expect("privacy filter refresh state poisoned");
        if outcome.request_satisfied {
            state.static_fallback_suppressed = true;
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn record_initial_privacy_filter_outcome(
    app_handle: &tauri::AppHandle,
    _settings: &capture_types::RecordingSettings,
    outcome: Option<capture_screen::PrivacyFilterApplyOutcome>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    record_privacy_filter_apply_outcome(
        app_handle,
        PrivacyRefreshMode::StaticExcludedAppsOnly,
        outcome,
    );
}

#[cfg(target_os = "macos")]
pub(super) fn apply_privacy_filter_update(
    app_handle: &tauri::AppHandle,
    runtime: &mut NativeCaptureRuntime,
    update: PrivacyFilterUpdate,
) -> Result<capture_screen::PrivacyFilterApplyOutcome, CaptureErrorResponse> {
    if !capture_screen::screen_capture_session_is_live(runtime.active_screen_session.as_ref()) {
        return Ok(capture_screen::PrivacyFilterApplyOutcome {
            request_satisfied: false,
        });
    }

    let Some(filter) = update.filter else {
        mark_privacy_decision_applied(app_handle, update.decision);
        return Ok(capture_screen::PrivacyFilterApplyOutcome {
            request_satisfied: true,
        });
    };

    let outcome =
        capture_screen::update_active_privacy_filter(&mut runtime.active_screen_session, filter)
            .map_err(|error| CaptureErrorResponse {
                code: if error.kind
                    == capture_screen::PrivacyFilterApplyErrorKind::DisplayUnavailable
                {
                    PRIVACY_FILTER_DISPLAY_UNAVAILABLE_CODE.to_string()
                } else {
                    "privacy_filter_apply_failed".to_string()
                },
                message: error.message,
            })?;
    mark_privacy_decision_applied(app_handle, update.decision);
    Ok(outcome)
}

#[cfg(target_os = "macos")]
fn mark_privacy_decision_applied(
    app_handle: &tauri::AppHandle,
    decision: capture_metadata::PrivacyFilterDecision,
) {
    crate::native_capture::metadata::mark_applied_privacy_decision(
        app_handle
            .state::<crate::native_capture::CaptureMetadataState>()
            .inner(),
        decision,
    );
}

#[cfg(target_os = "macos")]
fn empty_privacy_filter() -> capture_screen::PrivacyContentFilter {
    capture_screen::PrivacyContentFilter {
        display_id: 0,
        excluded_bundle_ids: Vec::new(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::native_capture::settings::default_recording_settings;

    #[test]
    fn privacy_refresh_uses_metadata_collection_when_metadata_is_enabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
        assert!(settings.privacy.excluded_apps.is_empty());
    }

    #[test]
    fn privacy_refresh_keeps_static_fast_path_for_static_rule_mutations() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::StaticAppRuleMutation),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
    }

    #[test]
    fn privacy_refresh_uses_metadata_collection_for_metadata_settings_mutations() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = false;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::MetadataSettingsMutation),
            PrivacyRefreshMode::MetadataAndStaticApps
        );
    }

    #[test]
    fn privacy_refresh_keeps_static_fast_path_when_metadata_is_disabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = false;

        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::FallbackPoll),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
        assert_eq!(
            privacy_refresh_mode(&settings, PrivacyRefreshReason::WorkspaceFocusChanged),
            PrivacyRefreshMode::StaticExcludedAppsOnly
        );
    }

    // A collector spawned for one capture session can still be running its slow
    // Gecko AX live read (~1.4s) when the user stops and restarts capture. The
    // restart resets the refresh runtime; if the stale collector's write-back
    // is then applied, it poisons `last_completed_generation` with the old
    // session's (larger) generation, and the new session can no longer start a
    // collection until its `requested_generation` climbs back past it — many
    // ticks of stale privacy filter / metadata. The completing collector must
    // no-op once it is no longer the in-flight generation.
    #[test]
    fn stale_collector_completion_does_not_suppress_new_session_collections() {
        let mut runtime = PrivacyFilterRefreshRuntime::default();

        // Session 1 runs for a while and starts a collection at generation 50.
        runtime.requested_generation = 50;
        let (gen1, token1, _reason) = runtime
            .begin_collection()
            .expect("session 1 should start a collection");
        assert_eq!(gen1, 50);

        // Stop + restart: the new session resets the refresh runtime while the
        // session-1 collector is still in flight (its slow AX read hasn't
        // returned yet).
        runtime = PrivacyFilterRefreshRuntime::default();

        // The new session requests a refresh (generation climbs from 0 to 1).
        runtime.requested_generation = runtime.requested_generation.saturating_add(1);

        // Now the stale session-1 collector finally finishes and writes back.
        let applied = runtime.complete_collection(
            token1,
            gen1,
            CollectedPrivacyFilterUpdate {
                generation: gen1,
                reason: PrivacyRefreshReason::FallbackPoll,
                mode: PrivacyRefreshMode::MetadataAndStaticApps,
                update: PrivacyFilterUpdate {
                    decision: capture_metadata::PrivacyFilterDecision::default(),
                    filter: None,
                },
            },
        );
        assert!(
            !applied,
            "a collector from a previous session must not apply after a reset"
        );

        // The new session must still be able to start its collection.
        assert!(
            runtime.begin_collection().is_some(),
            "new session collection suppressed by a stale collector's write-back"
        );
    }

    // REGRESSION (deep-review finding): the EQUAL-generation collision the
    // generation-only guard could not catch. After a reset the runtime is
    // replaced with `default()` (generation counter back to 0), so the new
    // session's FIRST collection re-claims the SAME generation a still-in-flight
    // stale collector holds. Matching the completion on `generation` alone would
    // then apply the previous session's data over the live session's in-flight
    // collection. The process-global token must reject the stale write while the
    // live session's own collector still applies.
    #[test]
    fn equal_generation_stale_collector_does_not_clobber_fresh_collector_after_reset() {
        let mut runtime = PrivacyFilterRefreshRuntime::default();

        // Session 1's first collection claims generation 1.
        runtime.requested_generation = runtime.requested_generation.saturating_add(1);
        let (stale_gen, stale_token, _reason) = runtime
            .begin_collection()
            .expect("session 1 should start a collection");
        assert_eq!(stale_gen, 1);

        // Stop + restart resets the runtime while the session-1 collector is
        // still running its slow AX read.
        runtime = PrivacyFilterRefreshRuntime::default();

        // Session 2's first collection ALSO claims generation 1 (the runtime
        // started over at 0). Its fresh collector is now in flight too — and it
        // holds a DIFFERENT token from the never-reset global source.
        runtime.requested_generation = runtime.requested_generation.saturating_add(1);
        let (fresh_gen, fresh_token, _reason) = runtime
            .begin_collection()
            .expect("session 2 should start its collection");
        assert_eq!(fresh_gen, 1);
        assert_ne!(
            stale_token, fresh_token,
            "tokens must be unique across a reset even at the same generation"
        );

        // The STALE session-1 collector lands first with generation 1.
        let stale_applied = runtime.complete_collection(
            stale_token,
            stale_gen,
            CollectedPrivacyFilterUpdate {
                generation: stale_gen,
                reason: PrivacyRefreshReason::FallbackPoll,
                mode: PrivacyRefreshMode::MetadataAndStaticApps,
                update: PrivacyFilterUpdate {
                    decision: capture_metadata::PrivacyFilterDecision::default(),
                    filter: None,
                },
            },
        );
        assert!(
            !stale_applied,
            "a stale collector must not apply after a reset, even when its \
             generation number collides with the new session's"
        );

        // The fresh session-2 collector must still be able to apply its result.
        let fresh_applied = runtime.complete_collection(
            fresh_token,
            fresh_gen,
            CollectedPrivacyFilterUpdate {
                generation: fresh_gen,
                reason: PrivacyRefreshReason::FallbackPoll,
                mode: PrivacyRefreshMode::MetadataAndStaticApps,
                update: PrivacyFilterUpdate {
                    decision: capture_metadata::PrivacyFilterDecision::default(),
                    filter: None,
                },
            },
        );
        assert!(
            fresh_applied,
            "the live session's own collector must still apply its result"
        );
    }

    #[test]
    fn initial_privacy_filter_collects_metadata_snapshot_when_metadata_is_enabled() {
        let mut settings = default_recording_settings();
        settings.metadata.enabled = true;
        let metadata_state = crate::native_capture::CaptureMetadataState::default();

        let _decision = collect_initial_privacy_filter_decision(&metadata_state, &settings);

        let runtime = metadata_state
            .lock()
            .expect("capture metadata state should lock");
        assert!(runtime.latest_snapshot().is_some());
    }
}
