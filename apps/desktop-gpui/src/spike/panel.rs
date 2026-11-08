//! Quick-Recall-shaped panel: `WindowKind::PopUp`, 800×60, 16 px corners, transparent.

use gpui_kit::base::Root;
use gpui_kit::{
    div, point, px, size, transparent_black, App, AppContext, Bounds, Context, IntoElement,
    ParentElement, Render, Styled, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions,
};

use super::fonts::{MONO, UI};
use super::theme::{Mode, Tokens};

pub struct PanelView {
    pub mode: Mode,
}

impl Render for PanelView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = Tokens::for_mode(self.mode);
        div().size_full().p(px(0.)).child(
            div()
                .size_full()
                .rounded(px(16.))
                .bg(t.surface)
                .border_1()
                .border_color(t.border)
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(20.))
                .text_color(t.fg)
                .font_family(UI)
                .child(div().text_size(px(16.)).child("Quick Recall spike"))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(t.muted)
                        .child("cmd-opt-space toggles"),
                ),
        )
    }
}

pub fn open(cx: &mut App, mode: Mode) -> anyhow::Result<WindowHandle<Root>> {
    let panel_size = size(px(800.), px(60.));
    let display = cx.primary_display();
    let origin = display
        .map(|d| {
            let area = d.bounds();
            point(
                area.origin.x + (area.size.width - panel_size.width) / 2.,
                area.origin.y + area.size.height * 0.15,
            )
        })
        .unwrap_or(point(px(200.), px(120.)));
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, panel_size))),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        |window, cx| {
            let sharing = super::native::apply_sharing_type(window);
            super::log(format!(
                "panel created hidden; sharingType set before first show: {:?}",
                sharing.map(|s| s.0)
            ));
            let view = cx.new(|_| PanelView { mode });
            // gpui-component's Root plugin paints the theme background on the root
            // surface; the panel's rounded corners need it transparent.
            cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
        },
    )
}
