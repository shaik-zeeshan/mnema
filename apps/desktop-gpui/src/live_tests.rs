//! Live wiring: a real `AppCore` on a seeded temp dir drives each area's
//! backend door, and every `CoreEvent` goes through the real host channel and
//! `bridge::events::handle`. Nothing here is a fixture backend (except where a
//! test needs to count calls), nothing reaches the keychain or real data, and
//! no capture stream is opened.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use app_core::fixtures::{self, FixtureId};
use app_core::native_capture::app_notifications as bell;
use app_core::native_capture::lifecycle::NativeCaptureState;
use app_core::{AppCore, CoreEvent};
use capture_types::{AppearanceSetting, TurnUpdate};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::AppContext as _;
use gpui_kit::{
    px, size, Bounds, Entity, Point, TestAppContext, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use crate::bridge::events::{self, Core};
use crate::chat::{self, History};
use crate::host::{GpuiHost, HostMsg};
use crate::shell::{data as shell_data, Overlay, Shell, ShellModel};
use crate::timeline::data::{CoreSource, Source};
use crate::timeline::fixture::{Call, FixtureSource};
use crate::{keymap, native, theme};

/// The seed points one process-wide env var at its key store, so live cores run
/// one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Live {
    core: Arc<AppCore>,
    rx: async_channel::Receiver<HostMsg>,
    /// "Recording error" dialogs the host was asked to show.
    dialogs: Vec<String>,
    _dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

/// A started-enough core: the seeded capture index, the recording settings (all
/// sources off, so a start is refused before any stream opens) and every
/// default state, without the tray, the broker socket or the workers.
fn live() -> Live {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().expect("temp dir");
    // Never the real keychain: a vault read panics, and the master key is a scratch file.
    std::env::set_var("MNEMA_TRIPWIRE_NO_KEYCHAIN", "1");
    // Never the real keychain for licence tokens either: `MNEMA_TRIPWIRE_NO_KEYCHAIN`
    // doesn't cover them; this file store does (debug builds only).
    std::env::set_var(
        "MNEMA_LICENSE_TOKEN_DIR",
        std::env::temp_dir().join(format!("mnema-gpui-licence-{}", std::process::id())),
    );
    std::env::set_var("MNEMA_DEV_MASTER_KEY_FILE", dir.path().join("master.key"));
    let paths = fixtures::seed(dir.path(), FixtureId::Populated).expect("seed");
    // Two belts so app-infra can only ever open the seed: the default capture
    // root (used when no valid settings file loads) and the saved one.
    std::env::set_var("MNEMA_SAVE_DIRECTORY", &paths.data);
    let (host, rx) = GpuiHost::new();
    let core = Arc::new(AppCore::new(Arc::new(host), paths.clone()).expect("app core"));
    let mut settings = app_core::recording_settings::default_recording_settings();
    settings.save_directory = paths.data.to_string_lossy().into_owned();
    app_core::recording_settings::persist_recording_settings(&core, &settings).expect("settings");
    app_core::startup::manage_default_states(&core);
    app_core::native_capture::control::initialize_recording_settings_from_disk(&core);
    // Checked BEFORE the index opens: never real data.
    assert_eq!(
        app_core::app_infra::get_storage_location(&core).unwrap(),
        paths.data.to_string_lossy(),
        "the live core must open the seed, never real data"
    );
    app_core::app_infra::initialize(&core).expect("app infra");
    // Every source off (in memory: a saved file like that doesn't validate), so
    // app-core refuses a start before any stream opens.
    {
        let state = core.state::<app_core::recording_settings::RecordingSettingsState>();
        let mut runtime = state.lock().unwrap();
        runtime.settings.capture_screen = false;
        runtime.settings.capture_microphone = false;
        runtime.settings.capture_system_audio = false;
    }
    Live {
        core,
        rx,
        dialogs: Vec::new(),
        _dir: dir,
        _guard: guard,
    }
}

fn open_shell(
    cx: &mut TestAppContext,
    live: &Live,
    timeline: Option<Arc<dyn Source>>,
) -> (WindowHandle<Root>, Entity<Shell>) {
    let core = live.core.clone();
    // Backend answers wake tasks from app-core's tokio threads.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&keymap::Bindings::default(), cx);
        crate::bridge::tokio::init_from_handle(cx, core.runtime_handle());
        cx.set_global(Core(core));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| match timeline {
                Some(source) => {
                    Shell::with_timeline_source(ShellModel::default(), source, window, cx)
                }
                None => Shell::new(ShellModel::default(), window, cx),
            })
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), shell)
    })
}

