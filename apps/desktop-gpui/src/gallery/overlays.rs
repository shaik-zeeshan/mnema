//! Popover surface, menu, tooltip, toasts, modal. Static specimens (the live
//! ones open from the buttons in the gallery window).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Window};

use super::{cell, panel, row, Gallery};
use crate::theme::{self, space, text};
use crate::ui::dialog::Modal;
use crate::ui::menu::{menu, menu_item};
use crate::ui::toast::{toast_box, Toast};
use crate::ui::tooltip::tip_body;
use crate::ui::{kicker, popover, Button, Force};

pub(super) fn render(g: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let pop = popover::surface(&p)
        .w(px(260.))
        .child(
            div()
                .px(space::S4)
                .pt(space::S3)
                .pb(space::S2)
                .child(kicker("Popover", &p)),
        )
        .child(
            div()
                .px(space::S4)
                .pb(space::S4)
                .text_size(text::BASE)
                .text_color(p.text_muted)
                .child("Raised surface, overlay border, radius 12, the popover shadow."),
        );
    let items = [
        ("Copy frame", Some("⌘C"), Force::None),
        ("Open in Finder", None, Force::Hover),
        ("Ask about this moment", Some("⌘J"), Force::None),
    ];
    let menu_el = div().w(px(220.)).child(menu(
        items
            .into_iter()
            .enumerate()
            .map(|(i, (label, meta, force))| {
                menu_item(
                    ("menu-item", i),
                    label,
                    meta.map(Into::into),
                    force,
                    |_, _, _| {},
                    &p,
                )
            }),
        &p,
    ));
    let tips = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(space::S3)
        .child(tip_body("Start recording  ⌥⌘R".into(), &p))
        .child(tip_body("Screen: on — click to skip next time".into(), &p));
    let toasts = div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(8.))
        .child(toast_box(
            &Toast::ok("Deleted the last 15 minutes").action("Undo", true, |_, _| {}),
            900,
            &p,
        ))
        .child(toast_box(&Toast::ok("Link copied"), 901, &p))
        .child(toast_box(
            &Toast::danger("Couldn’t export — the disk is full").action("Retry", false, |_, _| {}),
            902,
            &p,
        ));
    let toaster = g.toasts.clone();
    let live = Button::new("show-toast", "Show a live toast")
        .sm()
        .on_click(move |_, _, cx| {
            toaster.update(cx, |t, cx| {
                t.push(
                    Toast::ok("Deleted 3 moments").action("Undo", true, |_, _| {}),
                    cx,
                );
            });
        });
    let this = cx.weak_entity();
    let (open, close) = (this.clone(), this);
    let modal_btn = Button::new("open-modal", "Open a modal")
        .sm()
        .on_click(move |_, _, cx| {
            open.update(cx, |g, cx| {
                g.modal_open = true;
                cx.notify();
            })
            .ok();
        });
    let swatch = div().size(px(12.)).rounded(px(3.)).bg(p.cat_research);
    let modal = Modal::new("gallery-modal", "Research")
        .lead(swatch)
        .child(
            div()
                .p(space::S5)
                .text_size(text::MD)
                .text_color(p.text_muted)
                .child("3h 12m this week across 6 apps. Scrim 62%, raised panel, radius 16."),
        )
        .on_close(move |_, cx| {
            close
                .update(cx, |g, cx| {
                    g.modal_open = false;
                    cx.notify();
                })
                .ok();
        });
    // Static specimen: the modal over a framed stand-in window.
    let frame = panel(&p)
        .relative()
        .w(px(520.))
        .h(px(250.))
        .overflow_hidden()
        .child(
            div()
                .p(space::S4)
                .text_color(p.text_subtle)
                .child("Window content under the scrim"),
        )
        .child(modal);
    div()
        .flex()
        .flex_col()
        .gap(space::S5)
        .child(
            row(
                "surfaces",
                vec![
                    cell("popover", pop, &p),
                    cell("menu · hover (2)", menu_el, &p),
                    cell("tooltip", tips, &p),
                ],
                &p,
            )
            .items_start(),
        )
        .child(
            row(
                "toast",
                vec![
                    cell("undo (drain) · ok · danger + retry", toasts, &p),
                    cell(
                        "live",
                        div().flex().gap(space::S2).child(live).child(modal_btn),
                        &p,
                    ),
                ],
                &p,
            )
            .items_start(),
        )
        .child(row("modal", vec![cell("open", frame, &p)], &p).items_start())
        .when(g.modal_open, |el| {
            let close = cx.weak_entity();
            el.child(
                Modal::new("live-modal", "Live modal")
                    .child(div().p(space::S5).child("Esc, the scrim or × closes it."))
                    .child(
                        div().px(space::S5).pb(space::S5).child(
                            Button::new("modal-ok", "Done")
                                .primary()
                                .icon(IconName::Check.path()),
                        ),
                    )
                    .on_close(move |_, cx| {
                        close
                            .update(cx, |g, cx| {
                                g.modal_open = false;
                                cx.notify();
                            })
                            .ok();
                    }),
            )
        })
        .into_any_element()
}
