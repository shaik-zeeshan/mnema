//! Renders the banner (`.mx-banner`, between the titlebar and the surface) and
//! the receipt (the shared `ui::dialog::Modal`). Everything it shows comes from
//! `banner::shown` / `receipt::copy`.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, App, Entity, Role, SharedString};

use super::banner::{self, Action};
use super::receipt::{self, Face};
use super::Licence;
use crate::theme::{self, size, space, text};
use crate::ui::chip::{chip, ChipKind};
use crate::ui::dialog::Modal;
use crate::ui::notice::{self, strong};
use crate::ui::{icon, progress, Button};

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
    let offset = crate::ui::clock::now(cx).offset();
    let copy = receipt::copy(face, flow, r.slow, offset);
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
    let glyph: AnyElement = match face {
        Face::Working => progress::spinner(size::SPIN_LG).into_any_element(),
        Face::Activated { .. } | Face::Renewed { .. } => div()
            .text_color(p.accent)
            .child(icon(IconName::Check.path(), size::SPIN_LG))
            .into_any_element(),
        Face::Pending { .. } => div()
            .text_color(p.accent)
            .child(icon(IconName::CircleDashed.path(), size::SPIN_LG))
            .into_any_element(),
        Face::OverCap { .. } | Face::Failed { .. } => div()
            .text_color(p.warn)
            .child(icon(IconName::TriangleAlert.path(), size::SPIN_LG))
            .into_any_element(),
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
        Face::Working => vec![Button::new("receipt-dismiss", "Dismiss")
            .ghost()
            .on_click(on(|l, cx| {
                l.close_receipt(cx);
            }))
            .into_any_element()],
        Face::Activated { .. } | Face::Pending { .. } | Face::Renewed { .. } => {
            vec![Button::new("receipt-continue", "Continue")
                .primary()
                .on_click(on(|l, cx| {
                    l.close_receipt(cx);
                }))
                .into_any_element()]
        }
        Face::OverCap { reset_url, buy_url } => vec![
            open_link("receipt-buy", "Buy another license", buy_url),
            open_link("receipt-help", "Help", reset_url),
            Button::new(
                "receipt-reset",
                if r.resetting {
                    "Freeing up\u{2026}"
                } else {
                    "Free up my devices"
                },
            )
            .primary()
            .busy(r.resetting)
            .disabled(r.resetting)
            .on_click(on(Licence::reset_devices))
            .into_any_element(),
        ],
        Face::Failed { .. } => vec![
            Button::new("receipt-close", "Close")
                .ghost()
                .on_click(on(|l, cx| {
                    l.close_receipt(cx);
                }))
                .into_any_element(),
            Button::new("receipt-settings", "Open License Settings")
                .primary()
                .on_click(on(|l, cx| {
                    l.close_receipt(cx);
                    cx.defer(|cx| crate::window::open_settings(Some("license"), cx));
                }))
                .into_any_element(),
        ],
    };
    let line = |s: String, color| div().text_color(color).child(SharedString::from(s));
    let body = div()
        .id("receipt-body")
        .role(Role::Group)
        .aria_label(copy.title)
        .flex()
        .flex_col()
        .gap(space::S1)
        .px(space::S5)
        .py(space::S4)
        .text_size(text::MD)
        .children(
            copy.badge
                .map(|b| div().child(chip(b, ChipKind::Neutral, &p))),
        )
        .children(copy.who.map(|w| line(w, p.text)))
        .children(copy.sub.map(|s| {
            line(s, p.text_muted)
                .text_size(text::BASE)
                .line_height(gpui_kit::relative(1.55))
        }))
        .children(
            copy.was
                .map(|w| line(w, p.text_faint).text_size(text::BASE).line_through()),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(space::S2)
                .pt(space::S3)
                .children(buttons),
        )
        .children(r.reset_note.clone().map(|n| {
            line(n, p.text_muted)
                .id("receipt-reset-note")
                .role(Role::Status)
                .text_size(text::BASE)
                .test_support()
        }));
    Some(
        Modal::new("licence-receipt", copy.title)
            .max_w(size::RECEIPT_MAX_W)
            .lead(glyph)
            .child(body)
            .on_close(close)
            .into_any_element(),
    )
}