/// Deliver what the core told the host, through the app's own router. Dialogs
/// are recorded instead of shown.
fn pump(cx: &mut TestAppContext, live: &mut Live, shell: &Entity<Shell>) {
    cx.run_until_parked();
    while let Ok(msg) = live.rx.try_recv() {
        match msg {
            HostMsg::Event(_) | HostMsg::RefreshTray => {
                let weak = shell.downgrade();
                cx.update(|cx| events::handle(msg, &weak, cx));
            }
            HostMsg::Message(dialog) => live.dialogs.push(dialog.message),
            _ => {}
        }
    }
    cx.run_until_parked();
}

/// Backend answers arrive from real threads: pump until `done`, for up to 20 s.
fn wait_for(
    cx: &mut TestAppContext,
    live: &mut Live,
    shell: &Entity<Shell>,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pump(cx, live, shell);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn set_running(core: &AppCore, on: bool) {
    // The app-core capture tests' own fake: a session flag with no streams behind it.
    let state = core.state::<NativeCaptureState>();
    state.lock().unwrap().runtime_mut().is_running = on;
}

fn running(cx: &mut TestAppContext, shell: &Entity<Shell>) -> bool {
    cx.read(|cx| shell.read(cx).model.status.running)
}

// ── Timeline ────────────────────────────────────────────────────────────────

#[test]
fn the_timeline_source_reads_the_seeded_frames_and_a_preview() {
    let live = live();
    let source = CoreSource(live.core.clone());
    let frames = source.list_frames(None, 200).unwrap();
    assert!(frames.len() >= 6, "seeded frames: {}", frames.len());
    assert!(
        frames
            .windows(2)
            .all(|w| w[0].captured_at_ms > w[1].captured_at_ms),
        "newest first"
    );
    let newest = &frames[0];
    assert_eq!(newest.app_name.as_deref(), Some("Slack"));
    assert_eq!(
        newest.captured_at_ms,
        (fixtures::fixture_now().unix_timestamp() - 4) * 1000
    );

    // Paging, the jump window, the jumper's reads.
    let older = source.list_frames(Some(newest.id), 2).unwrap();
    assert_eq!(older.len(), 2);
    assert!(older.iter().all(|f| f.id < newest.id));
    let mid = &frames[frames.len() / 2];
    let window = source.window_around(mid.id, 2, 2).unwrap();
    assert_eq!(window.frames[window.target_index].id, mid.id);
    assert!(window.has_newer && window.has_older);
    let oldest = frames.last().unwrap();
    let span = (oldest.captured_at_ms - 1000, newest.captured_at_ms + 1000);
    assert_eq!(
        source
            .latest_in_range(span.0, span.1)
            .unwrap()
            .map(|f| f.id),
        Some(newest.id)
    );
    assert_eq!(
        source.frame_times(span.0, span.1).unwrap().len(),
        frames.len()
    );

    // The exact preview is a real image on disk.
    let preview = source.frame_preview(newest.id).unwrap().expect("a preview");
    assert!(preview.file_path.is_file(), "{:?}", preview.file_path);
    assert!(preview.mime_type.starts_with("image/"));

    // The side reads answer (nothing seeded) rather than erroring.
    assert!(source.audio_segments(span.0, span.1).unwrap().is_empty());
    assert!(source.episodes(span.0, span.1).unwrap().is_empty());
    assert!(source.match_boxes(newest.id, "slack", false).is_ok());
    assert!(source.scrub_availability(span.0, span.1, false).is_ok());
    // The seed's OCR jobs finished with text but no boxes: the Svelte loader
    // calls that an invalid payload too.
    assert!(source.ocr(newest.id).is_err());
    // No captured page behind a fixture frame: a benign "nothing to open".
    assert_eq!(source.open_captured_url(newest.id), Ok(false));
    let settings = source.settings();
    assert_eq!(settings.keep_days, None);
}

#[test]
fn ocr_payloads_parse_like_frame_ocr_ts() {
    let json = r#"{"observations":[
        {"text":"Hello","confidence":0.9,"boundingBox":{"x":0.1,"y":0.2,"width":0.3,"height":0.1}},
        {"text":"no box"}]}"#;
    let boxes = crate::timeline::data::parse_ocr_payload(json).unwrap();
    assert_eq!(boxes.len(), 1);
    // Lower-left origin flipped to top-left: 1 - 0.2 - 0.1.
    assert!((boxes[0].y - 0.7).abs() < 1e-6);
    assert_eq!(boxes[0].text, "Hello");
    assert!(crate::timeline::data::parse_ocr_payload("{}").is_none());
}

