//! The live backends against a real `AppCore` on a temp dir (no fixtures in the
//! path): what the buttons and autosave reach when the app actually runs.
//! Keys live in scratch files (never the keychain); nothing starts real capture.

use std::sync::{Arc, Mutex, MutexGuard};

use app_core::fixtures::{self, FixtureId};
use app_core::{
    AppCore, AppInfraState, ConfirmDialog, CoreEvent, ExitKind, Host, MessageDialog, WindowRequest,
    WindowSignal,
};
use capture_types::{RecordingSettings, StartNativeCaptureRequest};
use mnema_gpui::settings::autosave::Unit;
use mnema_gpui::settings::data::Backend as _;
use mnema_gpui::small::debug::data::{DebugBackend as _, Section};
use mnema_gpui::{onboarding, settings, small};

/// Records every emit; confirms answer yes; the rest is a no-op.
#[derive(Default)]
struct RecHost {
    events: Mutex<Vec<CoreEvent>>,
}

impl Host for RecHost {
    fn emit(&self, event: CoreEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn message(&self, _: MessageDialog) {}
    fn confirm(&self, _: ConfirmDialog) -> tokio::sync::oneshot::Receiver<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(true);
        rx
    }
    fn open_url(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn refresh_tray(&self) {}
    fn open_window(&self, _: WindowRequest) -> Result<(), String> {
        Ok(())
    }
    fn window_signal(&self, _: WindowSignal) {}
    fn exit(&self, _: ExitKind) {}
}

impl RecHost {
    fn saw(&self, f: impl Fn(&CoreEvent) -> bool) -> bool {
        self.events.lock().unwrap().iter().any(f)
    }
}

/// The seed points process-wide env at its key store, so cores are built one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Rig {
    host: Arc<RecHost>,
    core: Arc<AppCore>,
    _dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

fn on_disk(core: &AppCore) -> RecordingSettings {
    let path = app_core::recording_settings::recording_settings_file_path(core);
    serde_json::from_str(&std::fs::read_to_string(path).expect("settings were written"))
        .expect("settings parse")
}

/// A started-enough core: default states, settings loaded from disk (its capture
/// root inside the temp dir, never `~/.mnema`), and app-infra on the seeded DB.
fn rig() -> Rig {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("MNEMA_TRIPWIRE_NO_KEYCHAIN", "1");
    if std::env::var_os("MNEMA_DEV_MASTER_KEY_FILE").is_none() {
        let keys = std::env::temp_dir().join(format!("mnema-gpui-wiring-{}", std::process::id()));
        std::fs::create_dir_all(&keys).unwrap();
        std::env::set_var("MNEMA_DEV_MASTER_KEY_FILE", keys.join("dev-master-key"));
    }
    let paths = fixtures::seed(dir.path(), FixtureId::Empty).unwrap();
    // The capture root can never fall back to `~/.mnema`: the env override AND a
    // valid saved settings file both name the temp dir (an invalid or missing
    // file silently reads as the default folder).
    std::env::set_var("MNEMA_SAVE_DIRECTORY", &paths.data);
    let host = Arc::new(RecHost::default());
    let core = Arc::new(AppCore::new(host.clone(), paths.clone()).unwrap());
    app_core::startup::manage_default_states(&core);
    let mut s = app_core::recording_settings::default_recording_settings();
    s.save_directory = paths.data.to_string_lossy().into_owned();
    app_core::recording_settings::persist_recording_settings(&core, &s).unwrap();
    app_core::native_capture::control::initialize_recording_settings_from_disk(&core);
    // Proven BEFORE app-infra opens anything.
    let root = dir.path().canonicalize().unwrap();
    let loaded = app_core::native_capture::settings_update::get_recording_settings(&core);
    assert_eq!(
        loaded.save_directory, s.save_directory,
        "the saved settings loaded as written"
    );
    for resolved in [
        loaded.save_directory.clone(),
        app_core::app_infra::get_storage_location(&core).expect("the storage root resolves"),
    ] {
        let resolved = std::path::Path::new(&resolved).canonicalize().unwrap();
        assert!(
            resolved.starts_with(&root),
            "the capture root must be inside the temp dir, got {}",
            resolved.display()
        );
    }
    let infra = core
        .block_on(app_infra::AppInfra::initialize(&paths.data))
        .expect("app infra opens the seeded DB");
    core.manage::<AppInfraState>(Arc::new(infra));
    Rig {
        host,
        core,
        _dir: dir,
        _guard: guard,
    }
}

fn data_dir(core: &AppCore) -> String {
    core.paths().data.to_string_lossy().into_owned()
}

#[test]
fn a_recording_settings_save_round_trips_to_disk_and_emits_the_change() {
    let r = rig();
    let backend = settings::data::CoreBackend(r.core.clone());
    let mut draft = settings::live::save_domain(
        &r.core,
        Unit::Developer,
        &app_core::native_capture::settings_update::get_recording_settings(&r.core),
    )
    .expect("an unchanged domain still saves");
    r.host.events.lock().unwrap().clear();

    draft.screen_frame_rate = 1.0;
    draft.segment_duration_seconds = 120;
    let saved = backend
        .save_domain(Unit::Video, &draft)
        .expect("the video domain saves");
    assert_eq!(saved.screen_frame_rate, 1.0);
    // One unit saves only its own fields.
    assert_ne!(saved.segment_duration_seconds, 120);
    assert_eq!(on_disk(&r.core).screen_frame_rate, 1.0);
    assert!(r
        .host
        .saw(|e| matches!(e, CoreEvent::RecordingSettingsChanged(_))));
    assert!(r
        .host
        .saw(|e| matches!(e, CoreEvent::RecordingSettingsDomainChanged(_))));

    // A value the backend refuses comes back as its own message, nothing written.
    draft.screen_frame_rate = 500.0;
    let refused = backend.save_domain(Unit::Video, &draft).unwrap_err();
    assert!(refused.contains("screenFrameRate"), "{refused}");
    assert_eq!(on_disk(&r.core).screen_frame_rate, 1.0);

    // Every recording unit maps to a real command.
    let current = app_core::native_capture::settings_update::get_recording_settings(&r.core);
    for unit in Unit::ALL {
        if matches!(unit, Unit::Keyboard | Unit::Microphone) {
            continue;
        }
        backend
            .save_domain(unit, &current)
            .unwrap_or_else(|e| panic!("{} failed: {e}", unit.command()));
    }
}

#[test]
fn a_privacy_exclusion_is_added_and_removed() {
    let r = rig();
    let backend = settings::data::CoreBackend(r.core.clone());
    let mut next = app_core::native_capture::settings_update::get_recording_settings(&r.core);
    app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
        &mut next,
        "com.example.bank",
        "Bank",
        true,
    )
    .unwrap();
    let saved = backend.save_privacy(&next).expect("the add applies");
    let app = saved
        .privacy
        .excluded_apps
        .iter()
        .find(|a| a.bundle_id == "com.example.bank")
        .expect("the app is excluded")
        .clone();
    assert!(app.enabled);
    assert!(on_disk(&r.core)
        .privacy
        .excluded_apps
        .iter()
        .any(|a| a.id == app.id));

