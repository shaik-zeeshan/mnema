//! S0 spike binary. Env knobs: SPIKE_THEME=light|dark, SPIKE_SHARING=readonly,
//! SPIKE_FONTS=variable, SPIKE_HIDE_ALL_AFTER_MS=<ms>, SPIKE_LOG=<file>.

use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::{
    actions, AnyWindowHandle, App, AppContext, Entity, KeyBinding, Menu, MenuItem, WindowAppearance,
};
use mnema_gpui::bridge::tokio::Tokio;
use mnema_gpui::spike::main_view::{self, MainView};
use mnema_gpui::spike::theme::Mode;
use mnema_gpui::spike::{fonts, log, mark_start, native, panel};
use tray_icon::menu::{ContextMenu as _, Menu as TrayMenu, MenuEvent, MenuItem as TrayMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

actions!(spike, [Quit]);

enum Event {
    Tray(String),
    Hotkey,
    Urls(Vec<String>),
}

fn main() {
    mark_start();
    log("main: start");
    let (tx, rx) = async_channel::unbounded::<Event>();

    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    let url_tx = tx.clone();
    // No `&mut App` here: forward into the channel, handle in a cx.spawn loop.
    app.on_open_urls(move |urls| {
        let _ = url_tx.try_send(Event::Urls(urls));
    });

    app.run(move |cx| {
        gpui_kit::init(cx);
        mnema_gpui::bridge::tokio::init(cx);
        let variable_only = std::env::var("SPIKE_FONTS").as_deref() == Ok("variable");
        if let Err(e) = fonts::register(cx, variable_only) {
            log(format!("fonts: register failed: {e:#}"));
        }
        for family in [fonts::UI, fonts::MONO] {
            let (ids, fallback) = fonts::resolved_ids(cx, family);
            log(format!(
                "fonts: variable_only={variable_only} {family} 400/500/600 -> {:?}/{:?}/{:?} fallback={:?} distinct={}",
                ids[0], ids[1], ids[2], fallback, fonts::weights_resolve(cx, family)
            ));
        }

        let mode = match std::env::var("SPIKE_THEME").as_deref() {
            Ok("light") => Mode::Light,
            Ok("dark") => Mode::Dark,
            _ => match cx.window_appearance() {
                WindowAppearance::Light | WindowAppearance::VibrantLight => Mode::Light,
                _ => Mode::Dark,
            },
        };

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("Mnema Spike").items([MenuItem::action("Quit Mnema Spike", Quit)])]);

        let quit_rx = native::install_terminate_handler();
        cx.on_app_quit(|_| async {
            log("gpui on_app_quit fired");
        })
        .detach();

        let mut main_entity = None;
        let main = gpui_kit::open_window(main_view::window_options(cx), cx, |window, cx| {
            let sharing = native::apply_sharing_type(window);
            log(format!("main created hidden; sharingType set before first show: {:?}", sharing.map(|s| s.0)));
            window.activate_window();
            let view = cx.new(|_| MainView::new(mode));
            main_entity = Some(view.clone());
            view
        });
        let (main_window, _) = main.expect("main window");
        let main_entity = main_entity.expect("main view");

        let panel_window: AnyWindowHandle = panel::open(cx, mode).expect("panel").into();
        let _ = panel_window.update(cx, |_, window, _| {
            window.activate_window();
        });

        // Report window facts once AppKit has ordered them front.
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = cx.update(|cx| {
                for (name, handle) in [("main", main_window), ("panel", panel_window)] {
                    let _ = handle.update(cx, |_, window, _| {
                        log(format!(
                            "{name}: windowNumber={} visible={} {}",
                            native::window_number(window),
                            native::is_visible(window),
                            native::panel_report(window)
                        ));
                    });
                }
            });
        })
        .detach();

        start_db_query(main_entity.clone(), cx);
        let (tray, tray_menu) = start_tray(tx.clone()).unzip();
        if let (Some(menu), Ok(_)) = (tray_menu.clone(), std::env::var("SPIKE_TRAY_POPUP")) {
            // Test hook: Ice/AX can't click a status item, so pop its menu up here.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(1500)).await;
                // Not inside cx.update: a nested modal run loop re-enters GPUI's
                // menu validation, which borrows App and panics if we hold it.
                log("tray: popping up the status-item menu (test hook)");
                native::pop_up_menu(menu.ns_menu(), 400., 700.);
                log("tray: menu closed");
            })
            .detach();
        }
        let hotkeys = start_hotkey(tx.clone());
        spawn_event_loop(rx, main_entity.clone(), [main_window, panel_window], cx);
        if let Some(quit_rx) = quit_rx {
            spawn_graceful_quit(quit_rx, main_entity.clone(), cx);
        }
        if let Some(ms) = std::env::var("SPIKE_HIDE_ALL_AFTER_MS").ok().and_then(|v| v.parse().ok()) {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                let _ = cx.update(|cx| hide_all(cx, [main_window, panel_window]));
            })
            .detach();
        }
        // Keep the OS handles alive for the app's lifetime.
        cx.set_global(OsHandles {
            _tray: tray,
            _tray_menu: tray_menu,
            _hotkeys: hotkeys,
        });
    });
}

struct OsHandles {
    _tray: Option<TrayIcon>,
    _tray_menu: Option<TrayMenu>,
    _hotkeys: Option<GlobalHotKeyManager>,
}
impl gpui_kit::Global for OsHandles {}

