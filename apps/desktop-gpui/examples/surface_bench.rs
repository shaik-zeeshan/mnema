//! Frame time on every non-Timeline screen: the main window (1280x800, dark,
//! in front) on one screen over its parity fixture, settled ~2 s, then two
//! measured phases, each in its own `feel::section` after a
//! `surface bench: phase <name>` line, then quit:
//! - `scroll`: 300 wheel events of -24 px every 8 ms over the content, then 300 back;
//! - `hover`: 400 pointer moves every 8 ms down the window (below the titlebar)
//!   and back, weaving across most of its width so the pointer crosses rails,
//!   rows, chips and chart marks. GPUI only redraws on a hover *change*, so this
//!   phase's frame count is how many hover changes the sweep caused.
//!
//! Fixture data only: safe to run anywhere. `scripts/gpui-surface-bench.sh`
//! builds it, runs every screen and prints the table.
//!
//! `cargo run -p mnema-gpui --features fixtures,feel-metrics --example surface-bench -- <log dir> <surface> <state>`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    point, px, size, transparent_black, AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Entity,
    InputEvent, Modifiers, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollEvent,
    TitlebarOptions, WindowBounds, WindowOptions,
};
use mnema_gpui::insights::fixture::{FixtureBackend as InsightsFixture, Open, STATES as IN_STATES};
use mnema_gpui::insights::Insights;
use mnema_gpui::quick_recall::data::Backend as _;
use mnema_gpui::quick_recall::fixture::FixtureBackend as QrFixture;
use mnema_gpui::quick_recall::{QuickRecall, ASK_PAUSE, COMMIT, SEARCH_DEBOUNCE};
use mnema_gpui::settings::fixture::{settings_data, FixtureBackend as SettingsFixture};
use mnema_gpui::settings::Settings;
use mnema_gpui::shell::{fixture, Shell, ShellModel, Surface};
use mnema_gpui::{assets::MnemaAssets, feel, keymap, logging, theme, window};

const USAGE: &str = "surface-bench <log dir> <insights|chat|settings|quick-recall> <state>";
/// Whole-page Insights states (the modal, receipt and empty states are left out).
const INSIGHTS: [&str; 5] = ["overview", "journal", "subjects", "context", "subject"];
const CHAT: [&str; 2] = ["list", "done"];
const QUICK_RECALL: [&str; 2] = ["results", "ask-done"];
/// The feel target: the app log keeps it at every level (an example's own
/// target isn't an app target, so its lines would be dropped).
const LOG: &str = "mnema_gpui::feel";
const MAIN: (f32, f32) = (1280., 800.);
/// Quick Recall's golden viewport (`tests/visual/quick_recall.rs`).
const QR: (f32, f32) = (800., 700.);
const STEP: Duration = Duration::from_millis(8);
const SCROLLS: usize = 300;
const DELTA_PX: f32 = 24.;
const MOVES: usize = 400;

/// What the post-open steps need to reach.
enum Target {
    Insights(Entity<Insights>, Open),
    Settings(Entity<Settings>, &'static str),
    Chat,
    /// The bar, and the answer to adopt once the typed query has committed.
    QuickRecall(Entity<QuickRecall>, Option<capture_types::TurnSnapshot>),
}

struct Opened {
    window: AnyWindowHandle,
    /// The scrolling content, in window points (the wheel events land at its centre).
    area: Bounds<Pixels>,
    target: Target,
}

fn main() {
    feel::mark_start();
    if std::env::args().nth(1).as_deref() == Some("--list") {
        list();
        return;
    }
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(surface), Some(state)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: {USAGE}");
        std::process::exit(2);
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("log dir");
    logging::init(&dir);
    gpui_kit::application()
        .with_assets(MnemaAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, cx.window_appearance(), cx);
            keymap::bind(&keymap::Bindings::default(), cx);
            let opened = match surface.as_str() {
                "insights" => insights(&state, cx),
                "chat" => chat(&state, cx),
                "settings" => settings(&state, cx),
                "quick-recall" => quick_recall(&state, cx),
                other => Err(anyhow::anyhow!("no surface {other}; {USAGE}")),
            };
            let opened = opened.unwrap_or_else(|e| {
                eprintln!("surface-bench: {e:#}");
                std::process::exit(2);
            });
            // A background, occluded window gets no display-link frames: come forward.
            cx.activate(true);
            cx.spawn(async move |cx| run(opened, cx).await).detach();
        });
}

/// Every supported `<surface> <state>`, one per line (the script reads it).
fn list() {
    let settings = mnema_gpui::settings::fixture::STATES.map(|(s, _)| s);
    for (surface, states) in [
        ("insights", &INSIGHTS[..]),
        ("chat", &CHAT[..]),
        ("settings", &settings[..]),
        ("quick-recall", &QUICK_RECALL[..]),
    ] {
        for state in states {
            println!("{surface} {state}");
        }
    }
}

