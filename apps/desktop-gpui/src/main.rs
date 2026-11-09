//! `mnema-gpui`: the startup order of BUILD-PROMPT §3.2 on a real AppCore.
//! `AppCore::start` (with the tray, global shortcuts and OS observers as its
//! host callbacks) → open the main window → `spawn_deferred_startup`.
//!
//! Run it as the dev bundle (`scripts/bundle-gpui-app.sh --dev`) or through
//! `scripts/dev-gpui.sh`; both point `MNEMA_SAVE_DIRECTORY` /
//! `MNEMA_APP_CONFIG_DIR` at a copy of the dev sandbox.

use std::sync::Arc;

use app_core::{AppCore, Paths, StartError};
use capture_types::AppearanceSetting;
use gpui_kit::{actions, App, KeyBinding, Menu, MenuItem};
use mnema_gpui::bridge::events::{self, Core};
use mnema_gpui::host::GpuiHost;
use mnema_gpui::native::{self, deep_links, hotkeys, notification, sys, tray};
use mnema_gpui::shell::{DeveloperOptions, ShellModel};
use mnema_gpui::{assets::MnemaAssets, feel, keymap, logging, theme, window, BUNDLE_ID};

actions!(app, [Quit]);

const ALREADY_RUNNING_MESSAGE: &str =
    "Mnema is already running. Close the existing Mnema window before opening it again.";

fn main() {
    // Must run first: the speaker-analysis worker re-execs this binary as a helper.
    app_core::speaker_analysis_runtime::maybe_run_subprocess_helper_and_exit();
    feel::mark_start();
    let paths = Paths::from_env(BUNDLE_ID);
    logging::init(&paths.log);
    log::info!("mnema-gpui starting");

    let (url_tx, url_rx) = async_channel::unbounded::<Vec<String>>();
    let app = gpui_kit::application().with_assets(MnemaAssets);
    // No `&mut App` here: forward, handle in a cx.spawn loop.
    app.on_open_urls(move |urls| {
        let _ = url_tx.try_send(urls);
    });
    // A Dock click or Finder relaunch with nothing visible opens main
    // (onboarding, while it isn't done: `ensure_main`).
    app.on_reopen(|cx| {
        window::ensure_main(cx);
    });

    app.run(move |cx| {
        gpui_kit::init(cx);
        // shortcut: the persisted appearance is in recording settings; Settings ›
        // Appearance applies it once loaded. Follow the OS until then.
        theme::init(AppearanceSetting::System, cx.window_appearance(), cx);
        theme::reduce_motion::observe(cx);
        mnema_gpui::timeline::model::set_24_hour(
            app_core::clock_format::system_uses_24_hour_time().unwrap_or(false),
        );
        feel::observe_keystrokes(cx);

        let (host, host_rx) = GpuiHost::new();
        let core = match AppCore::new(Arc::new(host), paths) {
            Ok(core) => Arc::new(core),
            Err(e) => {
                log::error!("app-core runtime failed to start: {e}");
                native::quit::exit_now();
            }
        };
        app_core::startup::manage_default_states(&core);
        mnema_gpui::bridge::tokio::init_from_handle(cx, core.runtime_handle());
        cx.set_global(Core(core.clone()));
        native::quit::install(core.clone());

        let (tray_tx, tray_rx) = async_channel::unbounded::<String>();
        let (key_tx, key_rx) = async_channel::unbounded::<u32>();
        let mut handles = None;
        let started = core.start(
            |core| {
                let tray = tray::create(core, tray_tx, cx)?;
                handles = Some((tray, hotkeys::init(core, key_tx)));
                Ok(())
            },
            sys::register_core_notifiers,
        );
        let report = match started {
            Ok(report) => report,
            Err(StartError::AlreadyRunning) => {
                already_running(cx);
                return;
            }
            Err(StartError::Failed(e)) => {
                log::error!("startup failed: {e}");
                native::quit::exit_now();
            }
        };
        if let Some((tray, keys)) = handles {
            cx.set_global(tray);
            hotkeys::install(keys, cx);
        }
        sys::observe_sleep_wake(&core, cx);
        notification::on_response(core.clone(), cx);
        spawn_os_events(core.clone(), tray_rx, key_rx, cx);
        cx.on_window_closed(|cx, _| native::dock::refresh(cx))
            .detach();

        keymap::bind(&keymap::Bindings::default(), cx);
        let quit_core = core.clone();
        cx.on_action(move |_: &Quit, _| quit_core.shutdown(app_core::ExitKind::Quit));
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("Mnema").items([MenuItem::action("Quit Mnema", Quit)])]);

        // sharingType is decided before the first show, from the saved setting.
        let developer = native::developer_options(&core);
        cx.set_global(DeveloperOptions(developer));
        // Before onboarding completes, the onboarding window opens instead of main;
        // its Finale opens main (`window::ensure_main`).
        if report.onboarding_complete {
            let shell = match window::open_main(
                ShellModel {
                    developer_options: developer,
                    ..Default::default()
                },
                cx,
            ) {
                Ok(shell) => shell,
                Err(e) => {
                    log::error!("main window failed to open: {e:#}");
                    native::quit::exit_now();
                }
            };
            feel::milestone("main_window_opened");
            events::reload_shell(&shell, cx);
        } else {
            mnema_gpui::onboarding::open(cx);
            feel::milestone("onboarding_window_opened");
        }
        core.spawn_deferred_startup(&report);
        tray::refresh(cx);
        events::spawn(host_rx, cx);
        deep_links::spawn(core, url_rx, cx);
    });
}

/// Tray clicks and shortcut presses arrive on OS threads; handle them here.
fn spawn_os_events(
    core: Arc<AppCore>,
    tray_rx: async_channel::Receiver<String>,
    key_rx: async_channel::Receiver<u32>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Ok(id) = tray_rx.recv().await {
            let _ = cx.update(|cx| tray::handle(&id, cx));
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(id) = key_rx.recv().await {
            let _ = cx.update(|cx| hotkeys::pressed(&core, id, cx));
        }
    })
    .detach();
}

/// Another Mnema holds the data lock: say so, then exit. The alert runs from a
/// spawned task, outside any `App` borrow (a modal loop inside one panics).
fn already_running(cx: &mut App) {
    log::warn!("another Mnema instance holds the data directory lock; exiting");
    cx.activate(true);
    cx.spawn(async move |_| {
        native::dialogs::run_modal_alert("Mnema is already running", ALREADY_RUNNING_MESSAGE);
        native::quit::exit_now();
    })
    .detach();
}
