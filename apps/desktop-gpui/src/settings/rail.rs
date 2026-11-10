//! The 252 px rail (`SettingsRail.svelte`): a 52 px drag strip over the
//! traffic lights, "Back to app" (Esc), the search field (`/`), the five groups
//! as tinted-tile buttons with the open group's sections nested under a
//! hairline guide, and a REC + lanes + version footer.

use crate::ui::input::FieldText as _;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Div, Hsla, MouseButton, SharedString, Window};

use super::groups::{self, GroupId, SectionId};
use super::Settings;
use crate::shell::model::{self as shell_model, RecMod};
use crate::theme::{self, breakpoint, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::{icon, kbd, mix, Button};
use gpui_kit::component::input::Input;

pub const RAIL_W: f32 = 252.;
pub const RAIL_W_TIGHT: f32 = 200.;
pub const DRAG_H: f32 = 52.;

/// The tile tint per group (`TINT` in the rail).
fn tint(g: GroupId, p: &Palette) -> Hsla {
    match g {
        GroupId::General => p.chart_grey_4,
        GroupId::Capture => p.source_screen,
        GroupId::Intelligence => p.cat_learning,
        GroupId::Data => p.cat_organizing,
        GroupId::About => p.cat_research,
    }
}

fn drag_strip(h: f32) -> Div {
    div()
        .flex_none()
        .h(px(h))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
}

/// `.st-side`: 252 px, 200 px on a window ≤800 px wide.
pub fn rail_w(window: &Window) -> f32 {
    if window.viewport_size().width <= breakpoint::SETTINGS_TIGHT {
        RAIL_W_TIGHT
    } else {
        RAIL_W
    }
}

pub fn render(s: &Settings, window: &mut Window, cx: &mut Context<Settings>) -> AnyElement {
    let p = theme::tokens(cx);
    let query = s.inputs.search.read(cx).value().to_string();
    let filtering = !query.trim().is_empty();
    let visible = groups::filter(&query);
    let light = theme::mode(cx) == theme::Mode::Light;
    let bg = if light {
        mix(p.fg, p.bg, 0.035)
    } else {
        mix(p.surface, p.bg, 0.62)
    };

    let back = Button::new("settings-back", "Back to app")
        .ghost()
        .sm()
        .icon("icons/arrow-left.svg")
        .kbd("Esc")
        .on_click(cx.listener(|_, _, _, cx| cx.emit(super::SettingsEvent::Back)));
    let search = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(size::H_MD)
        .pl(px(10.))
        .pr(px(6.))
        .rounded(radius::MD)
        .border_1()
        .border_color(p.hairline)
        .bg(p.wash)
        .text_color(p.text_subtle)
        .child(icon("icons/search.svg", px(13.)))
        .child(
            div().flex_1().min_w_0().text_color(p.text_strong).child(
                Input::new(&s.inputs.search)
                    .id("settings-search")
                    .aria_label("Search settings")
                    .appearance(false)
                    .px_0()
                    .field_text(text::BASE)
                    .field_h(size::H_MD - px(2.)),
            ),
        )
        .child(kbd("/", &p));

    let nav = div()
        .id("settings-nav")
        .role(gpui_kit::Role::Navigation)
        .aria_label("Settings sections")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .gap(px(2.))
        .px(space::S3)
        .pt(space::S1)
        .pb(space::S4)
        .children(
            visible
                .iter()
                .map(|(gid, sections)| group_block(s, *gid, sections, filtering, &p, cx)),
        )
        .when(visible.is_empty(), |d| {
            d.child(
                div()
                    .p(space::S2)
                    .text_size(text::BASE)
                    .text_color(p.text_muted)
                    .child(format!(
                        "No settings match \u{201c}{}\u{201d}.",
                        query.trim()
                    )),
            )
        });

    div()
        .id("settings-rail")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(rail_w(window)))
        .h_full()
        .bg(bg)
        .border_r_1()
        .border_color(p.hairline)
        .child(drag_strip(DRAG_H))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(space::S3)
                .px(space::S3)
                .pb(space::S3)
                .child(div().flex().ml(px(-6.)).child(back))
                .child(search),
        )
        .child(crate::ui::scroll::area(nav))
        .child(footer(s, &p, window, cx))
        .into_any_element()
}

