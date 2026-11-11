//! TimelineJumper (`lib/timeline/{TimelineJumper,JumperCalendar,JumperTimeList}.svelte`,
//! `jumper-time.ts`): the trigger (day · time), and a popover with the month
//! (density dots) and the picked day's hours. Picking a day only previews it;
//! an hour (latest frame at or before its end) or "latest of day" commits and
//! closes. The "latest at or before" lookup and the focused window stay backend-owned.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Role, SharedString};
use time::{Date, Duration, OffsetDateTime, Time, UtcOffset};

use super::model::{clock_label, day_label, local};
use super::TimelineView;
use crate::theme::{self, radius, size, space, text, Palette, FONT_MONO};
use crate::ui::calendar::{calendar, month_of, shift_cursor, Body, CalSpec, Foot, GridKeys};
use crate::ui::{icon, nid, popover, with_tip, Button};

#[derive(Default)]
pub struct JumperState {
    pub open: bool,
    pub jumping: bool,
    pub error: Option<String>,
    /// The month on screen and the previewed day.
    pub view: Option<Date>,
    pub picked: Option<Date>,
    /// Capture instants of the viewed month (`None` while loading).
    pub times: Option<Vec<i64>>,
    /// The day grid's keyboard cursor (bits-ui's focused day); `cursor()` resolves it.
    pub cursor: Option<Date>,
    /// Move focus into the day grid on the next render (opening moves focus inside).
    pub focus_grid: bool,
}

fn offset(cx: &gpui_kit::App) -> UtcOffset {
    crate::ui::clock::now(cx).offset()
}

fn day_start_ms(d: Date, o: UtcOffset) -> i64 {
    (d.with_time(Time::MIDNIGHT)
        .assume_offset(o)
        .unix_timestamp_nanos()
        / 1_000_000) as i64
}

/// `dayHeat`: 0..=3 against the month's busiest day.
pub fn day_heat(count: usize, month_max: usize) -> u8 {
    if count == 0 {
        return 0;
    }
    (count as f32 / count.max(month_max) as f32 * 3.)
        .ceil()
        .min(3.) as u8
}

impl TimelineView {
    pub fn open_jumper(&mut self, cx: &mut Context<Self>) {
        if self.jumper.open {
            return;
        }
        let o = offset(cx);
        let day = self
            .paging
            .active_frame()
            .map(|f| local(f.captured_at_ms, o).date())
            .unwrap_or_else(|| crate::ui::clock::now(cx).date());
        self.jumper.open = true;
        self.jumper.error = None;
        self.jumper.picked = Some(day);
        self.jumper.cursor = Some(day);
        self.jumper.focus_grid = true;
        self.view_month(day, cx);
    }

    /// Preview `d` (a click, or Enter / Space on the cursor): picking only previews.
    fn preview_day(&mut self, d: Date, cx: &mut Context<Self>) {
        self.jumper.picked = Some(d);
        self.jumper.cursor = Some(d);
        if month_of(d, 0).0 != self.jumper.view.unwrap_or(d) {
            self.view_month(d, cx);
        }
        cx.notify();
    }

    /// The cursor if it is a pickable day of the month on screen, else the
    /// previewed day, else the month's first pickable day (bits-ui's placeholder).
    fn cursor(&self, today: Date) -> Option<Date> {
        let view = self.jumper.view?;
        let shown = |d: &Date| month_of(*d, 0).0 == view && *d <= today;
        self.jumper
            .cursor
            .filter(shown)
            .or(self.jumper.picked.filter(shown))
            .or((view <= today).then_some(view))
    }

