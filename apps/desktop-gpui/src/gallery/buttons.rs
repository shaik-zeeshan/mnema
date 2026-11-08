//! Buttons: every variant × size × state (index.html "Buttons" matrix + states).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Window};

use super::{cell, row, Gallery};
use crate::theme::{self, space};
use crate::ui::{Button, Force, Variant};

pub(super) fn render(_: &mut Gallery, _: &mut Window, cx: &mut Context<Gallery>) -> AnyElement {
    let p = theme::tokens(cx);
    let variants = [
        (
            "primary",
            Variant::Primary,
            "Save changes",
            IconName::Download.path(),
            "Download",
        ),
        (
            "secondary",
            Variant::Secondary,
            "Export",
            IconName::Search.path(),
            "Search",
        ),
        (
            "ghost",
            Variant::Ghost,
            "Cancel",
            IconName::Settings.path(),
            "Settings",
        ),
        (
            "danger",
            Variant::Danger,
            "Delete",
            IconName::Trash.path(),
            "Delete",
        ),
    ];
    let rows = variants.into_iter().map(|(name, v, label, glyph, aria)| {
        let id = |s: &str| format!("btn-{name}-{s}");
        let b = |s: &str| Button::new(id(s), label).variant(v);
        // State columns use the short label, as index.html does ("Save").
        let (short, sm_label) = match v {
            Variant::Primary => ("Save", "Install"),
            _ => (label, label),
        };
        let st = |s: &str| Button::new(id(s), short).variant(v);
        let md = if v == Variant::Secondary {
            b("md").kbd("⌘E")
        } else {
            b("md")
        };
        let items = vec![
            cell("md 32", md, &p),
            cell(
                "sm 26",
                Button::new(id("sm"), sm_label)
                    .variant(v)
                    .sm()
                    .icon(glyph.clone()),
                &p,
            ),
            cell(
                "icon md · sm",
                div()
                    .flex()
                    .gap(space::S2)
                    .child(Button::icon_only(id("icon"), glyph.clone(), aria).variant(v))
                    .child(
                        Button::icon_only(id("icon-sm"), glyph.clone(), aria)
                            .variant(v)
                            .sm()
                            .pressed(v == Variant::Ghost),
                    ),
                &p,
            ),
            cell("hover", st("hover").force(Force::Hover), &p),
            cell("focus", st("focus").force(Force::Focus), &p),
            cell("press", st("press").force(Force::Press), &p),
            cell("pressed", st("pressed").pressed(true), &p),
            cell("disabled", st("disabled").disabled(true), &p),
            cell("busy", st("busy").busy(true), &p),
        ];
        row(name, items, &p).into_any_element()
    });
    let actions = vec![
        cell(
            "ask action",
            Button::new("btn-ask", "Ask about this week")
                .ghost()
                .sm()
                .icon(IconName::Sparkle.path())
                .accent_icon()
                .kbd("⌘J"),
            &p,
        ),
        cell(
            "busy + spinner",
            Button::new("btn-refresh", "Refreshing…")
                .sm()
                .busy(true)
                .spinner(true),
            &p,
        ),
        cell(
            "primary busy",
            Button::new("btn-install", "Installing…")
                .primary()
                .sm()
                .busy(true)
                .spinner(true),
            &p,
        ),
        cell(
            "toggle on",
            Button::new("btn-pin", "Pinned").sm().pressed(true),
            &p,
        ),
        cell(
            "soft-disabled",
            Button::new("btn-record", "Record")
                .sm()
                .soft_disabled(true)
                .tip("Trial ended — read-only"),
            &p,
        ),
        cell(
            "focus + tip",
            Button::icon_only("btn-tip", IconName::Settings.path(), "Settings")
                .ghost()
                .tip("Settings  ⌘,")
                .force(Force::Focus),
            &p,
        ),
    ];
    div()
        .flex()
        .flex_col()
        .gap(space::S4)
        .children(rows)
        .child(row("action", actions, &p))
        .into_any_element()
}
