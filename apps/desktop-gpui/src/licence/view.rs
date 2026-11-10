//! Renders the banner (`.mx-banner`, between the titlebar and the surface) and
//! the receipt (the shared `ui::dialog::Modal`, unpainted: the receipt draws its
//! own paper). Everything it shows comes from `banner::shown` / `receipt::copy`;
//! the device limit is a ticket (`ticket.rs`), every other face the ruler
//! (`ruler.rs`).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{canvas, div, px, AnyElement, App, Entity, Role, SharedString};

use super::banner::{self, Action};
use super::parts::Parts;
use super::receipt::{self, Face};
use super::Licence;
use super::{ruler, ticket};
use crate::theme::{self, size, text};
use crate::ui::dialog::Modal;
use crate::ui::notice::{self, strong};
use crate::ui::Button;

/// The one shell banner, if any.
pub fn banner(licence: &Entity<Licence>, cx: &App) -> Option<AnyElement> {
    let l = licence.read(cx);
    let shown = banner::shown(l.status.as_ref(), l.low_disk, l.dismissed)?;
    let p = theme::tokens(cx);
    let weak = licence.downgrade();
    let mut acts: Vec<AnyElement> = shown
        .actions
        .iter()
        .map(|&action| {
            let busy = action == Action::Recheck && l.rechecking;
            let label = if busy {
                "Checking\u{2026}"
            } else {
                action.label()
            };
            let weak = weak.clone();
            Button::new(action.id(), label)
                .sm()
                .when(action.ghost(), |b| b.ghost())
                .when(busy, |b| b.busy(true).spinner(true).disabled(true))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |l, cx| l.act(action, cx));
                })
                .into_any_element()
        })
        .collect();
    if let Some(d) = shown.dismiss {
        acts.push(
            Button::icon_only("licence-banner-dismiss", IconName::X.path(), "Dismiss")
                .ghost()
                .sm()
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |l, cx| l.dismiss(d, cx));
                })
                .into_any_element(),
        );
    }
    let message = div()
        .flex()
        .flex_wrap()
        .gap_x(px(4.))
        .children(shown.bold.map(|b| strong(b, &p)))
        .child(shown.text)
        .into_any_element();
    Some(
        notice::banner(shown.tone, message, acts, &p)
            .id("licence-banner")
            .role(if shown.alert {
                Role::Alert
            } else {
                Role::Status
            })
            .test_support()
            .into_any_element(),
    )
}

/// The deep-link receipt, while it is open.
pub fn receipt(licence: &Entity<Licence>, cx: &App) -> Option<AnyElement> {
    let l = licence.read(cx);
    let r = &l.receipt;
    let (face, flow) = (r.face.as_ref().filter(|_| r.open)?, r.flow?);
    let p = theme::tokens(cx);
    let now = crate::ui::clock::now(cx);
    let copy = receipt::copy(face, flow, r.slow, now.offset());
    let title = copy.title;
    let weak = licence.downgrade();
    let close = {
        let weak = weak.clone();
        move |_: &mut gpui_kit::Window, cx: &mut App| {
            let _ = weak.update(cx, |l, cx| l.close_receipt(cx));
        }
    };
    let on = |f: fn(&mut Licence, &mut gpui_kit::Context<Licence>)| {
        let weak = weak.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut App| {
            let _ = weak.update(cx, |l, cx| f(l, cx));
        }
    };
    let close_it = |l: &mut Licence, cx: &mut gpui_kit::Context<Licence>| {
        l.close_receipt(cx);
    };
    // The primary action holds the receipt's focus (Enter presses it).
    let primary = |b: Button| match &l.primary {
        Some(h) => b.focus_handle(h),
        None => b,
    };
    let open_link = |id: &'static str, label: &'static str, url: &str| {
        let url = receipt::safe_external(url).map(str::to_string);
        Button::new(id, label)
            .ghost()
            .on_click(move |_, _, cx| {
                if let Some(url) = &url {
                    cx.open_url(url);
                }
            })
            .into_any_element()
    };
    let buttons: Vec<AnyElement> = match face {
        Face::Working => vec![primary(Button::new("receipt-dismiss", "Dismiss"))
            .ghost()
            .on_click(on(close_it))
            .into_any_element()],
        Face::Activated { .. } | Face::Pending { .. } | Face::Renewed { .. } => {
            vec![primary(Button::new("receipt-continue", "Continue"))
                .primary()
                .on_click(on(close_it))
                .into_any_element()]
        }
        Face::OverCap { reset_url, buy_url } => vec![
            open_link("receipt-buy", "Buy another license", buy_url),
            open_link("receipt-help", "Help", reset_url),
            primary(Button::new(
                "receipt-reset",
                if r.resetting {
                    "Freeing up\u{2026}"
                } else {
                    "Free up my devices"
                },
            ))
            .primary()
            .busy(r.resetting)
            .disabled(r.resetting)
            .on_click(on(Licence::reset_devices))
            .into_any_element(),
        ],
        Face::Failed { .. } => vec![
            Button::new("receipt-close", "Close")
                .ghost()
                .on_click(on(close_it))
                .into_any_element(),
            primary(Button::new("receipt-settings", "Open License Settings"))
                .primary()
                .on_click(on(|l, cx| {
                    l.close_receipt(cx);
                    cx.defer(|cx| crate::window::open_settings(Some("license"), cx));
                }))
                .into_any_element(),
        ],
    };
    let note = r.reset_note.clone().map(|n| {
        div()
            .id("receipt-reset-note")
            .role(Role::Status)
            .text_size(text::BASE)
            .text_color(p.text_muted)
            .child(SharedString::from(n))
            .test_support()
            .into_any_element()
    });
    let parts = Parts {
        copy,
        buttons,
        note,
        now,
    };
    let body = match face {
        Face::OverCap { .. } => ticket::render(parts, &p),
        _ => ruler::render(face, parts, &p),
    };
    Some(
        Modal::new("licence-receipt", title)
            .bare()
            .unpainted()
            .max_w(size::RECEIPT_MAX_W)
            .child(body)
            .child(focus_primary(licence, face, r.generation))
            .on_close(close)
            .into_any_element(),
    )
}

/// Moves focus to the primary action once per face (on open and on each
/// morph), during paint where the window is at hand.
fn focus_primary(licence: &Entity<Licence>, face: &Face, generation: u64) -> impl IntoElement {
    let key = (generation, std::mem::discriminant(face));
    let weak = licence.downgrade();
    canvas(
        move |_, window, cx| {
            let Some(l) = weak.upgrade() else { return };
            let (focused, handle) = {
                let l = l.read(cx);
                (l.focused == Some(key), l.primary.clone())
            };
            if focused {
                return;
            }
            l.update(cx, |l, _| l.focused = Some(key));
            if let Some(h) = handle {
                window.defer(cx, move |window, cx| h.focus(window, cx));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}
