//! The graceful exit ([`AppCore::shutdown`]) and its coordinator: whether a quit
//! has begun, whether it should relaunch, and whether its work is done. Managed
//! on the `AppCore`; each host's quit paths call `shutdown` and long-running
//! startup work polls it.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{startup::step, AppCore, ExitKind};

#[derive(Default)]
pub struct AppExitCoordinatorState {
    exit_requested: AtomicBool,
    final_graceful_exit_ready: AtomicBool,
    restart_after_graceful_exit: AtomicBool,
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

        let core = Arc::clone(self);
        self.spawn(async move {
            crate::debug_log::log_info(if restart_after_exit {
                "starting graceful app relaunch; stopping capture and background workers before unloading cached Local Whisper contexts"
            } else {
                "starting graceful app exit; stopping capture and background workers before unloading cached Local Whisper contexts"
            });

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

/// Marks the exit final (the host's quit hooks then let it through) and hands
/// the process to the host, relaunching when any request asked for it.
fn complete_graceful_exit(core: &AppCore) {
    let exit_state = core.state::<AppExitCoordinatorState>();
    let restart_requested = exit_state.should_restart_after_graceful_exit();
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
    use super::AppExitCoordinatorState;

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