async fn run(o: Opened, cx: &mut AsyncApp) {
    let ex = cx.background_executor().clone();
    let wait = |ms| ex.timer(Duration::from_millis(ms));
    wait(300).await;
    // Steps that need the first layout or a landed load.
    match &o.target {
        Target::Settings(s, tab) => {
            let _ = s.update(cx, |s, cx| s.open_tab(tab, cx));
        }
        Target::Insights(i, Open::Subject) => {
            let first = i.read_with(cx, |i, _| {
                i.subjects
                    .as_ref()
                    .and_then(|s| s.conclusions.first().map(|c| c.subject.clone()))
            });
            if let Some(s) = first {
                let _ = i.update(cx, |i, cx| i.open_subject(s, cx));
                wait(500).await;
                let _ = i.update(cx, |i, cx| {
                    i.open_conclusion = i
                        .detail
                        .as_ref()
                        .and_then(|d| d.view.as_ref())
                        .and_then(|v| v.conclusions.first().map(|c| c.id));
                    cx.notify();
                });
            } else {
                log::warn!(target: LOG, "surface bench: no subject to open");
            }
        }
        Target::QuickRecall(qr, Some(snap)) => {
            // As the goldens: the query commits and pauses, then the answer lands.
            cx.background_executor()
                .timer(COMMIT.max(SEARCH_DEBOUNCE) + ASK_PAUSE)
                .await;
            let snap = snap.clone();
            let _ = qr.update(cx, |qr, cx| qr.adopt_snapshot(snap, cx));
        }
        _ => {}
    }
    wait(2000).await;
    let mid = o.area.center();
    let view = cx
        .update_window(o.window, |_, window, _| window.viewport_size())
        .unwrap_or(size(px(MAIN.0), px(MAIN.1)));
    let sweep = Bounds::new(point(px(16.), px(44.)), view - size(px(32.), px(60.)));

    log::info!(target: LOG, "surface bench: phase scroll");
    section(o.window, true, cx);
    for i in 0..2 * SCROLLS {
        let dy = if i < SCROLLS { -DELTA_PX } else { DELTA_PX };
        let event = ScrollEvent {
            position: mid,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        };
        if !send(o.window, event.to_platform_input(), cx) {
            break;
        }
        cx.background_executor().timer(STEP).await;
    }
    wait(50).await;
    section(o.window, false, cx);
    wait(500).await;

    log::info!(target: LOG, "surface bench: phase hover");
    section(o.window, true, cx);
    for i in 0..MOVES {
        let event = MouseMoveEvent {
            position: hover_at(sweep, i),
            modifiers: Modifiers::default(),
            pressed_button: None,
        };
        if !send(o.window, event.to_platform_input(), cx) {
            break;
        }
        cx.background_executor().timer(STEP).await;
    }
    wait(50).await;
    section(o.window, false, cx);
    wait(300).await;
    log::info!(target: LOG, "surface bench: done");
    cx.update(|cx| cx.quit());
}

/// A section edge, after draining the frames presented so far (so they count
/// on the right side of it).
fn section(window: AnyWindowHandle, start: bool, cx: &mut AsyncApp) {
    let _ = cx.update_window(window, |_, window, _| feel::frame_rendered(window));
    feel::section(start);
}

fn send(window: AnyWindowHandle, input: gpui_kit::PlatformInput, cx: &mut AsyncApp) -> bool {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(input, cx);
        // Drain the frames presented so far: the shell's render drains them
        // too, but a frame that only redraws a child view never runs it, and
        // its sample would land in the log after the section closed.
        feel::frame_rendered(window);
    })
    .is_ok()
}

/// Down `a` over the first half, back up over the second, weaving across
/// 90% of its width (one side to the other every ~9 moves).
fn hover_at(a: Bounds<Pixels>, i: usize) -> Point<Pixels> {
    let half = MOVES / 2;
    let t = if i < half { i } else { MOVES - 1 - i } as f32 / (half - 1) as f32;
    let (x0, y0) = (f32::from(a.origin.x), f32::from(a.origin.y));
    let (w, h) = (f32::from(a.size.width), f32::from(a.size.height));
    let x = x0 + w * (0.5 + 0.45 * (i as f32 * 0.35).sin());
    point(px(x), px(y0 + h * t))
}

fn area(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

/// The real main window's options at the bench size, shown.
fn main_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(MAIN.0), px(MAIN.1)),
            cx,
        ))),
        show: true,
        ..window::main_options(cx)
    }
}

/// The main window on `surface`; `fill` loads the fixture into the shell.
fn open_shell(
    model: ShellModel,
    surface: Surface,
    cx: &mut App,
    fill: impl FnOnce(&Entity<Shell>, &mut App),
) -> anyhow::Result<(AnyWindowHandle, Entity<Shell>)> {
    let (handle, shell) = gpui_kit::open_window(main_options(cx), cx, |window, cx| {
        window.activate_window();
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(model, window, cx);
            shell.go(surface, cx);
            shell
        });
        fill(&shell, cx);
        shell
    })?;
    Ok((handle.into(), shell))
}

