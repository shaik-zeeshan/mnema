//! Our tokens pushed into gpui-kit's `ThemeColor`, so kit components (inputs,
//! buttons, popovers, scrollbars) paint with §3.7 colours and our fonts.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::App;

use super::scale::{radius, text, FONT_MONO, FONT_UI};
use super::{Mode, Palette};

pub(super) fn apply(mode: Mode, cx: &mut App) {
    let kit_mode = match mode {
        Mode::Dark => ThemeMode::Dark,
        Mode::Light => ThemeMode::Light,
    };
    // `change` loads the kit's own palette for the mode; ours goes on top in a
    // second update, as `Theme::update`'s docs require.
    Theme::change(kit_mode, None, cx);
    let p = mode.palette();
    Theme::update(cx, |theme| map(theme, &p));
}

fn map(theme: &mut Theme, p: &Palette) {
    theme.font_family = FONT_UI.into();
    theme.mono_font_family = FONT_MONO.into();
    theme.font_size = text::MD;
    theme.mono_font_size = text::BASE;
    theme.radius = radius::SM;
    theme.radius_lg = radius::LG;

    let c = &mut theme.colors;
    c.background = p.bg;
    c.foreground = p.fg;
    c.border = p.border;
    c.window_border = p.border;
    c.input = p.border_strong;
    c.ring = p.accent_glow;
    c.caret = p.text_strong;
    c.selection = p.accent_glow;
    c.muted = p.surface_subtle;
    c.muted_foreground = p.text_muted;
    c.accent = p.wash_strong;
    c.accent_foreground = p.text_strong;
    c.popover = p.pop_bg;
    c.popover_foreground = p.text;
    c.overlay = p.overlay_bg;
    c.link = p.accent;
    c.link_hover = p.accent_strong;
    c.link_active = p.accent_strong;

    c.primary = p.accent;
    c.primary_hover = p.accent_strong;
    c.primary_active = p.accent_strong;
    c.primary_foreground = p.accent_contrast;
    c.secondary = p.surface_raised;
    c.secondary_hover = p.surface_hover;
    c.secondary_active = p.surface_hover;
    c.secondary_foreground = p.text_strong;
    c.button = p.surface_raised;
    c.button_hover = p.surface_hover;
    c.button_active = p.surface_hover;
    c.button_foreground = p.text_strong;
    c.button_primary = p.accent;
    c.button_primary_hover = p.accent_strong;
    c.button_primary_active = p.accent_strong;
    c.button_primary_foreground = p.accent_contrast;

    c.danger = p.danger;
    c.danger_hover = p.danger_strong;
    c.danger_active = p.danger_strong;
    c.danger_foreground = p.danger_text;
    c.warning = p.warn;
    c.warning_hover = p.warn_strong;
    c.warning_active = p.warn_strong;
    c.warning_foreground = p.warn;
    c.info = p.info;
    c.info_hover = p.info_strong;
    c.info_active = p.info_strong;
    c.info_foreground = p.info;
    c.success = p.accent;
    c.success_hover = p.accent_strong;
    c.success_active = p.accent_strong;
    c.success_foreground = p.accent;

    c.list = p.surface;
    c.list_hover = p.wash;
    c.list_active = p.wash_strong;
    c.list_active_border = p.border_strong;
    c.list_even = p.surface;
    c.list_head = p.surface_subtle;
    c.tab_bar = p.wash;
    c.tab_bar_segmented = p.wash;
    c.tab = p.bg;
    c.tab_active = p.surface_raised;
    c.tab_active_foreground = p.text_strong;
    c.tab_foreground = p.text_muted;
    c.switch = p.border_strong;
    c.switch_thumb = p.surface_raised;
    c.slider_bar = p.accent;
    c.slider_thumb = p.surface_raised;
    c.progress_bar = p.accent;
    c.skeleton = p.wash_strong;
    c.scrollbar = p.bg;
    c.scrollbar_thumb = p.border_strong;
    c.scrollbar_thumb_hover = p.border_hover;
    c.title_bar = p.titlebar_bg;
    c.title_bar_border = p.titlebar_border;
    c.status_bar = p.status_bg;
    c.status_bar_border = p.status_border;
    c.sidebar = p.surface_subtle;
    c.sidebar_border = p.border;
    c.sidebar_foreground = p.text;
    c.sidebar_accent = p.wash_strong;
    c.sidebar_accent_foreground = p.text_strong;
    c.sidebar_primary = p.accent;
    c.sidebar_primary_foreground = p.accent_contrast;
    c.chart_1 = p.cat_creating;
    c.chart_2 = p.cat_communication;
    c.chart_3 = p.cat_research;
    c.chart_4 = p.cat_personal;
    c.chart_5 = p.cat_meetings;
    c.chart_grid = p.hairline;
}
