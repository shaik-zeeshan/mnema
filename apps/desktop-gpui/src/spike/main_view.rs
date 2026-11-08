//! Main-style window: transparent titlebar, 78 px traffic-light inset, app-owned drag.

use gpui_kit::assets::IconName;
use gpui_kit::{
    div, point, px, size, App, Bounds, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, Styled, TitlebarOptions, Window,
    WindowBounds, WindowOptions,
};

use super::fonts::{MONO, UI, WEIGHTS};
use super::theme::{Mode, Tokens};

pub const TITLEBAR_H: f32 = 38.;
pub const TRAFFIC_LIGHT_INSET: f32 = 78.;

pub struct MainView {
    pub mode: Mode,
    pub db_status: SharedString,
    pub events: Vec<SharedString>,
    first_render_logged: bool,
}

impl MainView {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            db_status: "sqlx: pending".into(),
            events: Vec::new(),
            first_render_logged: false,
        }
    }

    pub fn push_event(&mut self, event: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.events.push(event.into());
        if self.events.len() > 6 {
            self.events.remove(0);
        }
        cx.notify();
    }
}

fn kicker(text: &'static str, t: Tokens) -> impl IntoElement {
    div()
        .font_family(MONO)
        .text_size(px(10.))
        .text_color(t.accent)
        .child(text)
}

fn weight_label(w: FontWeight) -> &'static str {
    match w.0 as u32 {
        400 => "400",
        500 => "500",
        _ => "600",
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if !self.first_render_logged {
            self.first_render_logged = true;
            super::log("main window: first render");
            window.on_next_frame(|_, _| super::log("main window: first frame presented"));
        }
        let t = Tokens::for_mode(self.mode);
        let titlebar = div()
            .id("titlebar")
            .h(px(TITLEBAR_H))
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(TRAFFIC_LIGHT_INSET))
            .border_b_1()
            .border_color(t.border)
            .text_color(t.muted)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(
                div()
                    .size(px(14.))
                    .text_color(t.muted)
                    .child(IconName::Search),
            )
            .child(div().text_size(px(13.)).child("Mnema GPUI spike"));

        let font_rows = [UI, MONO].into_iter().flat_map(|family| {
            WEIGHTS.into_iter().map(move |weight| {
                div()
                    .font_family(family)
                    .font_weight(weight)
                    .text_size(px(18.))
                    .child(format!(
                        "{family} {} The quick brown fox 0123456789",
                        weight_label(weight)
                    ))
            })
        });

        div()
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(UI)
            .flex()
            .flex_col()
            .child(titlebar)
            .child(
                div()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(kicker("// FONTS", t))
                    .children(font_rows)
                    .child(div().h(px(8.)))
                    .child(kicker("// TOKIO + SQLX", t))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .child(self.db_status.clone()),
                    )
                    .child(div().h(px(8.)))
                    .child(kicker("// EVENTS", t))
                    .children(self.events.iter().map(|e| {
                        div()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .child(e.clone())
                    }))
                    .child(div().h(px(8.)).w(px(120.)).bg(t.accent).rounded(px(4.))),
            )
    }
}

pub fn window_options(cx: &App) -> WindowOptions {
    let win = size(px(800.), px(600.));
    let origin = cx
        .primary_display()
        .map(|d| {
            let a = d.bounds();
            point(
                a.origin.x + (a.size.width - win.width) / 2.,
                a.origin.y + (a.size.height - win.height) / 2.,
            )
        })
        .unwrap_or(point(px(120.), px(120.)));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, win))),
        titlebar: Some(TitlebarOptions {
            title: Some("Mnema GPUI spike".into()),
            appears_transparent: true,
            // Traffic lights vertically centred in the 38 px bar.
            traffic_light_position: Some(point(px(14.), px(12.))),
        }),
        app_owns_titlebar_drag: true,
        window_min_size: Some(size(px(640.), px(480.))),
        focus: true,
        show: false,
        ..Default::default()
    }
}
