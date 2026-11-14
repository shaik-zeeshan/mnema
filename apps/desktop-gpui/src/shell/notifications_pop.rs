//! The bell's popover (`.mx-pop.mx-notifs`, 380 px): head with count + Clear all,
//! one row per notification (tone dot, title, age, message, action), or the
//! "You're all caught up" empty state.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, relative, Context, Role};

use super::model::{NotificationRow, RowAction, Severity};
use super::Shell;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::{clock, icon, kicker, pop_in, popover, with_tip};

/// The row time's tooltip (`toLocaleString`): "11/4/2026, 2:31:56 PM".
pub fn stamp(created_ms: i64, offset: time::UtcOffset) -> String {
    let t = crate::timeline::model::local(created_ms, offset);
    format!(
        "{}/{}/{}, {}",
        t.month() as u8,
        t.day(),
        t.year(),
        crate::timeline::model::clock_label(t, true)
    )
}

/// "just now" · "12m ago" · "3h ago" · "2d ago".
pub fn age(now_ms: i64, created_ms: i64) -> String {
    let seconds = (now_ms - created_ms).max(0) / 1000;
    if seconds < 45 {
        return "just now".into();
    }
    let minutes = (seconds as f64 / 60.).round() as i64;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = (minutes as f64 / 60.).round() as i64;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", (hours as f64 / 24.).round() as i64)
}

fn ghost_sm(
    id: impl Into<gpui_kit::ElementId>,
    label: &str,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .flex()
        .items_center()
        .h(size::H_SM)
        .px(px(9.))
        .rounded(radius::BTN_SM)
        .text_size(text::BASE)
        .font_weight(theme::W_MEDIUM)
        .text_color(p.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
        .child(label.to_string())
}

fn row(
    i: usize,
    n: &NotificationRow,
    now_ms: i64,
    offset: time::UtcOffset,
    p: &Palette,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let tone = match n.severity {
        Severity::Error => p.danger_text,
        Severity::Warning => p.warn,
        Severity::Info => p.info,
    };
    let action = n.action.as_ref().map(|label| {
        div()
            .mt(px(5.))
            .flex()
            .gap(px(6.))
            .child(
                div()
                    .id(("notif-action", i))
                    .role(Role::Button)
                    .flex()
                    .items_center()
                    .h(size::H_SM)
                    .px(px(9.))
                    .rounded(radius::BTN_SM)
                    .bg(p.surface_raised)
                    .border_1()
                    .border_color(p.border_strong)
                    .text_size(text::BASE)
                    .font_weight(theme::W_MEDIUM)
                    .text_color(p.text_strong)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.surface_hover))
                    .child(label.clone())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.run_notification(i, false, cx)),
                    )
                    .test_support(),
            )
            .when(n.kind == RowAction::Reminder, |acts| {
                acts.child(
                    ghost_sm(("notif-snooze", i), "Snooze for today", p).on_click(
                        cx.listener(move |this, _, _, cx| this.run_notification(i, true, cx)),
                    ),
                )
            })
    });
    // `clear_app_notification`: on row hover it takes the time's place (`.mx-notif__x`).
    let id = n.id.clone();
    let dismiss = div()
        .id(("notif-dismiss", i))
        .role(Role::Button)
        .aria_label("Dismiss notification")
        .absolute()
        .right_0()
        .top_0()
        .flex()
        .items_center()
        .justify_center()
        .size(size::H_MINI)
        .rounded(radius::BTN_SM)
        .text_color(p.text_muted)
        .cursor_pointer()
        .invisible()
        .group_hover("notif-row", |s| s.visible())
        .hover(|s| s.bg(p.wash_strong).text_color(p.text_strong))
        .child(icon(IconName::X.path(), px(13.)))
        .on_click(cx.listener(move |this, _, _, cx| this.clear_notifications(Some(id.clone()), cx)))
        .test_support();
    let time = with_tip(
        div()
            .id(("notif-time", i))
            .font_family(FONT_MONO)
            .text_size(text::XS)
            .text_color(p.text_subtle)
            .group_hover("notif-row", |s| s.opacity(0.))
            .child(age(now_ms, n.created_at_ms)),
        stamp(n.created_at_ms, offset),
    );
    div()
        .id(("notif", i))
        .group("notif-row")
        .relative()
        .flex()
        .gap(px(10.))
        .pt(px(9.))
        .pb(px(10.))
        .pl(px(10.))
        .pr(px(8.))
        .rounded(radius::SM)
        .hover(|s| s.bg(p.wash))
        .child(
            div()
                // Centred on the 22 px first row (`.mx-notif::before { align-self: center }`).
                .mt(px(8.))
                .flex_none()
                .size(px(6.))
                .rounded_full()
                .bg(tone),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(text::MD)
                        .font_weight(theme::W_SEMIBOLD)
                        // The first grid row is the × button's 22 px (`--_h: 22px`).
                        .min_h(size::H_MINI)
                        .line_height(relative(1.35))
                        .text_color(p.text_strong)
                        .child(n.title.clone()),
                )
                .child(
                    div()
                        .text_size(text::BASE)
                        .line_height(relative(1.45))
                        .text_color(p.text_muted)
                        .child(n.message.clone()),
                )
                .children(action),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .h(size::H_MINI)
                .child(time)
                .child(dismiss),
        )
}