#[gpui_kit::test]
fn the_timeline_view_shows_live_frames_and_reacts_to_a_delete(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    let timeline = cx.read(|cx| shell.read(cx).timeline.clone());
    let seeded = CoreSource(live.core.clone())
        .list_frames(None, 200)
        .unwrap();

    wait_for(cx, &mut live, &shell, "the first page", |cx| {
        cx.read(|cx| timeline.read(cx).paging.frames.len() == seeded.len())
    });
    let newest = seeded[0].id;
    wait_for(cx, &mut live, &shell, "the exact preview", |cx| {
        cx.read(|cx| timeline.read(cx).engine.exact(newest).is_some())
    });
    let path = cx.read(|cx| {
        timeline
            .read(cx)
            .engine
            .exact(newest)
            .unwrap()
            .file_path
            .clone()
    });
    assert!(path.is_file());

    // `timeline_data_changed` (retention / delete recent): the frame leaves the rail.
    let payload = app_core::app_infra::TimelineDataChangedPayload {
        reason: "delete_recent_capture".into(),
        deleted_before: None,
        started_at: None,
        ended_at: None,
        deleted_frame_ids: vec![newest],
        deleted_audio_segment_ids: Vec::new(),
    };
    live.core
        .host()
        .emit(CoreEvent::TimelineDataChanged(payload));
    wait_for(cx, &mut live, &shell, "the deleted frame to leave", |cx| {
        cx.read(|cx| {
            let t = timeline.read(cx);
            t.paging.frames.len() == seeded.len() - 1
                && t.paging.frames.iter().all(|f| f.id != newest)
        })
    });
    let note = cx
        .read(|cx| timeline.read(cx).status.clone())
        .expect("a note");
    assert!(note.message.contains("deleted"), "{}", note.message);
}

#[gpui_kit::test]
fn audio_and_scrub_cache_events_refetch_the_timeline(cx: &mut TestAppContext) {
    let mut live = live();
    // A call-recording source, so each event's refetch can be counted.
    let source = Arc::new(FixtureSource::load("default").unwrap());
    let (_window, shell) = open_shell(cx, &live, Some(source.clone()));
    pump(cx, &mut live, &shell);
    let count = |c: Call| source.calls().iter().filter(|x| **x == c).count();

    let audio = count(Call::Audio);
    live.core.host().emit(CoreEvent::AudioSegmentsChanged);
    pump(cx, &mut live, &shell);
    assert_eq!(count(Call::Audio), audio + 1, "audio_segments_changed");

    // The playhead's exact image isn't in (the fixture decodes nothing here), so
    // a grown scrub cache re-asks for the stills around it after the throttle.
    let timeline = cx.read(|cx| shell.read(cx).timeline.clone());
    let active = cx.read(|cx| timeline.read(cx).paging.active_frame().map(|f| f.id));
    let exact = cx.read(|cx| active.and_then(|id| timeline.read(cx).engine.exact(id).cloned()));
    let scrub = count(Call::Scrub { enqueue: true });
    let payload = app_core::app_infra::frame_preview::ScrubPreviewCacheChangedPayload {
        rendition: "scrub".into(),
        start_unix_ms: 0,
        end_unix_ms: 1,
    };
    live.core
        .host()
        .emit(CoreEvent::ScrubPreviewCacheChanged(payload));
    pump(cx, &mut live, &shell);
    cx.executor().advance_clock(Duration::from_millis(500));
    pump(cx, &mut live, &shell);
    if exact.is_none() {
        assert_eq!(
            count(Call::Scrub { enqueue: true }),
            scrub + 1,
            "scrub_preview_cache_changed"
        );
    } else {
        assert_eq!(
            count(Call::Scrub { enqueue: true }),
            scrub,
            "exact already in"
        );
    }
}

