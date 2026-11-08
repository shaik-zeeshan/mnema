//! The app's startup order (BUILD-PROMPT §3.2), shared by both hosts: the
//! synchronous part a host runs before it opens its window ([`AppCore::start`])
//! and the `mnema-deferred-startup` thread it spawns once the window is open
//! ([`AppCore::spawn_deferred_startup`]). Graceful exit is
//! [`AppCore::shutdown`](crate::AppCore::shutdown) in `exit.rs`.

use std::{error::Error, sync::Arc, sync::Mutex};

use crate::AppCore;

#[cfg(test)]
mod tests;

pub const DEFERRED_STARTUP_THREAD_NAME: &str = "mnema-deferred-startup";

/// What [`AppCore::start`] read that the host needs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartReport {
    /// Read before the window opens; gates capture auto-start and the update timer.
    pub onboarding_complete: bool,
}

#[derive(Debug)]
pub enum StartError {
    /// Another Mnema holds the data directory lock: the host shows its
    /// "already running" dialog and exits.
    AlreadyRunning,
    /// Startup can't go on; the host fails its setup with this error.
    Failed(Box<dyn Error>),
}

/// Marks a startup or shutdown step. Always `true` in the app; the order test
/// records each step and skips the ones that reach the OS, the network or the
/// keychain.
pub(crate) fn step(core: &AppCore, name: &'static str) -> bool {
    #[cfg(test)]
    if let Some(probe) = core.try_state::<tests::StepProbe>() {
        return probe.record(name);
    }
    let _ = (core, name);
    true
}

/// Registers the `Default` states every host shares. Call once, before `start`.
pub fn manage_default_states(core: &AppCore) {
    core.manage(crate::one_time_prompts::OneTimePromptStateStore::default());
    core.manage(crate::licensing::LicenseGate(Mutex::new(None)));
    core.manage(crate::licensing::ActivationHint(Mutex::new(None)));
    core.manage(crate::audio_transcription_models::AudioTranscriptionModelDownloadState::default());
    core.manage(crate::speaker_analysis_models::SpeakerAnalysisModelDownloadState::default());
    core.manage(crate::ocr_models::OcrModelDownloadState::default());
    core.manage(crate::updates::AppUpdateSettingsState::default());
    core.manage(crate::semantic_search_models::SemanticSearchModelDownloadState::default());
    // One settings state, shared by the host's commands and app-core.
    core.manage(crate::recording_settings::RecordingSettingsState::default());
    core.manage(crate::onboarding_state::OnboardingStateStore::default());
    core.manage(crate::exit::AppExitCoordinatorState::default());
    core.manage(crate::native_capture::metadata::CaptureMetadataState::default());
    core.manage(crate::native_capture::privacy::PrivacyFilterRefreshState::default());
    core.manage(crate::broker_authorization_channel::BrokerAuthorizationChannelState::default());
    // The Semantic Index Backfill worker's health, published each sweep pass and
    // read by the debug surface's `get_semantic_index_status`. Registered here
    // (not in the worker) so the state exists before deferred startup spawns it;
    // one `Arc`, shared by the worker (app-core) and the host's command.
    core.manage(crate::debug_status::SemanticWorkerHealthState::default());
    // The capture lock, the bell list and the microphone preference: one `Arc`
    // each, shared by the host's commands and app-core (capture).
    core.manage(crate::native_capture::lifecycle::NativeCaptureState::default());
    core.manage(crate::native_capture::app_notifications::AppNotificationsState::default());
    core.manage(crate::native_capture::microphone::MicrophoneControllerPreferencesState::default());
    core.manage(crate::native_capture::microphone::MicrophoneDeviceChangeNotifierState::default());
    core.manage(crate::native_capture::system_wake::SystemWakeNotifierState::default());
    core.manage(crate::native_capture::system_wake::DisplayReconfigurationNotifierState::default());
    core.manage(crate::native_capture::metadata::MetadataNotifierState::default());
    // MCP tool connectors (Workstream C, ADR 0048): persistent connection
    // manager. Lazy — nothing connects here at launch; it dials on first use.
    core.manage(crate::ask_ai::mcp::McpManager::default());
}

