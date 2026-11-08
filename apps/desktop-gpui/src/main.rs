//! `mnema-gpui`: opens the main window on a real AppCore.
//!
//! Run it through `scripts/dev-gpui.sh`, which points `MNEMA_SAVE_DIRECTORY` /
//! `MNEMA_APP_CONFIG_DIR` at a copy of the dev sandbox.

use std::sync::Arc;

use app_core::{AppCore, Paths};
use capture_types::AppearanceSetting;
use gpui_kit::{actions, KeyBinding, Menu, MenuItem};
use mnema_gpui::bridge::events::{self, Core};
use mnema_gpui::host::GpuiHost;
use mnema_gpui::shell::ShellModel;
use mnema_gpui::{assets::MnemaAssets, feel, keymap, logging, theme, window, BUNDLE_ID};

actions!(app, [Quit]);

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

    app.run(move |cx| {
        gpui_kit::init(cx);
        // shortcut: the persisted appearance is in recording settings, not in
        // app-core until S2; follow the OS until then.
        theme::init(AppearanceSetting::System, cx.window_appearance(), cx);
        theme::reduce_motion::observe(cx);
        feel::observe_keystrokes(cx);

        let (host, host_rx) = GpuiHost::new();
        let core = match AppCore::new(Arc::new(host), paths) {
            Ok(core) => Arc::new(core),
            Err(e) => {
                log::error!("app-core runtime failed to start: {e}");
                cx.quit();
                return;
            }
        };
        mnema_gpui::bridge::tokio::init_from_handle(cx, core.runtime_handle());
        cx.set_global(Core(core));
        // shortcut: nothing opens the DB yet. Wiring `startup::manage_default_states`,
        // `AppCore::start` / `spawn_deferred_startup` and Quit → `AppCore::shutdown`
        // waits for the already-running dialog, the OS notifiers and the
        // `terminate:` swizzle (S7).

        keymap::bind(&keymap::Bindings::default(), cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("Mnema").items([MenuItem::action("Quit Mnema", Quit)])]);

        let shell = match window::open_main(ShellModel::default(), cx) {
            Ok(shell) => shell,
            Err(e) => {
                log::error!("main window failed to open: {e:#}");
                cx.quit();
                return;
            }
        };
        feel::milestone("main_window_opened");
        events::spawn(host_rx, shell.downgrade(), cx);
        events::reload_shell(&shell, cx);

        cx.spawn(async move |_| {
            while let Ok(urls) = url_rx.recv().await {
                // shortcut: deep-link routing is S7 (`deep_links.rs`). Count only: URLs carry ids.
                log::info!(
                    "deep link received ({} url(s)); routing lands in S7",
                    urls.len()
                );
            }
        })
        .detach();
    });
}