fn shell_model(surface: &str, set: &str) -> anyhow::Result<ShellModel> {
    fixture::shell_model(&fixture::load(surface, set)?)
}

/// As `tests/visual/insights.rs::render`, through the first `Open` step.
fn insights(state: &str, cx: &mut App) -> anyhow::Result<Opened> {
    anyhow::ensure!(INSIGHTS.contains(&state), "insights states: {INSIGHTS:?}");
    let st = IN_STATES.iter().find(|s| s.name == state).expect("state");
    let backend = Arc::new(InsightsFixture::load(st.set)?);
    let (window, shell) = open_shell(
        shell_model("insights", st.set)?,
        Surface::Insights,
        cx,
        |shell, cx| {
            shell.read(cx).insights.clone().update(cx, |i, cx| {
                i.backend = backend;
                i.tab = st.tab;
                i.reload(cx);
            })
        },
    )?;
    let insights = shell.read(cx).insights.clone();
    Ok(Opened {
        window,
        // Under the titlebar and the tab row, above the status bar.
        area: area(40., 110., 1200., 650.),
        target: Target::Insights(insights, st.open),
    })
}

/// As `tests/visual/chat.rs::render`.
fn chat(state: &str, cx: &mut App) -> anyhow::Result<Opened> {
    anyhow::ensure!(CHAT.contains(&state), "chat states: {CHAT:?}");
    let fx = mnema_gpui::chat::fixture::chat_fixture(state)?;
    let (window, _) = open_shell(
        shell_model("shell", "populated")?,
        Surface::Chat,
        cx,
        |shell, cx| {
            shell.read(cx).chat.clone().update(cx, |chat, cx| {
                chat.set_data(fx.data.clone(), cx);
                if let Some(convo) = &fx.open {
                    chat.show(convo, fx.snapshot.as_ref(), cx);
                }
            })
        },
    )?;
    Ok(Opened {
        window,
        // The conversation column, right of the chats rail.
        area: area(300., 60., 960., 700.),
        target: Target::Chat,
    })
}

/// As `tests/visual/settings.rs::render_open` (no modal, no prompt).
fn settings(state: &str, cx: &mut App) -> anyhow::Result<Opened> {
    use mnema_gpui::settings::fixture::STATES;
    let Some(&(_, tab)) = STATES.iter().find(|(s, _)| *s == state) else {
        let names: Vec<_> = STATES.iter().map(|(s, _)| *s).collect();
        anyhow::bail!("settings states: {names:?}");
    };
    let data = settings_data()?;
    let (window, shell) = open_shell(
        shell_model("shell", "populated")?,
        Surface::Settings,
        cx,
        |shell, cx| {
            shell.read(cx).settings.clone().update(cx, |s, cx| {
                s.set_backend(Arc::new(SettingsFixture::default()));
                s.set_data(data, cx)
            })
        },
    )?;
    let settings = shell.read(cx).settings.clone();
    Ok(Opened {
        window,
        // The section column, right of the settings rail.
        area: area(290., 40., 820., 720.),
        target: Target::Settings(settings, tab),
    })
}

/// As `tests/visual/quick_recall.rs::render` (a plain window at the golden
/// size, not the floating panel; native fit off as in the goldens).
fn quick_recall(state: &str, cx: &mut App) -> anyhow::Result<Opened> {
    anyhow::ensure!(
        QUICK_RECALL.contains(&state),
        "quick-recall states: {QUICK_RECALL:?}"
    );
    let backend = Arc::new(QrFixture::load(state)?);
    let query = backend.summon_query();
    let snap = (state == "ask-done")
        .then(|| backend.ask_snapshot(""))
        .flatten();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(QR.0), px(QR.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Mnema surface bench".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let b: Arc<dyn mnema_gpui::quick_recall::data::Backend> = backend;
    let mut view: Option<Entity<QuickRecall>> = None;
    // `cx.open_window` + our own transparent Root, as `quick_recall/panel.rs` opens it.
    let handle = cx.open_window(options, |window, cx| {
        window.activate_window();
        let qr = cx.new(|cx| {
            let mut qr = QuickRecall::new(b, window, cx);
            qr.native_fit = false;
            qr
        });
        view = Some(qr.clone());
        cx.new(|cx| Root::new(qr, window, cx).bg(transparent_black()))
    })?;
    let qr = view.expect("view");
    if let Some(q) = &query {
        qr.update(cx, |qr, cx| qr.take_query(q, cx));
    }
    Ok(Opened {
        window: handle.into(),
        // The results / answer under the field.
        area: area(20., 70., 760., 600.),
        target: Target::QuickRecall(qr, snap),
    })
}