impl AppCore {
    /// The synchronous startup, in order: recording settings, update settings,
    /// one-time prompts, `host_setup` (the host's tray and global shortcuts),
    /// the panic hook, app-infra (the directory lock), the broker socket, the
    /// model-missing warnings, the microphone notifier, then
    /// `register_native_notifiers` (the host's OS observers). The host opens its
    /// window next, then calls [`AppCore::spawn_deferred_startup`].
    pub fn start(
        self: &Arc<Self>,
        host_setup: impl FnOnce(&Arc<AppCore>) -> Result<(), Box<dyn Error>>,
        register_native_notifiers: impl FnOnce(&Arc<AppCore>),
    ) -> Result<StartReport, StartError> {
        let core = self;
        step(core, "recording_settings");
        crate::native_capture::control::initialize_recording_settings_from_disk(core);
        step(core, "update_settings");
        crate::updates::initialize(core);
        step(core, "one_time_prompts");
        crate::one_time_prompts::initialize(core);
        step(core, "host_setup");
        host_setup(core).map_err(StartError::Failed)?;
        if step(core, "panic_hook") {
            crate::debug_log::install_panic_hook();
        }
        step(core, "app_infra");
        crate::app_infra::initialize(core).map_err(|error| match error {
            crate::app_infra::AppInfraInitializeError::AlreadyRunning => StartError::AlreadyRunning,
            crate::app_infra::AppInfraInitializeError::Other(message) => {
                StartError::Failed(std::io::Error::other(message).into())
            }
        })?;
        step(core, "broker_socket");
        crate::broker_authorization_channel::start(core)
            .map_err(|error| StartError::Failed(std::io::Error::other(error).into()))?;
        {
            use crate::native_capture::model_warnings as warnings;
            step(core, "model_warnings");
            warnings::maybe_push_audio_transcription_unavailable_startup_warning(core);
            warnings::maybe_push_ocr_unavailable_startup_warning(core);
        }
        if step(core, "microphone_notifier") {
            crate::native_capture::microphone::controller::start_microphone_device_change_notifier(
                Arc::clone(core),
            );
        }
        step(core, "native_notifiers");
        register_native_notifiers(core);
        let onboarding = core.state::<crate::onboarding_state::OnboardingStateStore>();
        Ok(StartReport {
            onboarding_complete: crate::onboarding_state::current_onboarding_state(
                core,
                &onboarding,
            )
            .is_complete(),
        })
    }

    /// Starts the `mnema-deferred-startup` thread. Call after the window is open:
    /// it is never on the window path. Returns its handle, or `None` when the
    /// thread could not be spawned and the work ran inline.
    ///
    /// The window is open and the database is ready; the remaining startup work
    /// (index maintenance, filesystem repair, background workers) runs off the
    /// window-open critical path so it no longer delays the first paint. Capture
    /// auto-start and the update check run only after that maintenance completes,
    /// preserving the ordering the old synchronous path guaranteed (notably:
    /// hidden-segment workspace repair respects the live active-capture workspace
    /// set so it never deletes a workspace a manually-started recording is
    /// already using).
    ///
    /// The thread is detached and never joined. That is safe: it is best-effort
    /// and bails early once a graceful exit is requested (both at its start and
    /// after maintenance), so it does not start NEW capture or workers while the
    /// app is tearing down. Any maintenance step it does not finish is re-run on
    /// the next launch — every step commits through SQLite WAL, so a hard process
    /// exit mid-step is crash-safe and idempotent to re-run. We therefore do not
    /// build a join-with-timeout against the hard exit at the end of `shutdown`.
    pub fn spawn_deferred_startup(
        self: &Arc<Self>,
        report: &StartReport,
    ) -> Option<std::thread::JoinHandle<()>> {
        let onboarding_complete = report.onboarding_complete;
        let core = Arc::clone(self);
        match std::thread::Builder::new()
            .name(DEFERRED_STARTUP_THREAD_NAME.to_string())
            .spawn(move || run_deferred_startup(&core, onboarding_complete))
        {
            Ok(handle) => Some(handle),
            Err(error) => {
                // Spawning a thread effectively never fails; if it does, run the
                // deferred startup inline as a last resort rather than leaving the
                // app without background workers or capture. This re-blocks first
                // paint with the full maintenance workload, which is why it is the
                // fallback and not the normal path. Logged through the app log sink
                // so a "no background workers" incident is captured in the packaged
                // app log rather than only on stderr.
                crate::debug_log::log_warn(format!(
                    "failed to spawn deferred startup thread; running inline (re-blocks first paint): {error}"
                ));
                run_deferred_startup(self, onboarding_complete);
                None
            }
        }
    }
}

