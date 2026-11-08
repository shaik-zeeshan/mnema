//! General › Appearance (`Appearance.svelte`): theme, time format, follow live.

use capture_types::{AppearanceSetting, TimeFormatSetting};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context};

use super::ix;
use crate::settings::row::{row, section};
use crate::settings::{on, Settings};
use crate::theme::Palette;
use crate::ui::segmented::Segmented;
use crate::ui::switch::Switch;

const THEMES: [AppearanceSetting; 3] = [
    AppearanceSetting::System,
    AppearanceSetting::Light,
    AppearanceSetting::Dark,
];
const TIME: [TimeFormatSetting; 3] = [
    TimeFormatSetting::System,
    TimeFormatSetting::H12,
    TimeFormatSetting::H24,
];

pub fn render(s: &Settings, p: &Palette, cx: &mut Context<Settings>) -> AnyElement {
    let d = s.d();
    let rows = vec![
        row("Theme")
            .desc("System follows your OS setting; pick Light or Dark to override it.")
            .control(
                Segmented::new(
                    "st-theme",
                    "Theme",
                    ["System", "Light", "Dark"],
                    ix(&THEMES, &d.appearance),
                )
                .md()
                .on_change(on(cx, |s, i: usize, cx| {
                    s.edit(|d| d.appearance = THEMES[i].clone(), cx)
                })),
            )
            .build(p)
            .into_any_element(),
        row("Time format")
            .desc("System follows your Mac's 24-hour time setting.")
            .control(
                Segmented::new(
                    "st-time",
                    "Time format",
                    ["System", "12-hour", "24-hour"],
                    ix(&TIME, &d.time_format),
                )
                .md()
                .on_change(on(cx, |s, i: usize, cx| {
                    s.edit(|d| d.time_format = TIME[i].clone(), cx)
                })),
            )
            .build(p)
            .into_any_element(),
        row("Follow live recording")
            .desc("Keep the timeline pinned to the latest captured data while recording")
            .control(
                Switch::new(
                    "st-follow-live",
                    "Follow live recording",
                    d.follow_timeline_live,
                )
                .on_toggle(on(cx, |s, v: bool, cx| {
                    s.edit(|d| d.follow_timeline_live = v, cx)
                })),
            )
            .build(p)
            .into_any_element(),
    ];
    section(
        "Appearance",
        Some("Theme switches immediately when saved."),
        Vec::new(),
        false,
        rows,
        p,
    )
    .into_any_element()
}