    let mut next = saved.clone();
    app_core::privacy_redaction_sources::remove_privacy_excluded_app(&mut next, &app.id).unwrap();
    let saved = backend.save_privacy(&next).expect("the remove applies");
    assert!(saved.privacy.excluded_apps.iter().all(|a| a.id != app.id));
    assert!(on_disk(&r.core)
        .privacy
        .excluded_apps
        .iter()
        .all(|a| a.id != app.id));

    let off = !saved.privacy.filter_system_audio;
    let mut next = saved;
    next.privacy.filter_system_audio = off;
    backend.save_privacy(&next).unwrap();
    assert_eq!(on_disk(&r.core).privacy.filter_system_audio, off);
}

#[test]
fn the_sensitive_capture_prompt_records_shown_then_closes_on_dismiss() {
    use app_core::one_time_prompts::get_one_time_prompt_state;
    use app_core::sensitive_capture_recommendations::get_sensitive_capture_recommendations;
    use settings::privacy_prompt::{Mark, PromptData};
    let r = rig();
    let backend = settings::data::CoreBackend(r.core.clone());
    let read = || {
        let recs = get_sensitive_capture_recommendations(&r.core, &[]);
        PromptData::from_core(&recs, &get_one_time_prompt_state(&r.core))
    };
    let open = read();
    assert!(
        !open.closed && !open.apps.is_empty(),
        "a fresh install sees it"
    );
    assert!(settings::privacy_prompt::view(Some(&open), false, &[]).is_some());

    backend.prompt(&open.prompt_id, Mark::Shown).unwrap();
    assert!(!read().closed, "shown alone keeps it open");
    backend.prompt(&open.prompt_id, Mark::Dismissed).unwrap();
    assert!(read().closed, "dismissed for good");
    assert!(r
        .core
        .paths()
        .config
        .join("one-time-prompts.json")
        .is_file());
}