    /// bits-ui's grid keys: ←/→ a day, ↑/↓ a week (rolling across months),
    /// Enter / Space preview the cursor's day. True when handled.
    fn jumper_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let today = crate::ui::clock::now(cx).date();
        let (Some(view), Some(cursor)) = (self.jumper.view, self.cursor(today)) else {
            return false;
        };
        let add = match key {
            "left" => -1,
            "right" => 1,
            "up" => -7,
            "down" => 7,
            "enter" | "space" => {
                self.preview_day(cursor, cx);
                return true;
            }
            _ => return false,
        };
        if let Some((d, first)) = shift_cursor(cursor, add, view, Date::MIN, today) {
            self.jumper.cursor = Some(d);
            if first != view {
                self.view_month(first, cx);
            }
            cx.notify();
        }
        true
    }

    fn view_month(&mut self, day: Date, cx: &mut Context<Self>) {
        let o = offset(cx);
        let (first, last) = month_of(day, 0);
        let (start, end) = (
            day_start_ms(first, o),
            day_start_ms(last + Duration::days(1), o),
        );
        self.jumper.view = Some(first);
        self.jumper.times = None;
        cx.notify();
        let src = self.source.clone();
        cx.spawn(async move |this, cx| {
            let times = cx
                .background_executor()
                .spawn(async move { src.frame_times(start, end) })
                .await;
            this.update(cx, |view, cx| {
                if view.jumper.view == Some(first) {
                    match times {
                        Ok(t) => view.jumper.times = Some(t),
                        Err(e) => {
                            view.jumper.times = Some(Vec::new());
                            view.jumper.error = Some(e);
                        }
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Commit: the latest frame at or before the end of `hour` (`None` = the day's end).
    fn commit(&mut self, hour: Option<u8>, cx: &mut Context<Self>) {
        let Some(day) = self.jumper.picked else {
            return;
        };
        let o = offset(cx);
        let start = day_start_ms(day, o);
        let end = start + i64::from(hour.map_or(24, |h| h + 1)) * 3_600_000;
        self.jumper.jumping = true;
        cx.notify();
        let src = self.source.clone();
        cx.spawn(async move |this, cx| {
            let frame = cx
                .background_executor()
                .spawn(async move { src.latest_in_range(start, end) })
                .await;
            this.update(cx, |view, cx| match frame {
                Ok(Some(f)) => view.jump(f.id, cx),
                Ok(None) => {
                    view.jumper.jumping = false;
                    view.jumper.error = Some("No frames in that hour".into());
                    cx.notify();
                }
                Err(e) => {
                    view.jumper.jumping = false;
                    view.jumper.error = Some(e);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// The bar's trigger: calendar · day · mono time.
pub fn trigger(v: &TimelineView, p: &Palette, cx: &mut Context<TimelineView>) -> AnyElement {
    let o = offset(cx);
    let at = v.paging.active_frame().map(|f| local(f.captured_at_ms, o));
    let day = match at {
        Some(t) => day_label(t),
        None if v.paging.loading || v.paging.loading_more => "Loading…".into(),
        None => "No frames".into(),
    };
    let today = crate::ui::clock::now(cx).date();
    let tip = match at.map(|t| (today - t.date()).whole_days()) {
        None => "Jump to date and time  J".to_string(),
        Some(d) if d <= 0 => "Today · jump to date and time  J".into(),
        Some(1) => "Yesterday · jump to date and time  J".into(),
        Some(d) => format!("{d} days ago · jump to date and time  J"),
    };
    let border = if v.jumper.open {
        p.accent_border
    } else {
        p.border_strong
    };
    let hover = p.surface_hover;
    let btn = div()
        .id("jumper-trigger")
        .test_support()
        .role(Role::Button)
        .aria_label("Jump to date and time")
        .aria_expanded(v.jumper.open)
        .flex()
        .items_center()
        .gap(px(7.))
        .h(size::CMD_H)
        .pl(px(9.))
        .pr(px(10.))
        .rounded(radius::SM)
        .border_1()
        .border_color(border)
        .bg(p.surface_raised)
        .text_color(p.text_strong)
        // Inert until there is a frame (`disabled={!activeFrame}`; the kit's
        // date button doesn't dim when disabled, so neither does this).
        .when(at.is_some(), |d| {
            d.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .child(
            div()
                .text_color(p.text_subtle)
                .child(icon(IconName::Calendar.path(), px(14.))),
        )
        .child(
            div()
                .text_size(text::MD)
                .font_weight(theme::W_MEDIUM)
                .child(SharedString::from(day)),
        )
        .children(at.map(|t| {
            div()
                .flex()
                .items_center()
                .h(px(14.))
                .pl(px(8.))
                .border_l_1()
                .border_color(p.hairline)
                .font_family(FONT_MONO)
                .text_size(text::BASE)
                .font_weight(theme::W_MEDIUM)
                .text_color(p.text_muted)
                .child(clock_label(t, true))
        }))
        .when(at.is_some(), |d| {
            d.on_click(cx.listener(|this, _, _, cx| {
                if this.jumper.open {
                    this.jumper.open = false;
                    cx.notify();
                } else {
                    this.open_jumper(cx);
                }
            }))
        });
    v.jumper_at
        .track(div().flex().flex_none())
        .child(with_tip(btn, tip))
        .into_any_element()
}

/// "latest" (only when the playhead is more than 50 frames from the head).
pub fn latest_button(id: &'static str, v: &TimelineView, cx: &mut Context<TimelineView>) -> Button {
    let busy = v.paging.loading || v.paging.loading_more || v.jumper.jumping;
    Button::new(id, "latest")
        .sm()
        .h(size::CMD_H)
        .icon(IconName::ArrowRightToLine.path())
        .accent_icon()
        .kbd("L")
        .disabled(busy)
        .tip("Jump to latest frame")
        .on_click(cx.listener(|this, _, _, cx| this.jump_latest(cx)))
}

/// `showJumpToLatestButton`: more than 50 frames from the live head.
pub fn show_latest(v: &TimelineView) -> bool {
    !v.paging.frames.is_empty() && v.paging.active + usize::from(v.paging.has_newer) > 50
}

fn hour_rows(
    v: &TimelineView,
    day: Date,
    p: &Palette,
    cx: &mut Context<TimelineView>,
) -> Vec<AnyElement> {
    let o = offset(cx);
    let now = crate::ui::clock::now(cx);
    let last = if day == now.date() { now.hour() } else { 23 };
    let Some(times) = &v.jumper.times else {
        return (0..7)
            .map(|_| {
                crate::ui::skeleton::skel(p)
                    .h(px(22.))
                    .mx(px(4.))
                    .mb(px(6.))
                    .into_any_element()
            })
            .collect();
    };
    let mut counts = [0usize; 24];
    for t in times {
        let l = local(*t, o);
        if l.date() == day {
            counts[l.hour() as usize] += 1;
        }
    }
    if counts.iter().all(|c| *c == 0) {
        return vec![crate::ui::empty::empty(
            crate::ui::empty::EmptyLayout::Compact,
            crate::ui::Tone::Neutral,
            IconName::Calendar.path(),
            "No frames on this day",
            "Mnema wasn’t recording.",
            Vec::new(),
            p,
        )
        .into_any_element()];
    }
    let max = counts.iter().copied().max().unwrap_or(1).max(1);
    let here = v
        .paging
        .active_frame()
        .map(|f| local(f.captured_at_ms, o))
        .filter(|t| t.date() == day)
        .map(|t| t.hour());
    let busy = v.jumper.jumping || v.paging.loading;
    (0..=last)
        .map(|h| {
            let n = counts[h as usize];
            let off = n == 0;
            let label = clock_label(
                OffsetDateTime::UNIX_EPOCH
                    .replace_time(Time::from_hms(h, 0, 0).unwrap_or(Time::MIDNIGHT)),
                false,
            );
            let hover = (p.wash_strong, p.text_strong);
            div()
                .id(nid("jumper-hour", u64::from(h)))
                .test_support()
                .role(Role::Button)
                .relative()
                .flex()
                .items_center()
                .justify_between()
                .h(px(28.))
                .pl(px(14.))
                .pr(px(10.))
                .rounded(radius::SM)
                .overflow_hidden()
                .font_family(FONT_MONO)
                .text_size(text::SM)
                .font_weight(theme::W_MEDIUM)
                .text_color(if off { p.text_faint } else { p.text })
                .when(here == Some(h) && !off, |d| {
                    d.bg(p.wash_strong).text_color(p.text_strong)
                })
                .when(!off && !busy, |d| {
                    d.cursor_pointer()
                        .hover(move |s| s.bg(hover.0).text_color(hover.1))
                        .on_click(cx.listener(move |this, _, _, cx| this.commit(Some(h), cx)))
                })
                .when(n > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(4.))
                            .bottom(px(4.))
                            .w(gpui_kit::relative(n as f32 / max as f32))
                            .rounded(radius::XS)
                            .bg(crate::ui::alpha(p.chart_grey_3, 0.22)),
                    )
                })
                .child(div().relative().child(label))
                .child(
                    div()
                        .relative()
                        .text_size(text::XS)
                        .text_color(p.text_subtle)
                        .child(if off { "·".to_string() } else { n.to_string() }),
                )
                .into_any_element()
        })
        .collect()
}

/// The popover, anchored under the trigger.
pub fn render(v: &TimelineView, p: &Palette, cx: &mut Context<TimelineView>) -> Option<AnyElement> {
    if !v.jumper.open {
        return None;
    }
    let (view, picked) = (v.jumper.view?, v.jumper.picked?);
    let today = crate::ui::clock::now(cx).date();
    let o = offset(cx);
    let times = v.jumper.times.clone().unwrap_or_default();
    let mut per_day = std::collections::HashMap::<Date, usize>::new();
    for t in &times {
        *per_day.entry(local(*t, o).date()).or_default() += 1;
    }
    let month_max = per_day.values().copied().max().unwrap_or(0);
    let density =
        Rc::new(move |d: Date| day_heat(per_day.get(&d).copied().unwrap_or(0), month_max));
    // The committed moment's day keeps a soft tint while another day previews (`data-here`).
    let here = v
        .paging
        .active_frame()
        .map(|f| local(f.captured_at_ms, o).date());
    let entity = cx.entity();
    let (pick, nav, keys) = (entity.clone(), entity.clone(), entity.clone());
    let cal = calendar(CalSpec {
        id: "jumper-cal".into(),
        view,
        today,
        min: Date::MIN,
        max: today,
        body: Body::Days,
        sel: Some(picked),
        range: None,
        here,
        density,
        foot: Foot::Legend,
        grid: Some(GridKeys {
            focus: v.jumper_grid.clone(),
            cursor: v.cursor(today),
            on_key: Rc::new(move |e, _, cx| {
                if e.keystroke.modifiers.modified() {
                    return;
                }
                let key = e.keystroke.key.clone();
                if keys.update(cx, |this, cx| this.jumper_key(&key, cx)) {
                    cx.stop_propagation();
                }
            }),
        }),
        on_pick: Rc::new(move |d, _, cx| pick.update(cx, |this, cx| this.preview_day(d, cx))),
        on_nav: Rc::new(move |k, _, cx| {
            nav.update(cx, |this, cx| {
                let (first, _) = month_of(this.jumper.view.unwrap_or(today), k);
                this.view_month(first, cx);
            })
        }),
    });
    let foot: AnyElement = match &v.jumper.error {
        Some(e) => crate::ui::notice::inline(
            crate::ui::Tone::Danger,
            Some(icon(IconName::CircleAlert.path(), size::ICON_SM).into_any_element()),
            vec![SharedString::from(e.clone()).into_any_element()],
            p,
        )
        .into_any_element(),
        None => div()
            .child("Pick a day to preview its hours, then an hour to jump")
            .into_any_element(),
    };
    let busy = v.jumper.jumping || v.paging.loading;
    let hours = div()
        .flex()
        .flex_col()
        .w(px(212.))
        .min_h_0()
        .border_l_1()
        .border_color(p.hairline)
        .child(
            div()
                .flex()
                .justify_between()
                .px(space::S3)
                .pt(space::S3)
                .pb(space::S2)
                .text_size(text::BASE)
                .font_weight(theme::W_SEMIBOLD)
                .text_color(p.text_strong)
                .child(day_label(picked.with_time(Time::MIDNIGHT).assume_offset(o))),
        )
        .child(crate::ui::scroll::area(
            div()
                .id("jumper-hours")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(6.))
                .children(hour_rows(v, picked, p, cx)),
        ))
        .child(
            div()
                .px(space::S3)
                .py(space::S2)
                .border_t_1()
                .border_color(p.hairline)
                .child(
                    Button::new("jumper-day-latest", "latest of day")
                        .ghost()
                        .sm()
                        .icon(IconName::ArrowRightToLine.path())
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.commit(None, cx))),
                ),
        );
    let pop = popover::surface(p)
        .id("jumper-pop")
        .key_context(super::JUMPER_CONTEXT)
        .role(Role::Dialog)
        .aria_label("Jump to date and time")
        .absolute()
        .top(space::S3 + size::CMD_H + size::POP_GAP)
        .left(space::S4)
        .h(px(420.))
        .overflow_hidden()
        .on_mouse_down_out(cx.listener(|this, e, _, cx| {
            // The trigger toggles it (`TimelineJumper.svelte` skips it too).
            if this.jumper_at.outside(e, cx) {
                this.jumper.open = false;
                cx.notify();
            }
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(space::S3)
                .pt(space::S3)
                .pb(space::S2)
                .pl(space::S4)
                .pr(space::S3)
                .child(
                    div()
                        .text_size(text::MD)
                        .font_weight(theme::W_SEMIBOLD)
                        .text_color(p.text_strong)
                        .child("Jump to date & time"),
                )
                .child(latest_button("jumper-latest", v, cx)),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .border_y_1()
                .border_color(p.hairline)
                .child(div().p(px(4.)).child(cal))
                .child(hours),
        )
        .child(
            div()
                .px(space::S4)
                .pt(space::S2)
                .pb(space::S3)
                .text_size(text::SM)
                .text_color(p.text_muted)
                .child(foot),
        );
    Some(
        popover::float(popover::pop_in(
            pop,
            space::S3 + size::CMD_H + size::POP_GAP,
            false,
        ))
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_is_relative_to_the_busiest_day() {
        assert_eq!(day_heat(0, 10), 0);
        assert_eq!(day_heat(1, 10), 1);
        assert_eq!(day_heat(4, 10), 2);
        assert_eq!(day_heat(10, 10), 3);
    }
}
