//! `ReloadButton.svelte`: a ghost icon button whose icon becomes the kit spinner
//! while a reload runs, and for at least [`MIN_SPIN`] after a click, so a fast
//! reload still reads as one.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, AnyElement, Context, MouseButton};

use super::{icons, Settings};
use crate::theme::{radius, size};
use crate::ui::{icon, progress, with_tip};

/// `MIN_SPIN_MS`.
pub const MIN_SPIN: Duration = Duration::from_millis(500);

/// When each reload button's minimum spin ends.
#[derive(Default)]
pub struct Reloads {
    until: HashMap<&'static str, Instant>,
}

impl Reloads {
    pub fn click(&mut self, id: &'static str, now: Instant) {
        self.until.insert(id, now + MIN_SPIN);
    }
    /// Spins while its latch runs or a real reload (`busy`) is in flight.
    pub fn spinning(&self, id: &'static str, now: Instant, busy: bool) -> bool {
        busy || self.until.get(id).is_some_and(|until| now < *until)
    }
}

/// One reload button; `action` runs the reload.
pub fn button(
    s: &Settings,
    id: &'static str,
    label: &'static str,
    busy: bool,
    action: fn(&mut Settings, &mut Context<Settings>),
    cx: &mut Context<Settings>,
) -> AnyElement {
    let p = crate::theme::tokens(cx);
    let now = cx.background_executor().now();
    let spinning = s.reload.spinning(id, now, busy);
    let this = cx.entity().downgrade();
    let el = div()
        .id(id)
        .test_support()
        .role(gpui_kit::Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(size::H_SM)
        .rounded(radius::BTN_SM)
        .text_color(p.text_muted)
        .map(|el| {
            if spinning {
                el.child(progress::spinner(size::SPIN_SM))
            } else {
                el.cursor_pointer()
                    .hover(|st| st.bg(p.wash_strong).text_color(p.text_strong))
                    .child(icon(icons::RELOAD, size::BTN_ICON_SM))
            }
        })
        .when(!busy, |el| {
            el.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                let _ = this.update(cx, |s, cx| {
                    let now = cx.background_executor().now();
                    s.reload.click(id, now);
                    action(s, cx);
                    // Repaint once the latch runs out.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(MIN_SPIN).await;
                        let _ = this.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                    cx.notify();
                });
            })
        });
    with_tip(el, label).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spins_at_least_min_spin() {
        let t0 = Instant::now();
        let mut r = Reloads::default();
        assert!(!r.spinning("a", t0, false));
        r.click("a", t0);
        assert!(r.spinning("a", t0 + Duration::from_millis(499), false));
        assert!(!r.spinning("a", t0 + Duration::from_millis(500), false));
        assert!(r.spinning("a", t0 + Duration::from_millis(900), true));
        assert!(!r.spinning("b", t0, false));
    }
}