/// The draft carries the seeded privacy exclusions (always, on a first run), which the full save alone refuses ("Privacy rules must be changed
/// with dedicated privacy commands") — the founder's failed Finale.
#[test]
fn the_onboarding_finale_commits_settings_completes_and_requests_capture() {
    use onboarding::data::Backend as _;
    let r = rig();
    let backend = onboarding::data::CoreBackend(r.core.clone());
    let state_file = r.core.paths().config.join("onboarding-state.json");
    assert!(!state_file.exists());
    r.host.events.lock().unwrap().clear();

    let mut draft = app_core::native_capture::settings_update::get_recording_settings(&r.core);
    draft.save_directory = data_dir(&r.core);
    draft.screen_frame_rate = 1.0;
    // Audio only, with a transcription model that is not installed in the temp
    // config dir: the start is asked for and refused before any capture begins.
    draft.capture_screen = false;
    draft.capture_microphone = false;
    draft.capture_system_audio = true;
    draft.transcription.enabled = true;
    draft.transcription.system_audio_enabled = true;
    draft.transcription.provider = capture_types::AudioTranscriptionProvider::LocalWhisper;
    draft.transcription.model_id = Some("base".into());
    app_core::privacy_redaction_sources::upsert_privacy_excluded_app(
        &mut draft,
        "com.apple.Passwords",
        "Passwords",
        true,
    )
    .unwrap();

    // What the Finale used to send: the whole draft through the full save.
    let refused = app_core::native_capture::settings_update::update_recording_settings(
        app_core::recording_settings::recording_settings_request_from_settings(draft.clone()),
        &r.core,
    )
    .expect_err("the full save refuses a privacy change");
    assert!(refused.message.contains("dedicated privacy commands"));

    let start = StartNativeCaptureRequest {
        capture_screen: false,
        capture_microphone: false,
        capture_system_audio: true,
    };
    let (completed, why) = backend
        .finish(&draft, Some(start))
        .expect_err("the start is refused: its transcription model is missing");
    assert!(
        completed,
        "settings and completion land before the start: {why}"
    );
    assert!(
        why.contains("unavailable") || why.contains("speech detector"),
        "the refusal is the capture start's own: {why}"
    );

    let disk = on_disk(&r.core);
    assert_eq!(disk.screen_frame_rate, 1.0);
    assert!(disk.capture_system_audio && !disk.capture_screen);
    assert!(disk
        .privacy
        .excluded_apps
        .iter()
        .any(|a| a.bundle_id == "com.apple.passwords" || a.bundle_id == "com.apple.Passwords"));
    assert!(state_file.exists(), "onboarding is marked complete");
    let state = app_core::onboarding_state::current_onboarding_state(
        &r.core,
        &r.core
            .state::<app_core::onboarding_state::OnboardingStateStore>(),
    );
    assert!(state.is_complete());
    assert!(r
        .host
        .saw(|e| matches!(e, CoreEvent::RecordingSettingsChanged(_))));
    // Refused, so nothing is recording.
    let capture = r
        .core
        .state::<app_core::native_capture::lifecycle::NativeCaptureState>();
    assert!(!capture.lock().unwrap().runtime().is_running);

    // With nothing to record the same commit is a plain success.
    backend
        .finish(&disk, None)
        .expect("a commit without a start succeeds");
}

#[test]
fn debug_health_returns_rows() {
    let r = rig();
    let backend = small::debug::data::CoreDebug(r.core.clone());
    let got = backend.fetch(Section::Health, 1_000);
    assert_eq!(got.health_error, None);
    assert!(
        got.health.len() >= 10,
        "the rollup has a row per feature: {}",
        got.health.len()
    );
    assert!(got.health.iter().all(|row| row["feature"].is_string()));
    assert_eq!(got.fetched_at_ms, Some(1_000));
    // The capture tab's read is live too (not recording).
    assert!(backend.fetch(Section::Capture, 1_000).capture.is_some());
}

#[test]
fn debug_jobs_list_and_retry_go_through_app_core() {
    use small::debug::data::{DebugBackend as _, JobCall};
    let r = rig();
    let backend = small::debug::data::CoreDebug(r.core.clone());
    for status in [None, Some("queued"), Some("failed")] {
        let page = backend
            .jobs(&JobCall::List {
                processor: "audio_transcription".into(),
                status: status.map(str::to_string),
                limit: 25,
            })
            .expect("the jobs page");
        assert!(page["jobs"].is_array(), "{status:?}: {page}");
        assert!(page["total"].is_i64(), "{status:?}: {page}");
    }
    // The requeue path refuses a job that isn't waiting, in its own words.
    let refused = backend.jobs(&JobCall::RetryNow(987_654_321)).unwrap_err();
    assert!(!refused.is_empty());
}
