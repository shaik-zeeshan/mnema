//! The graceful-exit coordinator: whether a quit has begun, whether it should
//! relaunch, and whether its work is done. Managed on the `AppCore`; each host's
//! exit path drives it and long-running startup work polls it.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::AppCore;

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