/// Startup work that runs after the main window is shown, off the window-open
/// critical path: app-infra maintenance + background workers (see
/// [`crate::app_infra::startup::run_deferred_startup_blocking`]) and, once onboarding is
/// complete, capture auto-start and the update-check timer. Auto-start runs
/// only after the maintenance/repair passes complete so it preserves the
/// ordering the previous synchronous startup path guaranteed.
fn run_deferred_startup(core: &Arc<AppCore>, onboarding_complete: bool) {
    crate::app_infra::startup::run_deferred_startup_blocking(core);
    if step(core, "aggregate_reap") {
        reap_stale_system_audio_aggregate_devices();
    }
    step(core, "permission_evidence");
    hydrate_system_audio_permission_evidence(core);
    // If the user quit while the (now background) startup work was running,
    // graceful exit may have already stopped capture and be heading for a hard
    // process exit. Do not auto-start a NEW capture session or kick off the
    // update check on top of that teardown — that would record a segment
    // the hard exit then kills mid-write. The deferred maintenance
    // above already bails on this condition too (defense in depth).
    if crate::exit::is_graceful_exit_in_progress(core) {
        crate::debug_log::log_info(
            "graceful exit in progress; skipping capture auto-start and the update-check timer",
        );
        return;
    }
    // CRL refresh is anonymous and harmless without onboarding — kick off an
    // initial fetch plus the daily timer regardless of onboarding state.
    if step(core, "crl_refresh") {
        crate::crl_refresh::spawn_crl_refresh(Arc::clone(core));
    }
    step(core, "crl_timer");
    crate::crl_refresh::start_daily_crl_timer(Arc::clone(core));
    if step(core, "receipt_timer") {
        crate::licensing::receipt_refresh::start_receipt_refresh_timer(Arc::clone(core));
    }
    // Checks onboarding itself each tick, so it also covers a same-session finish.
    step(core, "not_recording_reminder");
    crate::native_capture::not_recording_reminder::spawn_not_recording_reminder(Arc::clone(core));
    if onboarding_complete {
        if step(core, "auto_start") {
            crate::native_capture::control::maybe_auto_start_native_capture(core);
        }
        // The update check is the host's (its updater plugin).
        step(core, "update_timer");
        core.host()
            .window_signal(crate::WindowSignal::StartUpdateCheckTimer);
    }
}

/// Destroys aggregate devices a crashed Mnema left behind: their UIDs collide
/// with the ones the system-audio tap mints, and the collision fails the tap's
/// start outright (ADR 0052). Runs off the first-paint path — an aggregate from a
/// previous process can only matter once a recording starts.
#[cfg(target_os = "macos")]
fn reap_stale_system_audio_aggregate_devices() {
    let destroyed = capture_system_audio::cleanup_stale_aggregate_devices();
    if destroyed > 0 {
        crate::debug_log::log_info(format!(
            "{} destroyed {destroyed} stale aggregate device(s) from a previous process",
            capture_system_audio::LOG_PREFIX
        ));
    }
}

#[cfg(not(target_os = "macos"))]
fn reap_stale_system_audio_aggregate_devices() {}

/// Loads the persisted system-audio denial evidence into the cache the
/// permission surfaces read (ADR 0052). Off the first-paint path with the reap
/// above: until it lands the permission reads as "not yet requested", and the
/// frontend re-polls on focus.
#[cfg(target_os = "macos")]
fn hydrate_system_audio_permission_evidence(core: &AppCore) {
    let Some(infra) = core.try_state::<crate::AppInfraState>() else {
        return;
    };
    let infra = infra.as_ref().clone();
    core.block_on(
        crate::native_capture::permissions::hydrate_system_audio_permission_evidence(&infra),
    );
}

#[cfg(not(target_os = "macos"))]
fn hydrate_system_audio_permission_evidence(_core: &AppCore) {}
