//! The §5 fast-scrub frame-time check: the main window over the populated parity
//! fixture (its frames tiled out to 2000 so the rail virtualises), scrubbed by
//! synthetic wheel events every 8 ms, then quit. Build it in release with
//! `--features fixtures,feel-metrics`; frame times land in `<log dir>/rust.log`
//! for `scripts/gpui-feel-report.sh`. Fixture data only: safe to run anywhere.
//!
//! `cargo run --release -p mnema-gpui --features fixtures,feel-metrics --example scrub-bench -- <log dir>`

use std::sync::Arc;
use std::time::Duration;

use capture_types::AppearanceSetting;
use gpui_kit::{
    point, px, size, AppContext, Bounds, InputEvent, ScrollDelta, ScrollEvent, TitlebarOptions,
    WindowBounds, WindowOptions,
};
use mnema_gpui::assets::MnemaAssets;
use mnema_gpui::shell::{Shell, ShellModel};
use mnema_gpui::timeline::fixture::FixtureSource;
use mnema_gpui::timeline::model::Frame;
use mnema_gpui::{feel, keymap, logging, theme};

const TILED: usize = 2000;
const EVENTS: usize = 1500;
const STEP: Duration = Duration::from_millis(8);
/// 24 px per 8 ms event = 3 px/ms: past the fast-scrub step.
const DELTA_PX: f32 = 24.;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scrub-bench-log".into());
    logging::init(std::path::Path::new(&dir));
    let src = FixtureSource::load("default").expect("timeline fixture");
    let base = src.frames();
    // The fixture has one image; rotate the nine web shots so the stage really
    // swaps images while scrubbing.
    let shots_dir = base[0].file_path.parent().expect("shots dir").to_path_buf();
    let mut shots: Vec<_> = std::fs::read_dir(&shots_dir)
        .expect("shots")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "webp"))
        .collect();
    shots.sort();
    // Every real frame is its own file: one symlink per frame makes each one a
    // fresh decode, GPU upload and (past the 192-entry LRU) eviction, as in the app.
    let links = std::path::Path::new(&dir).join("frames");
    let _ = std::fs::remove_dir_all(&links);
    std::fs::create_dir_all(&links).expect("frames dir");
    let frames: Vec<Frame> = (0..TILED)
        .map(|i| {
            let f = &base[i % base.len()];
            let link = links.join(format!("{i}.webp"));
            std::os::unix::fs::symlink(&shots[i % shots.len()], &link).expect("frame link");
            Frame {
                id: 1_000_000 - i as i64,
                captured_at_ms: base[0].captured_at_ms - i as i64 * 5000,
                file_path: Arc::from(link.as_path()),
                ..f.clone()
            }
        })
        .collect();
    let src = Arc::new(src.with_frames(frames).with_scrub_ready());
    gpui_kit::application()
        .with_assets(MnemaAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, cx.window_appearance(), cx);
            keymap::bind(&keymap::Bindings::default(), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Mnema scrub bench".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
                // Frames only draw for a visible window: bring it to the front.
                window.activate_window();
                cx.new(|cx| Shell::with_timeline_source(ShellModel::default(), src, window, cx))
            })
            .expect("window");
            let timeline = shell.read(cx).timeline.clone();
            cx.activate(true);
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                log::info!("scrub bench: start");
                feel::section(true);
                for i in 0..EVENTS {
                    // Older for the first half, back to the head for the second.
                    let dy = if i < EVENTS / 2 { -DELTA_PX } else { DELTA_PX };
                    let event = ScrollEvent {
                        position: point(px(640.), px(400.)),
                        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                        ..Default::default()
                    };
                    if i == EVENTS / 2 {
                        let at = cx.update(|cx| timeline.read(cx).paging.active);
                        log::info!("scrub bench: turned at frame {at:?}");
                    }
                    let sent = cx.update_window(window, |_, window, cx| {
                        window.dispatch_event(event.to_platform_input(), cx);
                    });
                    if sent.is_err() {
                        break;
                    }
                    cx.background_executor().timer(STEP).await;
                }
                feel::section(false);
                let deepest = cx.update(|cx| timeline.read(cx).paging.frames.len());
                log::info!("scrub bench: done; frames loaded {deepest:?}");
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
