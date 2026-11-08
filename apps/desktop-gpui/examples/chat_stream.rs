//! Streaming frame time (BUILD-PROMPT §5): opens the main window on Chat's
//! `mid-stream` parity fixture and feeds ~200 `ask_ai_update`s over ~5 s
//! through the real `Chat::on_update` path, then quits. Frame times land in
//! `<log dir>/rust.log` for `scripts/gpui-feel-report.sh`.
//!
//! cargo run -p mnema-gpui --release --features feel-metrics --example chat_stream -- <log dir>

use std::path::PathBuf;
use std::time::Duration;

use capture_types::{AppearanceSetting, TurnUpdate};
use mnema_gpui::chat::fixture::chat_fixture;
use mnema_gpui::chat::AskAiUpdateEvent;
use mnema_gpui::shell::{ShellModel, Surface};
use mnema_gpui::{assets::MnemaAssets, feel, keymap, logging, theme, window};

const UPDATES: u64 = 200;
const EVERY: Duration = Duration::from_millis(25);
const WORDS: &[&str] = &[
    "the",
    "**review**",
    "ran",
    "long,",
    "and",
    "Ana",
    "asked",
    "about",
    "`retention`",
    "twice.",
    "Then",
    "a",
    "short",
    "sync",
    "with",
    "the",
    "team\n\n-",
    "notes",
    "on",
    "**pricing**",
];

fn main() {
    feel::mark_start();
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("mnema-chat-stream"));
    std::fs::create_dir_all(&dir).expect("log dir");
    logging::init(&dir);
    gpui_kit::application()
        .with_assets(MnemaAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::Dark, cx.window_appearance(), cx);
            keymap::bind(&keymap::Bindings::default(), cx);
            let fx = chat_fixture("mid-stream").expect("mid-stream fixture");
            let shell = window::open_main(ShellModel::default(), cx).expect("main window");
            // A background, occluded window gets no display-link frames: come forward.
            cx.activate(true);
            let chat = shell.read(cx).chat.clone();
            shell.update(cx, |s, cx| s.go(Surface::Chat, cx));
            chat.update(cx, |c, cx| {
                c.set_data(fx.data.clone(), cx);
                c.show(
                    fx.open.as_ref().expect("open chat"),
                    fx.snapshot.as_ref(),
                    cx,
                );
            });
            let start = fx.snapshot.as_ref().map_or(0, |s| s.version);
            let id = fx.open.as_ref().unwrap().conversation_id.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                for i in 1..=UPDATES {
                    cx.background_executor().timer(EVERY).await;
                    let update = if i == UPDATES {
                        TurnUpdate::Done
                    } else {
                        TurnUpdate::AppendProse {
                            text: format!(" {}", WORDS[i as usize % WORDS.len()]),
                        }
                    };
                    let ev = AskAiUpdateEvent {
                        conversation_id: id.clone(),
                        version: start + i,
                        turn_index: 0,
                        update,
                    };
                    let _ = chat.update(cx, |c, cx| c.on_update(ev, cx));
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