// ── Chat ────────────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn chat_lists_creates_renames_searches_and_deletes(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    let chat_view = cx.read(|cx| shell.read(cx).chat.clone());
    let core = live.core.clone();
    let rows = |cx: &mut TestAppContext| {
        cx.read(|cx| match &chat_view.read(cx).data.history {
            History::Rows(rows) => Some(
                rows.iter()
                    .map(|r| (r.conversation_id.clone(), r.title.clone(), r.pinned))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
    };

    chat_view.update(cx, |c, cx| c.reload(cx));
    wait_for(cx, &mut live, &shell, "the empty list", |cx| {
        rows(cx) == Some(Vec::new())
    });
    // Ask AI is off in a fresh install: the composer learns why.
    let availability = cx
        .read(|cx| chat_view.read(cx).data.availability.clone())
        .expect("availability");
    assert!(!availability.available);
    assert_eq!(availability.reason.as_deref(), Some("ask_ai_disabled"));

    // Create (a pin on a chat with no turn yet creates its row), then rename.
    // Every write emits `conversation_changed`, which reloads the list.
    let id = "live-chat-1";
    core.block_on(chat::data::set_pinned(&core, id, true))
        .unwrap();
    wait_for(cx, &mut live, &shell, "the new chat", |cx| {
        rows(cx).is_some_and(|r| r.iter().any(|(i, _, pinned)| i == id && *pinned))
    });
    core.block_on(chat::data::set_title(&core, id, "Quarterly numbers"))
        .unwrap();
    wait_for(cx, &mut live, &shell, "the rename", |cx| {
        rows(cx).is_some_and(|r| r.iter().any(|(_, title, _)| title == "Quarterly numbers"))
    });

    // Open it (`get_conversation`); nothing is streaming (`ask_ai_snapshot`).
    let opened = core
        .block_on(chat::data::open(&core, id))
        .unwrap()
        .expect("the conversation");
    assert_eq!(opened.title, "Quarterly numbers");
    assert!(core.block_on(chat::data::snapshot(&core, id)).is_none());
    assert_eq!(core.block_on(chat::data::open(&core, "nope")), Ok(None));

    // Search goes to the store (`search_conversations`).
    let found = core.block_on(chat::data::load(&core, "quarterly"));
    assert_eq!(found.query, "quarterly");
    assert!(matches!(&found.history, History::Rows(r) if r.len() == 1));
    let none = core.block_on(chat::data::load(&core, "zebra"));
    assert_eq!(none.history, History::Rows(Vec::new()));

    // The engine pin, and a send that app-core refuses (Ask AI is off).
    core.block_on(chat::data::set_engine(
        &core,
        id,
        Some(("p1".into(), "m1".into())),
    ));
    let pinned = core.block_on(chat::data::open(&core, id)).unwrap().unwrap();
    assert_eq!(pinned.model.as_deref(), Some("m1"));
    let refused = core.block_on(chat::data::send(
        &core,
        chat::data::SendRequest {
            conversation_id: id.into(),
            question: "What changed?".into(),
            title: Some("Quarterly numbers".into()),
            first: true,
            scope: chat::Scope::default().to_wire(time::UtcOffset::UTC),
        },
    ));
    assert!(refused.is_err(), "Ask AI is off: {refused:?}");
    core.block_on(chat::data::cancel(&core, id));
    // No answer to delete: the store says so instead of pretending.
    assert!(core
        .block_on(chat::data::delete_last_turn(&core, id, 0))
        .is_err());

    core.block_on(chat::data::delete(&core, id)).unwrap();
    wait_for(cx, &mut live, &shell, "the delete", |cx| {
        rows(cx) == Some(Vec::new())
    });
}

#[gpui_kit::test]
fn ask_ai_updates_reach_the_open_chat(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    let chat_view = cx.read(|cx| shell.read(cx).chat.clone());
    chat_view.update(cx, |c, cx| {
        c.active = Some("c1".into());
        c.pane = chat::Pane::Thread;
        let mut turn = chat::ChatTurn::new(0, "What did I ship?", "streaming");
        turn.version = 1;
        c.turns.push(turn);
        c.streaming = true;
        cx.notify();
    });
    let update = |version, update| {
        CoreEvent::AskAiUpdate(chat::AskAiUpdateEvent {
            conversation_id: "c1".into(),
            version,
            turn_index: 0,
            update,
        })
    };
    let host = live.core.host();
    host.emit(update(
        2,
        TurnUpdate::AppendProse {
            text: "Shipped it.".into(),
        },
    ));
    host.emit(update(3, TurnUpdate::Done));
    pump(cx, &mut live, &shell);
    let (text, streaming) = cx.read(|cx| {
        let c = chat_view.read(cx);
        (c.turns[0].plain_text(), c.streaming)
    });
    assert_eq!(text, "Shipped it.");
    assert!(!streaming);
}

// ── Shell ───────────────────────────────────────────────────────────────────

#[test]
fn the_shell_model_reads_the_seeded_storage_and_the_session() {
    let live = live();
    let core = &live.core;
    let model = core.block_on(shell_data::load(core));
    let s = &model.status;
    assert!(!s.running && s.runtime.is_none());
    assert_eq!(s.selected, [false; 3], "the saved sources");
    assert!(
        s.storage_bytes.is_some_and(|b| b > 0),
        "the seed is on disk"
    );
    assert!(s.free_bytes.is_some_and(|b| b > 0));
    assert_eq!(s.tracked_ms, Some(0), "the seed isn't today");
    assert!(!s.read_only, "debug builds are licensed");
    assert_eq!(
        s.engine.as_ref().map(|e| e.label),
        Some("not set up"),
        "no AI provider"
    );
    assert!(s.local_only);
    assert!(s.screen_permission.is_some());
    assert!(model.notifications.is_empty());

    // A running session (the capture tests' fake) shows as REC with its sources.
    set_running(core, true);
    let model = core.block_on(shell_data::load(core));
    assert!(model.status.running && model.status.runtime.is_some());
    let mut status = model.status.clone();
    set_running(core, false);
    shell_data::refresh_live(core, &mut status);
    assert!(!status.running && status.runtime.is_none());

    // The 2 s feed reads app-core directly: capture health (free space on the
    // seeded root, no thresholds while stopped) and the popover's backlog.
    let health = shell_data::health(core).expect("capture health");
    assert!(health.disk.free_bytes.is_some_and(|b| b > 0));
    assert_eq!(health.disk.pause_threshold_bytes, None);
    let backlog = shell_data::backlog(core, true);
    assert!(backlog.index.is_some() && backlog.pipeline.is_some());
    assert_eq!(backlog.deepgram_auth_error, None, "nothing retrying");
}

#[gpui_kit::test]
fn record_reaches_capture_control_and_a_refusal_is_never_silent(cx: &mut TestAppContext) {
    let mut live = live();
    let (window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "nothing", |_| true);

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("status-record", cx);
    })
    .unwrap();
    // app-core's start validation refuses (every source is off) before any stream opens.
    let refusal = "At least one capture source must be enabled";
    wait_for(cx, &mut live, &shell, "the refused start", |cx| {
        cx.read(|cx| {
            let s = shell.read(cx);
            s.model.status.start_error.is_some() && !s.model.status.loading_start
        })
    });
    let (error, flag) = cx.read(|cx| {
        let s = &shell.read(cx).model.status;
        (
            s.start_error.clone(),
            crate::shell::model::status_flag(s).map(|f| f.action),
        )
    });
    assert_eq!(error.as_deref(), Some(refusal));
    assert_eq!(flag.as_deref(), Some("Retry"));
    // The bell row carries the message, and the "Recording error" dialog was asked for.
    wait_for(cx, &mut live, &shell, "the bell row", |cx| {
        cx.read(|cx| {
            shell
                .read(cx)
                .model
                .notifications
                .iter()
                .any(|n| n.id == native::CAPTURE_COMMAND_FAILED_ID && n.message == refusal)
        })
    });
    assert_eq!(live.dialogs, vec![refusal.to_string()]);
    assert!(
        !app_core::native_capture::control::current_native_capture_session(&live.core).is_running
    );

    // The tray's and the global shortcut's path is the same call, and refuses the same way.
    let Err((title, e)) = native::toggle_recording(&live.core, "global-shortcut") else {
        panic!("a start with no sources must be refused");
    };
    assert_eq!(
        (title, e.message.as_str()),
        ("Recording could not start", refusal)
    );
    // Pause on a stopped session is a no-op, not an error.
    assert!(native::toggle_pause(&live.core).is_ok());

    // Clearing one row, then all (`clear_app_notification(s)`).
    bell::push_warning_app_notification(&live.core, "w1", "Heads up", "Body", None, 1);
    wait_for(cx, &mut live, &shell, "the second row", |cx| {
        cx.read(|cx| shell.read(cx).model.notifications.len() == 2)
    });
    shell.update(cx, |s, cx| s.clear_notifications(Some("w1".into()), cx));
    wait_for(cx, &mut live, &shell, "one row cleared", |cx| {
        cx.read(|cx| shell.read(cx).model.notifications.len() == 1)
    });
    shell.update(cx, |s, cx| s.clear_notifications(None, cx));
    wait_for(cx, &mut live, &shell, "all rows cleared", |cx| {
        cx.read(|cx| shell.read(cx).model.notifications.is_empty())
    });
    assert!(bell::get_app_notifications(&live.core).is_empty());
}

