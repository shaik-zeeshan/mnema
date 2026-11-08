//! S0 golden: the spike's main window rendered offscreen through real Metal.
//! `harness = false`: AppKit needs the process main thread, which libtest doesn't give.

#[path = "visual/compare.rs"]
mod compare;

use std::sync::Arc;

use gpui_kit::base::Root;
use gpui_kit::{px, size, AppContext, VisualTestAppContext};
use mnema_gpui::spike::fonts;
use mnema_gpui::spike::main_view::MainView;
use mnema_gpui::spike::theme::Mode;

fn run() -> anyhow::Result<()> {
    compare::self_check();
    let platform = gpui_kit::platform::current_platform(false);
    let mut cx =
        VisualTestAppContext::with_asset_source(platform, Arc::new(gpui_kit::assets::Assets));
    cx.update(|cx| -> anyhow::Result<()> {
        gpui_kit::init(cx);
        fonts::register(cx, false)?;
        for family in [fonts::UI, fonts::MONO] {
            anyhow::ensure!(
                fonts::weights_resolve(cx, family),
                "{family}: 400/500/600 did not resolve to three distinct faces"
            );
        }
        Ok(())
    })?;
    println!("  fonts: 400/500/600 resolve to distinct faces for both families");

    let mut failures = Vec::new();
    for (mode, name) in [
        (Mode::Dark, "spike-window-dark"),
        (Mode::Light, "spike-window-light"),
    ] {
        let window = cx.open_offscreen_window(size(px(800.), px(600.)), |window, cx| {
            let view = cx.new(|_| MainView::new(mode));
            cx.new(|cx| Root::new(view, window, cx))
        })?;
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, _| window.refresh())?;
        cx.run_until_parked();
        let shot = cx.capture_screenshot(window.into())?;
        if let Err(e) = compare::check_golden(name, &shot) {
            failures.push(e.to_string());
        }
    }
    anyhow::ensure!(failures.is_empty(), failures.join("\n"));
    Ok(())
}

fn main() {
    println!("visual_spike:");
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => println!("visual_spike: ok"),
        Ok(Err(e)) => {
            eprintln!("visual_spike FAILED: {e:#}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("visual_spike panicked");
            std::process::exit(1);
        }
    }
}
