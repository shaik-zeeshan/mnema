//! `cargo run -p mnema-gpui --bin gallery [group]`: every shared control in one
//! window, following the OS appearance (⌘T flips light / dark). No AppCore, no
//! data: safe to run anywhere.

use capture_types::AppearanceSetting;
use gpui_kit::{
    actions, px, size, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions,
};
use mnema_gpui::assets::MnemaAssets;
use mnema_gpui::gallery::{Gallery, Group};
use mnema_gpui::theme::{self, Mode};

actions!(gallery, [Quit, FlipTheme]);

fn main() {
    let only = std::env::args()
        .nth(1)
        .and_then(|n| Group::ALL.into_iter().find(|g| g.name() == n));
    gpui_kit::application()
        .with_assets(MnemaAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(AppearanceSetting::System, cx.window_appearance(), cx);
            theme::reduce_motion::observe(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &FlipTheme, cx| {
                let next = match theme::mode(cx) {
                    Mode::Dark => AppearanceSetting::Light,
                    Mode::Light => AppearanceSetting::Dark,
                };
                theme::set_setting(next, cx);
                cx.refresh_windows();
            });
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-t", FlipTheme, None),
            ]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(980.), px(860.)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Mnema controls".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                cx.new(|cx| Gallery::new(only.map_or(Group::ALL.to_vec(), |g| vec![g]), window, cx))
            });
            if let Err(e) = opened {
                eprintln!("gallery: window failed to open: {e:#}");
                cx.quit();
            }
        });
}