#[gpui_kit::test]
fn every_shell_event_reloads_the_model(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "nothing", |_| true);
    let null = serde_json::Value::Null;
    let events = [
        CoreEvent::NativeCaptureSessionChanged(null.clone()),
        CoreEvent::RecordingSettingsChanged(null.clone()),
        CoreEvent::RecordingSettingsDomainChanged(null.clone()),
        CoreEvent::AppNotificationsChanged(null.clone()),
        CoreEvent::LicenseStatus(null.clone()),
        CoreEvent::KeyboardBindingsSettingsChanged(null),
    ];
    // Each event must make the shell re-read the session: flip it underneath, emit.
    let mut on = false;
    for event in events {
        on = !on;
        set_running(&live.core, on);
        assert_ne!(running(cx, &shell), on);
        let name = format!("{event:?}");
        live.core.host().emit(event);
        wait_for(cx, &mut live, &shell, &name, |cx| running(cx, &shell) == on);
    }
    set_running(&live.core, false);

    // A saved source toggle comes back through `recording_settings_domain_changed`.
    cx.update(|cx| shell.update(cx, |s, cx| s.save_source(0, true, cx)));
    wait_for(cx, &mut live, &shell, "the saved source", |cx| {
        cx.read(|cx| shell.read(cx).model.status.selected[0])
    });
    assert!(
        app_core::native_capture::control::current_recording_settings_from_core(&live.core)
            .capture_screen
    );

    // Window focus stamps the local UTC offset for User Context.
    cx.update(|cx| shell.update(cx, |s, cx| s.on_focus(cx)));
    let infra = live.core.state::<app_core::AppInfraState>();
    let expected = i64::from(native::sys::local_offset_seconds() / 60);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stamped = live
            .core
            .block_on(infra.user_context().local_offset_minutes())
            .ok()
            .flatten();
        if stamped == Some(expected) {
            break;
        }
        assert!(Instant::now() < deadline, "the offset was never stamped");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `StatusPop.svelte`'s foot: "Open developer window" only with developer
