//! `.mx-pop`: raised surface, overlay border, radius 12, popover shadow, and the
//! open motion. The CSS blur(22) is a solid surface here (`POP_SURFACE_ALPHA`).

use gpui_kit::prelude::*;
use gpui_kit::AnimationExt as _;
use gpui_kit::{
    deferred, div, px, App, Bounds, BoxShadow, Deferred, Div, ElementId, Entity, MouseDownEvent,
    Pixels, Stateful, Window,
};

use crate::theme::{self, radius, text, Palette};

/// `0 8px 24px rgba(0,0,0,.22)`.
pub fn popover_shadow(p: &Palette) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: p.shadow_popover,
        offset: gpui_kit::point(px(0.), px(8.)),
        blur_radius: px(24.),
        spread_radius: px(0.),
        inset: false,
    }]
}

gpui_kit::actions!(popover, [ClosePopover]);

/// The key context an open list/popover's wrapper carries. Its Esc binding
/// (`keymap.rs`) is deeper than the shell's "Back to app" Esc, so it wins:
/// GPUI runs bindings before any key handler, so a control's own Esc
/// `on_key_down` never ran under the shell's binding.
pub const OPEN_CONTEXT: &str = "OpenPopover";

/// While `open`, Esc inside `wrapper` (the trigger or the list) runs `close`.
pub fn esc_closes(
    wrapper: Div,
    open: bool,
    close: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    wrapper.when(open, |d| {
        d.key_context(OPEN_CONTEXT)
            .on_action(move |_: &ClosePopover, window, cx| close(window, cx))
    })
}

/// Paint layers for [`float`]-ed overlays (higher paints on top). GPUI paints
/// in tree order, so a plain `absolute` child is covered by every later sibling.
pub const LAYER_POPOVER: usize = 1;
/// `ui::dialog::Modal`'s scrim + panel.
pub const LAYER_MODAL: usize = 2;
/// A popover or list opened from inside a modal: deferred draws paint by
/// priority across the whole window, so one at `LAYER_POPOVER` would paint
/// under the modal that holds it.
pub const LAYER_IN_MODAL: usize = 3;

/// [`float`] for an overlay that lives inside a modal.
pub fn float_in_modal(el: impl IntoElement) -> Deferred {
    deferred(el).with_priority(LAYER_IN_MODAL)
}

/// Lift a positioned popover, menu or list out of tree order: it paints after
/// the whole window (above content that comes later in the tree) and outside
/// every ancestor's clip, while keeping its place in layout. Every floating
/// surface that isn't the last thing its window draws goes through this.
pub fn float(el: impl IntoElement) -> Deferred {
    deferred(el).with_priority(LAYER_POPOVER)
}

/// An overlay's trigger as last painted (keyed element state). An outside-click
/// close must skip the trigger: the trigger toggles, so closing on its
/// mouse-down reopened the overlay on its click. [`Anchor::opens_up`] picks the
/// side like `popover-direction.ts`.
#[derive(Clone)]
pub struct Anchor(Entity<Bounds<Pixels>>);

impl Anchor {
    pub fn new(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Self {
        Self(window.use_keyed_state(id, cx, |_, _| Bounds::default()))
    }

    /// An anchor a view owns, for a trigger and overlay drawn by different code.
    pub fn owned(cx: &mut App) -> Self {
        Self(cx.new(|_| Bounds::default()))
    }

    /// Put on the wrapper whose FIRST child is the trigger: records its bounds
    /// (read on the next render).
    pub fn track(&self, wrapper: Div) -> Div {
        self.record(wrapper, false)
    }

    /// [`Anchor::track`] for a measured popover: a new size re-renders the
    /// owner, so the side it picks from [`Anchor::height`] lands next frame.
    pub fn measure(&self, wrapper: Div) -> Div {
        self.record(wrapper, true)
    }

    fn record(&self, wrapper: Div, notify: bool) -> Div {
        let b = self.0.clone();
        wrapper.on_children_prepainted(move |bounds, _, cx| {
            if let Some(t) = bounds.first().copied() {
                let was = *b.read(cx);
                if was != t {
                    b.update(cx, |x, cx| {
                        *x = t;
                        if notify && was.size != t.size {
                            cx.notify();
                        }
                    });
                }
            }
        })
    }

    /// A mouse-down outside the overlay that also missed the trigger.
    pub fn outside(&self, e: &MouseDownEvent, cx: &App) -> bool {
        !self.0.read(cx).contains(&e.position)
    }

    /// Open upward when below can't fit `needed` and above has more room.
    pub fn opens_up(&self, needed: Pixels, window: &Window, cx: &App) -> bool {
        let b = *self.0.read(cx);
        let below = window.viewport_size().height - b.bottom();
        b.size.height > px(0.) && opens_upward(below, b.top(), needed)
    }

    /// The tracked element's last painted height (0 before its first frame).
    /// The tracked element's bounds as last painted (window coordinates).
    pub fn bounds(&self, cx: &App) -> Bounds<Pixels> {
        *self.0.read(cx)
    }

    pub fn height(&self, cx: &App) -> Pixels {
        self.0.read(cx).size.height
    }
}

/// `shouldOpenUpward` (`popover-direction.ts`): conservative, down by default.
pub fn opens_upward(below: Pixels, above: Pixels, needed: Pixels) -> bool {
    below < needed && above > below
}

/// The `.mx-pop` surface. Callers position it (`absolute` + edges) and wrap it
/// in [`float`] (and [`pop_in`] for the open motion).
pub fn popover(id: &'static str, label: &'static str, p: &Palette) -> Stateful<Div> {
    surface(p)
        .id(id)
        .role(gpui_kit::Role::Dialog)
        .aria_label(label)
        .absolute()
}

/// The popover look without positioning or a11y (menus, lists, static demos).
pub fn surface(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(p.pop_bg)
        .border_1()
        .border_color(p.overlay_border)
        .rounded(radius::LG)
        .shadow(popover_shadow(p))
        .occlude()
        .font_family(theme::FONT_UI)
        .text_size(text::BASE)
        .font_weight(theme::W_REGULAR)
        .text_color(p.text)
        .whitespace_normal()
}

/// Open motion: fade in (120 ms quart) while rising 4 px into place (220 ms expo).
/// `up` = opens upward from a bottom-anchored trigger. GPUI can't scale a box, so
/// the CSS `scale(.98)` is dropped. Reduce Motion skips straight to the end.
pub fn pop_in(el: Stateful<Div>, edge: Pixels, up: bool) -> impl IntoElement {
    let fade = theme::motion::FAST.as_secs_f32() / theme::motion::MED.as_secs_f32();
    el.with_animation(
        "pop-in",
        gpui_kit::Animation::new(theme::motion::MED).with_easing(theme::motion::expo),
        move |el, t| {
            let lift = px(4.) * (1. - t);
            let el = el.opacity((t / fade).min(1.));
            if up {
                el.bottom(edge - lift)
            } else {
                el.top(edge - lift)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_upward_only_when_below_is_tight_and_above_is_roomier() {
        let n = px(220.);
        assert!(!opens_upward(px(220.), px(500.), n), "fits below");
        assert!(
            opens_upward(px(219.), px(500.), n),
            "tight below, room above"
        );
        assert!(!opens_upward(px(100.), px(100.), n), "a tie stays down");
        assert!(!opens_upward(px(100.), px(50.), n), "above is tighter");
    }
}