pub fn render(shell: &Shell, p: &Palette, cx: &mut Context<Shell>) -> impl IntoElement {
    let rows = &shell.model.notifications;
    let now_ms = clock::now_ms(cx);
    let offset = clock::now(cx).offset();
    let head = div()
        .flex()
        .items_center()
        .gap(space::S2)
        .h(px(40.))
        .pl(space::S4)
        .pr(px(6.))
        .border_b_1()
        .border_color(p.hairline)
        .child(kicker("Notifications", p))
        .when(!rows.is_empty(), |h| {
            h.child(
                div()
                    .font_family(FONT_MONO)
                    .text_size(text::XS)
                    .text_color(p.text_subtle)
                    .child(rows.len().to_string()),
            )
            .child(div().flex_1())
            .child(
                ghost_sm("notifs-clear", "Clear all", p)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_notifications(None, cx))),
            )
        });
    let body = if rows.is_empty() {
        crate::ui::empty::empty(
            crate::ui::empty::EmptyLayout::Compact,
            crate::ui::Tone::Neutral,
            IconName::Bell.path(),
            "You’re all caught up",
            "Capture, model and update warnings land here.",
            Vec::new(),
            p,
        )
    } else {
        // `.mx-notifs__list`: 380 px tall at most, then it scrolls.
        div().child(crate::ui::scroll::area(
            div()
                .id("notifications-list")
                .flex()
                .flex_col()
                .gap(px(1.))
                .p(space::S1)
                .max_h(px(380.))
                .overflow_y_scroll()
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(i, n)| row(i, n, now_ms, offset, p, cx))
                        .collect::<Vec<_>>(),
                ),
        ))
    };
    // Right edge on the bell's: titlebar right padding + gear + the 2px gap.
    let right = space::S2 + size::H_SM + px(2.);
    pop_in(
        popover("notifications-popover", "Notifications", p)
            .right(right)
            .w(size::POP_NOTIFS_W)
            .child(head)
            .child(body),
        // `.mx-pop { top: calc(100% + 8px) }` under the bell button, centred in the bar.
        (size::TITLEBAR + size::H_SM) / 2. + size::POP_GAP,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{age, stamp};

    #[test]
    fn ages_round_like_the_svelte_popover() {
        let now = 1_000_000_000;
        assert_eq!(age(now, now - 44_000), "just now");
        assert_eq!(age(now, now - 45_000), "1m ago");
        assert_eq!(age(now, now - 20 * 60_000), "20m ago");
        assert_eq!(age(now, now - 90 * 60_000), "2h ago");
        assert_eq!(age(now, now - 36 * 3_600_000), "2d ago");
    }

    #[test]
    fn stamp_is_the_local_date_and_time() {
        // 2024-11-14 01:46:40 UTC.
        assert_eq!(
            stamp(1_731_548_800_000, time::UtcOffset::UTC),
            "11/14/2024, 1:46:40 AM"
        );
    }
}