fn group_block(
    s: &Settings,
    gid: GroupId,
    sections: &[SectionId],
    filtering: bool,
    p: &Palette,
    cx: &mut Context<Settings>,
) -> Div {
    let g = groups::group(gid);
    let open = filtering || gid == s.group;
    let current = !filtering && gid == s.group;
    let t = tint(gid, p);
    let tile = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(6.))
        .map(|d| {
            if current {
                d.bg(t)
                    .text_color(if theme::mode(cx) == theme::Mode::Light {
                        p.surface
                    } else {
                        p.bg
                    })
            } else {
                d.bg(gpui_kit::Hsla { a: 0.13, ..t })
                    .border_1()
                    .border_color(gpui_kit::Hsla { a: 0.22, ..t })
                    .text_color(t)
            }
        })
        .child(icon(g.icon, px(13.)));
    let first = g.sections[0].id;
    let btn = div()
        .id(SharedString::from(format!("settings-cat-{gid:?}")))
        .test_support()
        .role(gpui_kit::Role::Button)
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .h(px(32.))
        .pl(px(5.))
        .pr(space::S2)
        .rounded(radius::MD)
        .text_size(text::MD)
        .font_weight(theme::W_MEDIUM)
        .text_color(if current { p.text_strong } else { p.text })
        .when(current, |d| d.bg(p.wash_strong))
        .cursor_pointer()
        .hover(|st| st.bg(p.wash).text_color(p.text_strong))
        .child(tile)
        .child(g.label)
        .child(div().flex_1())
        .map(|d| {
            if filtering {
                d.child(
                    div()
                        .px(px(5.))
                        .h(px(16.))
                        .min_w(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(radius::PILL)
                        .font_family(FONT_MONO)
                        .text_size(text::XS)
                        .text_color(p.accent)
                        .bg(p.accent_bg)
                        .child(sections.len().to_string()),
                )
            } else {
                d.child(
                    div()
                        .text_color(if open { p.text_subtle } else { p.text_faint })
                        .child(icon(
                            if open {
                                "icons/chevron-down.svg"
                            } else {
                                "icons/chevron-right.svg"
                            },
                            px(12.),
                        )),
                )
            }
        })
        .on_click(super::click(cx, move |s, cx| s.navigate(first, cx)));
    let sub = open.then(|| {
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(1.))
            .mt(px(2.))
            .mb(px(6.))
            .ml(px(16.))
            .pl(px(13.))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(4.))
                    .bottom(px(4.))
                    .w(px(1.))
                    .bg(p.hairline),
            )
            .children(sections.iter().map(|id| {
                let id = *id;
                let active = id == s.section && !filtering;
                div()
                    .id(SharedString::from(format!("settings-tab-{id:?}")))
                    .test_support()
                    .role(gpui_kit::Role::Link)
                    .w_full()
                    .px(space::S2)
                    .py(px(5.))
                    .rounded(radius::SM)
                    .text_size(text::BASE)
                    .line_height(gpui_kit::relative(1.25))
                    .text_color(if active { p.text_strong } else { p.text_subtle })
                    .when(active, |d| d.font_weight(theme::W_MEDIUM).bg(p.wash_strong))
                    .cursor_pointer()
                    .hover(|st| st.text_color(p.text_strong).bg(p.wash))
                    .child(groups::section_label(id))
                    .on_click(super::click(cx, move |s, cx| s.navigate(id, cx)))
            }))
    });
    div().flex().flex_col().child(btn).children(sub)
}

/// REC pill + the three lanes (display only) + "Mnema <version>".
fn footer(s: &Settings, p: &Palette, _: &mut Window, cx: &mut Context<Settings>) -> Div {
    let view = shell_model::rec_view(&s.status);
    let (fg, border, dot) = match view.kind {
        RecMod::Running => (
            p.status_running_fg,
            p.status_running_border,
            p.status_running_dot,
        ),
        RecMod::Paused => (
            p.status_paused_fg,
            p.status_paused_border,
            p.status_paused_dot,
        ),
        RecMod::Suspended => (p.warn, p.warn_border, p.warn),
        RecMod::Stopped => (p.danger_text, p.danger_border, p.danger),
        RecMod::Starting => (p.text_muted, p.border_strong, p.status_dot),
        RecMod::Off => (p.status_fg, gpui_kit::transparent_black(), p.status_dot),
    };
    let label = match (view.clock, s.status.session_start_ms) {
        (Some(shell_model::Clock::Elapsed), Some(start)) => format!(
            "{} {}",
            view.label,
            shell_model::format_clock(crate::ui::clock::now_ms(cx) - start, true)
        ),
        _ => view.label.clone(),
    };
    let pill = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(18.))
        .pl(px(6.))
        .pr(px(7.))
        .rounded(radius::PILL)
        .border_1()
        .border_color(border)
        .font_family(if view.kind == RecMod::Off {
            theme::FONT_UI
        } else {
            FONT_MONO
        })
        .text_size(if view.kind == RecMod::Off {
            text::MD
        } else {
            text::XS
        })
        .font_weight(theme::W_MEDIUM)
        .text_color(fg)
        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
        .child(label);
    let lanes = div().flex().gap(px(4.)).ml_auto().children(
        [p.source_screen, p.source_mic, p.source_sysaudio]
            .into_iter()
            .zip(s.status.selected)
            .map(|(c, on)| {
                div()
                    .size(px(6.))
                    .rounded(px(1.))
                    .bg(c)
                    .opacity(if on { 1. } else { 0.35 })
            }),
    );
    let version = s
        .data
        .update
        .as_ref()
        .and_then(|u| u["app"]["version"].as_str())
        .map(str::to_string);
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(space::S4)
        .pt(space::S3)
        .pb(space::S4)
        .border_t_1()
        .border_color(p.hairline)
        .child(
            div()
                .flex()
                .items_center()
                .gap(space::S2)
                .child(pill)
                .child(lanes),
        )
        .child(
            div()
                .flex()
                .gap(px(4.))
                .text_size(text::SM)
                .text_color(p.text_subtle)
                .child("Mnema")
                .children(version.map(|v| div().font_family(FONT_MONO).child(v))),
        )
}