/// options on; it closes the popover and opens the window.
#[gpui_kit::test]
fn the_status_popover_opens_the_developer_window_with_developer_options(cx: &mut TestAppContext) {
    let mut live = live();
    let (window, shell) = open_shell(cx, &live, None);
    wait_for(cx, &mut live, &shell, "the first load", |_| true);
    let has = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            w.try_find("status-pop-debug").is_some()
        })
        .unwrap()
    };
    let toggle = |cx: &mut TestAppContext| {
        cx.update(|cx| shell.update(cx, |s, cx| s.toggle_overlay(Overlay::Status, cx)));
        cx.run_until_parked();
    };
    toggle(cx);
    assert!(!has(cx), "developer options off: no button");
    toggle(cx);

    {
        let state = live
            .core
            .state::<app_core::recording_settings::RecordingSettingsState>();
        state.lock().unwrap().settings.developer_options_enabled = true;
    }
    live.core
        .host()
        .emit(CoreEvent::RecordingSettingsChanged(serde_json::Value::Null));
    wait_for(cx, &mut live, &shell, "developer options on", |cx| {
        cx.read(|cx| shell.read(cx).model.developer_options)
    });
    toggle(cx);
    assert!(has(cx), "developer options on: the button");
    let before = cx.read(|cx| cx.windows().len());
    cx.update_window(window.into(), |_, w, cx| w.click("status-pop-debug", cx))
        .unwrap();
    pump(cx, &mut live, &shell);
    assert_eq!(
        cx.read(|cx| shell.read(cx).overlay),
        None,
        "the popover closed"
    );
    assert_eq!(
        cx.read(|cx| cx.windows().len()),
        before + 1,
        "the developer window opened"
    );
}

