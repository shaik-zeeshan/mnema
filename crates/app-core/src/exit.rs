//! The graceful exit ([`AppCore::shutdown`]) and its coordinator: whether a quit
//! has begun, whether it should relaunch, and whether its work is done. Managed
//! on the `AppCore`; each host's quit paths call `shutdown` and long-running
//! startup work polls it.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{startup::step, AppCore, ExitKind};

/// How long a graceful exit may run before the process is ended anyway. Each
/// recording writer gets up to 15 s to finalize, so this leaves room for several.
const QUIT_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct AppExitCoordinatorState {
    exit_requested: AtomicBool,
    final_graceful_exit_ready: AtomicBool,
    restart_after_graceful_exit: AtomicBool,
    host_exit_claimed: AtomicBool,
}

impl AppExitCoordinatorState {
    pub fn begin_exit(&self, restart_after_graceful_exit: bool) -> bool {
        // Merge restart intent on every call so a restart-to-update request is
        // honored even when a plain graceful exit is already in progress. We
        // only ever raise the flag (never downgrade true -> false), so a later
        // plain quit cannot cancel a pending update restart.
        self.restart_after_graceful_exit
            .fetch_or(restart_after_graceful_exit, Ordering::SeqCst);
        !self.exit_requested.swap(true, Ordering::SeqCst)
    }

    pub fn is_exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::SeqCst)
    }

    pub fn mark_final_graceful_exit_ready(&self) {
        self.final_graceful_exit_ready.store(true, Ordering::SeqCst);
    }

    pub fn is_final_graceful_exit_ready(&self) -> bool {
        self.final_graceful_exit_ready.load(Ordering::SeqCst)
    }

    pub fn should_restart_after_graceful_exit(&self) -> bool {
        self.restart_after_graceful_exit.load(Ordering::SeqCst)
    }

    /// `true` for exactly one caller: whoever hands the process to `Host::exit`
    /// (the finished shutdown or the quit deadline).
    fn claim_host_exit(&self) -> bool {
        !self.host_exit_claimed.swap(true, Ordering::SeqCst)
    }

    fn is_host_exit_claimed(&self) -> bool {
        self.host_exit_claimed.load(Ordering::SeqCst)
    }
}

pub fn is_graceful_exit_in_progress(core: &AppCore) -> bool {
    core.state::<AppExitCoordinatorState>().is_exit_requested()
}

impl AppCore {
    /// Graceful exit: stop capture, shut the background workers down (with a
    /// timeout) and requeue the jobs they were on, unload the cached Local
    /// Whisper contexts, then [`Host::exit`](crate::Host::exit) with the merged
    /// intent. Runs on the runtime and returns at once. A call while one is in
    /// flight only merges its restart intent (a later plain quit never cancels a
    /// pending relaunch).
    pub fn shutdown(self: &Arc<Self>, kind: ExitKind) {
        let restart_after_exit = kind == ExitKind::Restart;
        let exit_state = self.state::<AppExitCoordinatorState>();
        if !exit_state.begin_exit(restart_after_exit) {
            return;
        }
        step(self, "begin_exit");
        spawn_quit_deadline(self, QUIT_DEADLINE);

        let core = Arc::clone(self);
        self.spawn(async move {
            crate::debug_log::log_info(if restart_after_exit {
                "starting graceful app relaunch; stopping capture and background workers before unloading cached Local Whisper contexts"
            } else {
                "starting graceful app exit; stopping capture and background workers before unloading cached Local Whisper contexts"
            });

            // A voice take holds the microphone for up to 15 s: close it before
            // capture stops, so the lend's hand-back lands while stop can see it.
            let _ = core
                .spawn_blocking(
                    crate::native_capture::microphone::bounded_clip::cancel_bounded_microphone_clip_and_wait,
                )
                .await;

            step(&core, "stop_capture");
            let stop_core = Arc::clone(&core);
            match core
                .spawn_blocking(move || {
                    if crate::native_capture::control::current_native_capture_session(&stop_core)
                        .is_running
                    {
                        Some(crate::native_capture::control::stop_native_capture_from_core(
                            &stop_core,
                        ))
                    } else {
                        None
                    }
                })
                .await
            {
                Ok(Some(Ok(_))) => crate::debug_log::log_info(
                    "stopped active native capture during graceful app exit",
                ),
                Ok(Some(Err(error))) => crate::debug_log::log_error(format!(
                    "failed to stop active native capture during graceful app exit: [{}] {}",
                    error.code, error.message
                )),
                Ok(None) => {}
                Err(error) => {
                    crate::debug_log::log_error(format!(
                        "failed to join native capture stop during graceful app exit: {error}"
                    ));
                }
            }
            crate::debug_log::log_info("stopping background workers before terminating");
            step(&core, "shutdown_workers");
            crate::app_infra::shutdown_background_workers_for_app_exit(&core).await;

            step(&core, "unload_whisper");
            match audio_transcription::providers::local_whisper::unload_all_cached_contexts() {
                Ok(unloaded) => {
                    crate::debug_log::log(format!(
                        "unloaded {unloaded} cached Local Whisper context(s) after background worker shutdown"
                    ));
                }
                Err(error) => {
                    crate::debug_log::log_warn(format!(
                        "failed to unload cached Local Whisper contexts after background worker shutdown: {error}"
                    ));
                }
            }

            complete_graceful_exit(&core);
        });
    }
}

