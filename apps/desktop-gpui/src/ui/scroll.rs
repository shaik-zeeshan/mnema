//! The one scroll bar (`+layout.svelte`'s `::-webkit-scrollbar` rules, as a
//! macOS overlay): gpui-base's `Scrollbar` in hover mode, so it is hidden at
//! rest, fades in while the area scrolls or the pointer is on its 12 px edge,
//! and its thumb drags. It floats over the content (no gutter, nothing shifts)
//! and never shows on an area that doesn't overflow.
//!
//! Use: give the scroll area a `ScrollHandle` (`.track_scroll(&h)`, or
//! [`handle`] when the owner keeps none) and make [`bar`] its LAST child.

use std::panic::Location;

use gpui_kit::base::{Scrollbar, ScrollbarMode};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, App, Bounds, DispatchPhase, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, LayoutId, MouseMoveEvent, Pixels, Position, ScrollEvent, ScrollHandle,
    Style, Window,
};

use crate::theme::{self, radius, size};

/// The overlay bar for the vertical scroll area tracking `handle`.
#[track_caller]
pub fn bar(handle: &ScrollHandle) -> Bar {
    Bar {
        id: ElementId::CodeLocation(*Location::caller()),
        handle: handle.clone(),
    }
}

/// A scroll handle kept across frames under `id`, for an area whose owner
/// holds none.
pub fn handle(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> ScrollHandle {
    window
        .use_keyed_state(id, cx, |_, _| ScrollHandle::new())
        .read(cx)
        .clone()
}

/// `el` (an `overflow_y_scroll` area whose owner keeps no `ScrollHandle`) with
/// a handle of its own, kept per call site, and [`bar`] as its last child.
#[track_caller]
pub fn area<E: super::Control + 'static>(el: E) -> Area<E> {
    Area {
        id: ElementId::CodeLocation(*Location::caller()),
        el,
    }
}

#[derive(IntoElement)]
pub struct Area<E: super::Control + 'static> {
    id: ElementId,
    el: E,
}

impl<E: super::Control + 'static> RenderOnce for Area<E> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let h = handle(self.id.clone(), window, cx);
        self.el.track_scroll(&h).child(Bar {
            id: self.id,
            handle: h,
        })
    }
}

/// It paints at the handle's own bounds, not where it is laid out, so it sits
/// in a zero-size box: as a child of the area it adds nothing to the content
/// size the area scrolls over (a full-size child would). Being the last child,
/// it paints over the rows and leaves every row's index unchanged.
///
/// gpui-base shows a bar for 2 s after ANY offset change, a deep link or a
/// restored position included. So the bar is only attached once the pointer
/// has moved or scrolled over the area: until then a programmatic scroll
/// flashes nothing (and goldens, which never move a mouse, show no bar).
#[derive(IntoElement)]
pub struct Bar {
    id: ElementId,
    handle: ScrollHandle,
}

impl RenderOnce for Bar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let armed = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let on = *armed.read(cx);
        let p = theme::tokens(cx);
        let (rest, hover, drag) = (p.border_strong, p.border_hover, p.accent_strong);
        let clear = gpui_kit::transparent_black();
        div()
            .absolute()
            .size_0()
            .when(!on, |d| {
                d.child(Watch {
                    handle: self.handle.clone(),
                    armed,
                })
            })
            .when(on, |d| {
                d.child(
                    Scrollbar::vertical(&self.handle)
                        .id(self.id)
                        .mode(ScrollbarMode::Hover)
                        .styles(|s| {
                            s.track(|t| t.bg(clear).border_color(clear).width(size::SCROLLBAR))
                                .thumb(|t| {
                                    t.bg(rest)
                                        .width(size::SCROLL_THUMB)
                                        .inset((size::SCROLLBAR - size::SCROLL_THUMB) / 2.)
                                        .radius(radius::PILL)
                                })
                                .thumb_hover(|t| t.bg(hover))
                                .thumb_active(|t| t.bg(drag))
                        }),
                )
            })
    }
}

/// Arms the bar on the first pointer move or wheel over the area.
struct Watch {
    handle: ScrollHandle,
    armed: Entity<bool>,
}

impl IntoElement for Watch {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Watch {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.size.width = px(0.).into();
        style.size.height = px(0.).into();
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let arm = |handle: ScrollHandle, armed: Entity<bool>| {
            move |at: gpui_kit::Point<Pixels>, phase: DispatchPhase, cx: &mut App| {
                if phase == DispatchPhase::Bubble && handle.bounds().contains(&at) {
                    armed.update(cx, |on, cx| {
                        if !*on {
                            *on = true;
                            cx.notify();
                        }
                    });
                }
            }
        };
        let on_move = arm(self.handle.clone(), self.armed.clone());
        window
            .on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| on_move(e.position, phase, cx));
        let on_wheel = arm(self.handle.clone(), self.armed.clone());
        window.on_mouse_event(move |e: &ScrollEvent, phase, _, cx| on_wheel(e.position, phase, cx));
    }
}

#[cfg(test)]
#[path = "scroll_tests.rs"]
mod tests;