fn hide_all(cx: &mut App, windows: [AnyWindowHandle; 2]) {
    for handle in windows {
        let _ = handle.update(cx, |_, window, _| native::order_out(window));
    }
    log("hide_all: every window ordered out");
}

fn start_db_query(view: Entity<MainView>, cx: &mut App) {
    let query = Tokio::spawn(cx, async {
        let dir = std::env::temp_dir().join(format!("mnema-gpui-spike-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let url = format!("sqlite://{}?mode=rwc", dir.join("spike.sqlite3").display());
        let pool = sqlx::SqlitePool::connect(&url).await?;
        let row: (String, i64) = sqlx::query_as("select sqlite_version(), 6 * 7")
            .fetch_one(&pool)
            .await?;
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
        anyhow::Ok(format!(
            "sqlx 0.8 on tokio thread {:?}: sqlite_version()={} 6*7={}",
            std::thread::current().name().unwrap_or("?"),
            row.0,
            row.1
        ))
    });
    view.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            let status = match query.await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => format!("sqlx error: {e:#}"),
                Err(e) => format!("tokio join error: {e}"),
            };
            log(format!("db: {status}"));
            let _ = this.update(cx, |v, cx| {
                v.db_status = status.into();
                cx.notify();
            });
        })
        .detach();
    });
}

/// A 32×32 template icon (alpha only): a ring.
fn tray_icon_image() -> Icon {
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - 15.5).powi(2) + (y as f32 - 15.5).powi(2)).sqrt();
            if (9.0..13.0).contains(&d) {
                rgba[(y * n + x) * 4 + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, n as u32, n as u32).expect("valid icon")
}

fn start_tray(tx: async_channel::Sender<Event>) -> Option<(TrayIcon, TrayMenu)> {
    let menu = TrayMenu::new();
    for (id, label) in [
        ("tray_toggle_panel", "Toggle Panel"),
        ("tray_hide_all", "Hide All Windows"),
        ("tray_quit", "Quit Spike"),
    ] {
        menu.append(&TrayMenuItem::with_id(id, label, true, None))
            .ok()?;
    }
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu.clone()))
        .with_icon_templated(tray_icon_image())
        .with_tooltip("Mnema spike")
        .build();
    let tray = match tray {
        Ok(t) => t,
        Err(e) => {
            log(format!("tray: build failed: {e}"));
            return None;
        }
    };
    let receiver = MenuEvent::receiver().clone();
    std::thread::Builder::new()
        .name("tray-menu-forwarder".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if tx.send_blocking(Event::Tray(event.id.0)).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    log("tray: installed (3 items)");
    Some((tray, menu))
}

fn start_hotkey(tx: async_channel::Sender<Event>) -> Option<GlobalHotKeyManager> {
    let manager = GlobalHotKeyManager::new().ok()?;
    let hotkey = HotKey::new(Some(Modifiers::META | Modifiers::ALT), Code::Space);
    if let Err(e) = manager.register(hotkey) {
        log(format!("hotkey: register failed: {e}"));
        return None;
    }
    let receiver = GlobalHotKeyEvent::receiver().clone();
    std::thread::Builder::new()
        .name("hotkey-forwarder".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if event.state == HotKeyState::Pressed && tx.send_blocking(Event::Hotkey).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    log("hotkey: registered cmd-opt-space");
    Some(manager)
}

fn toggle_panel(cx: &mut App, panel: AnyWindowHandle) -> bool {
    panel
        .update(cx, |_, window, _| {
            if native::is_visible(window) {
                native::order_out(window);
                false
            } else {
                window.activate_window();
                true
            }
        })
        .unwrap_or(false)
}

fn spawn_event_loop(
    rx: async_channel::Receiver<Event>,
    view: Entity<MainView>,
    windows: [AnyWindowHandle; 2],
    cx: &mut App,
) {
    let panel = windows[1];
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            let _ = cx.update(|cx| {
                let line = match event {
                    Event::Tray(id) => {
                        match id.as_str() {
                            "tray_toggle_panel" => {
                                toggle_panel(cx, panel);
                            }
                            "tray_hide_all" => hide_all(cx, windows),
                            "tray_quit" => cx.quit(),
                            _ => {}
                        }
                        format!("tray: {id}")
                    }
                    Event::Hotkey => format!(
                        "hotkey: cmd-opt-space -> panel visible={}",
                        toggle_panel(cx, panel)
                    ),
                    Event::Urls(urls) => format!("deep link: {}", urls.join(" ")),
                };
                log(&line);
                view.update(cx, |v, cx| v.push_event(line, cx));
            });
        }
    })
    .detach();
}

fn spawn_graceful_quit(quit_rx: async_channel::Receiver<()>, view: Entity<MainView>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while quit_rx.recv().await.is_ok() {
            let _ = cx.update(|cx| {
                view.update(cx, |v, cx| {
                    v.push_event("quit: graceful shutdown (2 s)...", cx)
                })
            });
            let started = std::time::Instant::now();
            let shutdown = cx.update(|cx| {
                Tokio::spawn(cx, async {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                })
            });
            let _ = shutdown.await;
            log(format!(
                "quit: async shutdown on tokio finished in {} ms",
                started.elapsed().as_millis()
            ));
            native::mark_shutdown_done();
            let _ = cx.update(|cx| cx.quit());
        }
    })
    .detach();
}