/// Ends the process if the graceful exit hasn't reached `Host::exit` within
/// `deadline` (a hung worker or writer). Always a plain quit: a relaunch needs
/// the main thread on the GPUI host, and that may be what is stuck.
fn spawn_quit_deadline(core: &Arc<AppCore>, deadline: Duration) {
    let core = Arc::downgrade(core);
    let spawned = std::thread::Builder::new()
        .name("mnema-quit-deadline".to_string())
        .spawn(move || {
            std::thread::sleep(deadline);
            let Some(core) = core.upgrade() else { return };
            let exit_state = core.state::<AppExitCoordinatorState>();
            if !exit_state.claim_host_exit() {
                return;
            }
            crate::debug_log::log_warn("quit deadline reached, forcing exit");
            exit_state.mark_final_graceful_exit_ready();
            core.host().exit(ExitKind::Quit);
        });
    if let Err(error) = spawned {
        crate::debug_log::log_warn(format!("failed to start the quit deadline thread: {error}"));
    }
}

/// Marks the exit final (the host's quit hooks then let it through) and hands
/// the process to the host, relaunching when any request asked for it. A
/// relaunch leaves the quit deadline armed: on GPUI it only messages the main
/// thread, and if that is stuck the deadline's plain quit still ends the process.
fn complete_graceful_exit(core: &AppCore) {
    let exit_state = core.state::<AppExitCoordinatorState>();
    let restart_requested = exit_state.should_restart_after_graceful_exit();
    let ours = if restart_requested {
        !exit_state.is_host_exit_claimed()
    } else {
        exit_state.claim_host_exit()
    };
    if !ours {
        return; // the quit deadline already ended it
    }
    exit_state.mark_final_graceful_exit_ready();
    step(core, "host_exit");
    core.host().exit(if restart_requested {
        ExitKind::Restart
    } else {
        ExitKind::Quit
    });
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use super::{complete_graceful_exit, spawn_quit_deadline, AppExitCoordinatorState};
    use crate::test_support::{core_in, FakeHost};
    use crate::{AppCore, ExitKind};

    fn exiting_core(dir: &std::path::Path, restart: bool) -> (Arc<FakeHost>, Arc<AppCore>) {
        let host = Arc::new(FakeHost::default());
        let core = Arc::new(core_in(Arc::clone(&host), dir));
        core.manage(AppExitCoordinatorState::default());
        core.state::<AppExitCoordinatorState>().begin_exit(restart);
        (host, core)
    }

    #[test]
    fn quit_deadline_forces_one_exit_when_shutdown_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let (host, core) = exiting_core(dir.path(), true);

        spawn_quit_deadline(&core, Duration::from_millis(20));
        let give_up = Instant::now() + Duration::from_secs(10);
        while host.exits.lock().unwrap().is_empty() {
            assert!(Instant::now() < give_up, "quit deadline never fired");
            std::thread::sleep(Duration::from_millis(10));
        }
        // A plain quit even though a relaunch was asked for.
        assert_eq!(*host.exits.lock().unwrap(), [ExitKind::Quit]);
        assert!(core
            .state::<AppExitCoordinatorState>()
            .is_final_graceful_exit_ready());

        // The hung shutdown finishing late hands nothing over a second time.
        complete_graceful_exit(&core);
        assert_eq!(*host.exits.lock().unwrap(), [ExitKind::Quit]);
    }

    #[test]
    fn quit_deadline_stays_quiet_after_a_normal_quit() {
        let dir = tempfile::tempdir().unwrap();
        let (host, core) = exiting_core(dir.path(), false);

        spawn_quit_deadline(&core, Duration::from_millis(20));
        complete_graceful_exit(&core);
        std::thread::sleep(Duration::from_millis(200));

        assert_eq!(*host.exits.lock().unwrap(), [ExitKind::Quit]);
    }

    /// A relaunch only messages the GPUI main thread; if that is wedged the
    /// process must still end, so the deadline stays armed behind it.
    #[test]
    fn quit_deadline_still_quits_when_a_relaunch_never_lands() {
        let dir = tempfile::tempdir().unwrap();
        let (host, core) = exiting_core(dir.path(), true);

        spawn_quit_deadline(&core, Duration::from_millis(20));
        complete_graceful_exit(&core);
        let give_up = Instant::now() + Duration::from_secs(10);
        while host.exits.lock().unwrap().len() < 2 {
            assert!(Instant::now() < give_up, "quit deadline never fired");
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            *host.exits.lock().unwrap(),
            [ExitKind::Restart, ExitKind::Quit]
        );
    }

    #[test]
    fn app_exit_coordinator_marks_final_exit_only_after_graceful_work_is_done() {
        let coordinator = AppExitCoordinatorState::default();

        assert!(coordinator.begin_exit(false));
        assert!(!coordinator.begin_exit(false));
        assert!(coordinator.is_exit_requested());
        assert!(!coordinator.is_final_graceful_exit_ready());

        coordinator.mark_final_graceful_exit_ready();

        assert!(coordinator.is_final_graceful_exit_ready());
    }

    #[test]
    fn restart_intent_is_preserved_when_requested_after_exit_begins() {
        let coordinator = AppExitCoordinatorState::default();

        // A plain graceful exit (quit / window close) starts first.
        assert!(coordinator.begin_exit(false));
        assert!(!coordinator.should_restart_after_graceful_exit());

        // The user triggers restart-to-update before shutdown finishes; the
        // request is dropped for spawning purposes but its intent is retained.
        assert!(!coordinator.begin_exit(true));
        assert!(coordinator.should_restart_after_graceful_exit());
    }

    #[test]
    fn restart_intent_is_not_downgraded_by_a_later_plain_exit() {
        let coordinator = AppExitCoordinatorState::default();

        assert!(coordinator.begin_exit(true));
        assert!(coordinator.should_restart_after_graceful_exit());

        // A subsequent plain exit must not cancel the pending update restart.
        assert!(!coordinator.begin_exit(false));
        assert!(coordinator.should_restart_after_graceful_exit());
    }
}
