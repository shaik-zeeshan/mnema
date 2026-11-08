//! General › Startup (`Startup.svelte`).

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context};

use crate::settings::row::{row, section};
use crate::settings::{on, Settings};
use crate::theme::Palette;
use crate::ui::switch::Switch;

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let rows = vec![row("Auto-start recording on launch")
        .desc("Begin capturing immediately when the app opens")
        .control(
            Switch::new(
                "st-auto-start",
                "Auto-start recording on launch",
                s.d().auto_start,
            )
            .on_toggle(on(cx, |s, v: bool, cx| s.edit(|d| d.auto_start = v, cx))),
        )
        .build(p)
        .into_any_element()];
    section("Startup", None, Vec::new(), false, rows, p).into_any_element()
}
