//! The startup and shutdown order is the contract (BUILD-PROMPT §3.2): drive
//! `start`, `spawn_deferred_startup` and `shutdown` on a temp-dir core and
//! compare the steps against today's order.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{step, DEFERRED_STARTUP_THREAD_NAME};
use crate::test_support::{core_in, install_shared_test_secret_vault, FakeHost};
use crate::{AppCore, ExitKind, WindowSignal};

/// Records each step with its thread; `record` answers `false` for the steps in
/// `skip`, which reach the OS (panic hook, Core Audio), the network (CRL) or the
/// licensing keychain, or would start a recording.
pub(crate) struct StepProbe {
    skip: &'static [&'static str],
    log: Mutex<Vec<(&'static str, String)>>,
}

impl StepProbe {
    pub(crate) fn record(&self, name: &'static str) -> bool {
        let thread = std::thread::current().name().unwrap_or("").to_string();
        self.log.lock().unwrap().push((name, thread));
        !self.skip.contains(&name)
    }

    fn names(&self) -> Vec<&'static str> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| *name)
            .collect()
    }

    fn thread_of(&self, name: &str) -> String {
        let log = self.log.lock().unwrap();
        let (_, thread) = log.iter().find(|(n, _)| *n == name).unwrap();
        thread.clone()
    }
}

const SKIPPED: &[&str] = &[
    "panic_hook",
    "microphone_notifier",
    "license_gate",
    "aggregate_reap",
    "crl_refresh",
    "receipt_timer",
    "auto_start",
];

/// `lib.rs` setup → `run_deferred_startup` → `request_graceful_exit` at base 824c20bc.
const START: &[&str] = &[
    "recording_settings",
    "update_settings",
    "one_time_prompts",
    "host_setup",
    "host.tray_and_shortcuts",
    "panic_hook",
    "app_infra",
    "broker_socket",
    "model_warnings",
    "microphone_notifier",
    "native_notifiers",
    "host.os_observers",
];
const DEFERRED: &[&str] = &[
    "vault_unlock",
    "maintenance",
    "repair_passes",
    "workers",
    "license_gate",
    "processing_worker",
    "aggregate_reap",
    "permission_evidence",
    "crl_refresh",
    "crl_timer",
    "receipt_timer",
    "not_recording_reminder",
    "auto_start",
    "update_timer",
];
const SHUTDOWN: &[&str] = &[
    "begin_exit",
    "stop_capture",
    "shutdown_workers",
    "requeue_orphaned_jobs",
    "unload_whisper",
    "host_exit",
];

/// A core whose every dir (and capture root) is under `dir`, with the default
/// states and a step probe.
fn temp_core(dir: &std::path::Path) -> (Arc<FakeHost>, Arc<AppCore>) {
    install_shared_test_secret_vault();
    // The DB key and the vault master key in files, never the keychain: the DB
    // key as `fixtures::seed_paths` does, and AppInfra's own vault handle (the
    // deferred unlock) reads the debug-build file knob. Process-wide, so set only
    // when unset: overwriting it races a fixture test between its seed and reopen.
    let keys =
        std::env::temp_dir().join(format!("mnema-app-core-test-keys-{}", std::process::id()));
    for (var, path) in [
        (
            "MNEMA_CAPTURE_INDEX_KEY_DIR",
            keys.join("capture-index-keys"),
        ),
        ("MNEMA_DEV_MASTER_KEY_FILE", keys.join("dev-master-key")),
    ] {
        if std::env::var_os(var).is_none() {
            std::env::set_var(var, path);
        }
    }
    std::fs::create_dir_all(&keys).unwrap();
    let host = Arc::new(FakeHost::default());
    let core = Arc::new(core_in(host.clone(), dir));
    super::manage_default_states(&core);
    core.manage(StepProbe {
        skip: SKIPPED,
        log: Mutex::default(),
    });
    // Capture root in the temp dir: the default is `~/.mnema`, real data.
    let mut settings = crate::recording_settings::default_recording_settings();
    settings.save_directory = dir.join("data").to_string_lossy().to_string();
    crate::recording_settings::persist_recording_settings(&core, &settings).unwrap();
    assert_eq!(
        crate::recording_settings::load_recording_settings_or_default(&core).save_directory,
        settings.save_directory
    );
    (host, core)
}

/// Short: the broker socket path must fit `sun_path` (104 bytes).
fn short_temp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ms4")
        .tempdir_in("/tmp")
        .unwrap()
}

#[test]
fn start_deferred_startup_and_shutdown_run_in_todays_order() {
    let dir = short_temp_dir();
    let (host, core) = temp_core(dir.path());
    // Onboarding done, so the auto-start and update-timer branch runs too.
    crate::onboarding_state::persist_onboarding_state(
        &core,
        &core.state::<crate::onboarding_state::OnboardingStateStore>(),
        crate::onboarding_state::OnboardingState::completed_now(),
    )
    .unwrap();

    let report = core
        .start(
            |core| {
                step(core, "host.tray_and_shortcuts");
                Ok(())
            },
            |core| {
                step(core, "host.os_observers");
            },
        )
        .unwrap_or_else(|error| panic!("start failed: {error:?}"));
    assert!(report.onboarding_complete);
    let probe = core.state::<StepProbe>();
    assert_eq!(probe.names(), START, "start runs in today's order");
    let caller = std::thread::current().name().unwrap_or("").to_string();
    for name in START {
        assert_eq!(probe.thread_of(name), caller, "{name} runs on the caller");
    }

    core.spawn_deferred_startup(&report)
        .expect("deferred startup gets its own thread")
        .join()
        .expect("deferred startup finishes");
    assert_eq!(probe.names(), [START, DEFERRED].concat());
    for name in DEFERRED {
        assert_eq!(
            probe.thread_of(name),
            DEFERRED_STARTUP_THREAD_NAME,
            "{name}"
        );
    }
    assert!(host
        .window_signals
        .lock()
        .unwrap()
        .contains(&WindowSignal::StartUpdateCheckTimer));

    // A second request while the first is in flight is ignored, but its restart
    // intent is kept (`AppExitCoordinatorState`).
    core.shutdown(ExitKind::Quit);
    core.shutdown(ExitKind::Restart);
    let deadline = Instant::now() + Duration::from_secs(60);
    while host.exits.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "shutdown never reached Host::exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(*host.exits.lock().unwrap(), [ExitKind::Restart]);
    assert_eq!(probe.names(), [START, DEFERRED, SHUTDOWN].concat());
    assert!(core
        .state::<crate::exit::AppExitCoordinatorState>()
        .is_final_graceful_exit_ready());

    // Done is done: a quit after the exit finished starts nothing.
    core.shutdown(ExitKind::Quit);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(host.exits.lock().unwrap().len(), 1);
}

#[test]
fn a_held_directory_lock_reads_as_already_running() {
    let dir = short_temp_dir();
    let (_, first) = temp_core(dir.path());
    first
        .start(|_| Ok(()), |_| {})
        .unwrap_or_else(|error| panic!("first start failed: {error:?}"));
    let (_, second) = temp_core(dir.path());

    let result = second.start(|_| Ok(()), |_| {});

    assert!(matches!(result, Err(super::StartError::AlreadyRunning)));
    let probe = second.state::<StepProbe>();
    assert_eq!(
        probe.names().last(),
        Some(&"app_infra"),
        "nothing after the lock"
    );
}