// ── Settings ────────────────────────────────────────────────────────────────

/// Opening Settings lands the local reads (mic state, capture support,
/// permissions) with the recording settings, before any key store answers.
#[gpui_kit::test]
fn opening_settings_lands_the_mic_state_and_capture_support(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    cx.update(|cx| shell.update(cx, |s, cx| s.go(crate::shell::Surface::Settings, cx)));
    let settings = cx.read(|cx| shell.read(cx).settings.clone());
    wait_for(cx, &mut live, &shell, "the settings load", |cx| {
        cx.read(|cx| settings.read(cx).draft.is_some())
    });
    cx.read(|cx| {
        let s = settings.read(cx);
        assert!(s.mic_draft.is_some(), "Microphone: couldn't load");
        assert!(s.data.capture_support.is_some(), "Video: checking support");
        assert!(s.data.permissions.is_some());
    });
    // The vault/keychain/network reads land second, without touching those.
    wait_for(cx, &mut live, &shell, "the slow reads", |cx| {
        cx.read(|cx| settings.read(cx).data.complete)
    });
    cx.read(|cx| {
        let s = settings.read(cx);
        assert!(s.mic_draft.is_some() && s.data.capture_support.is_some());
        assert!(s.data.license.is_some(), "the seed's license status");
    });
}

/// A source switched from the tray (no Settings involved) reaches an open
/// Settings page: the router hands the settings events to it too.
#[gpui_kit::test]
fn an_open_settings_page_follows_a_tray_source_switch(cx: &mut TestAppContext) {
    let mut live = live();
    let (_window, shell) = open_shell(cx, &live, None);
    cx.update(|cx| shell.update(cx, |s, cx| s.go(crate::shell::Surface::Settings, cx)));
    let settings = cx.read(|cx| shell.read(cx).settings.clone());
    wait_for(cx, &mut live, &shell, "the settings load", |cx| {
        cx.read(|cx| settings.read(cx).draft.is_some())
    });
    assert!(!cx.read(|cx| settings.read(cx).d().capture_screen));

    let sources = capture_types::CaptureSources {
        screen: true,
        microphone: false,
        system_audio: false,
    };
    app_core::native_capture::settings_update::update_recording_sources_from_core(
        &live.core, sources,
    )
    .expect("saved");
    wait_for(cx, &mut live, &shell, "Settings re-reads", |cx| {
        cx.read(|cx| settings.read(cx).d().capture_screen)
    });
}

// ── Quick Recall ────────────────────────────────────────────────────────────

/// The embedder's state comes from app-core's default states (the Tauri builder
/// used to be the only one registering it, so every GPUI search failed).
#[test]
fn quick_recall_searches_the_seeded_index() {
    use crate::quick_recall::data::{Backend, CoreBackend};
    let live = live();
    let backend = CoreBackend(live.core.clone());
    // No semantic model installed: keyword-only, an honest answer either way.
    let hit = backend.search("slack", 24, 8).expect("a match");
    assert!(!hit.index_empty);
    let miss = backend
        .search("arsenal", 24, 8)
        .expect("no match is not a failure");
    assert!(miss.frames.is_empty() && miss.audio.is_empty() && !miss.index_empty);
    assert!(backend
        .facet("slack", app_infra::SearchFacet::App)
        .is_some());
}
