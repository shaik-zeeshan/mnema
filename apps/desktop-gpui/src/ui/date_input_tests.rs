//! DateInput driven on the real control: the week-rows and month-grid bodies,
//! the popover's unit switch, range picking by press and by drag, the caps of
//! an in-range run, and the capsule's keys, wheel and drag scrub.

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    div, point, px, size, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement,
    Render, ScrollDelta, Styled, TestAppContext, Window, WindowAppearance, WindowBounds,
    WindowHandle, WindowOptions,
};
use time::macros::date;
use time::Date;

use super::calendar::Unit;
use super::date_input::DateInput;
use super::nid;
use crate::theme;

const TODAY: Date = date!(2026 - 11 - 04); // a Wednesday

/// Holds the value like a host screen does and logs every change.
struct Host {
    value: Date,
    unit: Option<Unit>,
    /// `Some` = range mode (`Some(None)` = all time).
    span: Option<Option<(Date, Date)>>,
    log: Vec<(Date, Date, Option<Unit>)>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let mut d = DateInput::new("dt", self.value, TODAY).range(date!(2025 - 06 - 10), TODAY);
        if let Some(u) = self.unit {
            d = d.period(u);
        }
        d = match self.span {
            Some(span) => {
                let this = this.clone();
                d.span(span, move |next, _, cx| {
                    this.update(cx, |h, cx| {
                        h.span = Some(next);
                        if let Some((s, e)) = next {
                            h.log.push((s, e, None));
                        }
                        cx.notify();
                    })
                    .ok();
                })
            }
            None => d.on_change(move |s, e, u, _, cx| {
                this.update(cx, |h, cx| {
                    h.value = s;
                    h.unit = u;
                    h.log.push((s, e, u));
                    cx.notify();
                })
                .ok();
            }),
        };
        div().size_full().p(px(40.)).child(d)
    }
}

fn open(
    cx: &mut TestAppContext,
    value: Date,
    unit: Option<Unit>,
    span: Option<Option<(Date, Date)>>,
) -> (WindowHandle<Root>, Entity<Host>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(800.), px(700.)),
            })),
            ..Default::default()
        };
        let (window, host) = gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|_| Host {
                value,
                unit,
                span,
                log: Vec::new(),
            })
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), host)
    })
}

fn act(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    f: impl FnOnce(&mut Window, &mut gpui_kit::App),
) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn has(cx: &mut TestAppContext, w: WindowHandle<Root>, id: gpui_kit::ElementId) -> bool {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn id(kind: &str, d: Date) -> gpui_kit::ElementId {
    nid(&format!("dt-cal-{kind}"), d.to_julian_day() as u64)
}

fn pop(cx: &mut TestAppContext, w: WindowHandle<Root>) -> bool {
    has(cx, w, nid("dt-cal-nav", 0)) // the popover's ‹ button
}

fn log(cx: &mut TestAppContext, h: &Entity<Host>) -> Vec<(Date, Date, Option<Unit>)> {
    h.read_with(cx, |h, _| h.log.clone())
}

#[gpui_kit::test]
fn week_mode_draws_one_button_per_week_row_and_a_row_commits_its_week(cx: &mut TestAppContext) {
    let (w, h) = open(cx, TODAY, Some(Unit::Week), None);
    act(cx, w, |win, cx| win.click("dt", cx));
    assert!(pop(cx, w));
    // Week rows, not a run of selected day cells (the founder's square block).
    assert!(
        has(cx, w, id("week", date!(2026 - 11 - 02))),
        "this week's row"
    );
    assert!(!has(cx, w, id("day", TODAY)), "no day cells in week mode");
    // A week entirely after today can't be picked.
    act(cx, w, |win, cx| {
        win.click(id("week", date!(2026 - 11 - 09)), cx)
    });
    assert!(log(cx, &h).is_empty(), "a future week is disabled");
    act(cx, w, |win, cx| {
        win.click(id("week", date!(2026 - 10 - 26)), cx)
    });
    assert_eq!(
        log(cx, &h),
        vec![(
            date!(2026 - 10 - 26),
            date!(2026 - 11 - 01),
            Some(Unit::Week)
        )]
    );
    assert!(!pop(cx, w), "a pick closes the popover");
}

#[gpui_kit::test]
fn month_mode_draws_the_year_and_navigates_by_year(cx: &mut TestAppContext) {
    let (w, h) = open(cx, TODAY, Some(Unit::Month), None);
    act(cx, w, |win, cx| win.click("dt", cx));
    assert!(has(cx, w, id("month", date!(2026 - 01 - 01))));
    assert!(!has(cx, w, id("day", TODAY)));
    // December is still ahead: disabled. Next year is past max: its button too.
    act(cx, w, |win, cx| {
        win.click(id("month", date!(2026 - 12 - 01)), cx)
    });
    assert!(log(cx, &h).is_empty());
    let label = |cx: &mut TestAppContext| {
        cx.update_window(w.into(), |_, win, cx| {
            win.render_frame(cx);
            win.find(nid("dt-cal-nav", 0)).label().map(str::to_string)
        })
        .unwrap()
    };
    assert_eq!(label(cx).as_deref(), Some("Previous year"));
    act(cx, w, |win, cx| win.click(nid("dt-cal-nav", 0), cx));
    assert!(
        has(cx, w, id("month", date!(2025 - 07 - 01))),
        "2025 on screen"
    );
    // Before the first capture (Jun 10 2025): May 2025 has no day in range.
    act(cx, w, |win, cx| {
        win.click(id("month", date!(2025 - 05 - 01)), cx)
    });
    assert!(log(cx, &h).is_empty());
    act(cx, w, |win, cx| {
        win.click(id("month", date!(2025 - 07 - 01)), cx)
    });
    assert_eq!(
        log(cx, &h),
        vec![(
            date!(2025 - 07 - 01),
            date!(2025 - 07 - 31),
            Some(Unit::Month)
        )]
    );
    assert!(!pop(cx, w));
}

#[gpui_kit::test]
fn the_popover_unit_switch_changes_the_body_and_stays_open(cx: &mut TestAppContext) {
    let (w, h) = open(cx, TODAY, Some(Unit::Week), None);
    act(cx, w, |win, cx| win.click("dt", cx));
    act(cx, w, |win, cx| win.click(nid("dt-pop-unit", 2), cx));
    assert_eq!(
        log(cx, &h),
        vec![(
            date!(2026 - 11 - 01),
            date!(2026 - 11 - 30),
            Some(Unit::Month)
        )],
        "this week showed, so Month lands on this month"
    );
    assert!(pop(cx, w), "switching units keeps the popover open");
    assert!(has(cx, w, id("month", date!(2026 - 11 - 01))));
    // Day mode: the Today reset is disabled on today; picking a day commits it.
    act(cx, w, |win, cx| win.click(nid("dt-pop-unit", 0), cx));
    assert!(has(cx, w, id("day", TODAY)));
    act(cx, w, |win, cx| {
        win.click(id("day", date!(2026 - 11 - 02)), cx)
    });
    assert_eq!(
        log(cx, &h).last(),
        Some(&(
            date!(2026 - 11 - 02),
            date!(2026 - 11 - 02),
            Some(Unit::Day)
        ))
    );
    assert!(!pop(cx, w));
}

#[gpui_kit::test]
fn range_mode_picks_with_two_presses_or_one_drag(cx: &mut TestAppContext) {
    let (w, h) = open(cx, TODAY, None, Some(None));
    act(cx, w, |win, cx| win.click("dt", cx));
    // A press-and-release marks the anchor only; the next press commits (sorted).
    act(cx, w, |win, cx| {
        win.click(id("day", date!(2026 - 11 - 03)), cx)
    });
    assert!(log(cx, &h).is_empty() && pop(cx, w));
    act(cx, w, |win, cx| {
        win.click(id("day", date!(2026 - 10 - 28)), cx)
    });
    assert_eq!(
        log(cx, &h),
        vec![(date!(2026 - 10 - 28), date!(2026 - 11 - 03), None)]
    );
    assert!(!pop(cx, w));
    // A drag across days commits on release.
    act(cx, w, |win, cx| win.click("dt", cx));
    act(cx, w, |win, cx| {
        win.drag_to(
            id("day", date!(2026 - 10 - 20)),
            id("day", date!(2026 - 10 - 13)),
            cx,
        )
    });
    assert_eq!(
        log(cx, &h).last(),
        Some(&(date!(2026 - 10 - 13), date!(2026 - 10 - 20), None))
    );
    assert!(!pop(cx, w));
}

#[gpui_kit::test]
fn capsule_keys_step_and_open(cx: &mut TestAppContext) {
    let (w, h) = open(cx, date!(2026 - 10 - 20), None, None);
    act(cx, w, |win, cx| win.click("dt", cx)); // focus (and open)
    act(cx, w, |win, cx| win.press("escape", cx));
    assert!(!pop(cx, w), "Esc closes");
    act(cx, w, |win, cx| win.press("up", cx));
    act(cx, w, |win, cx| win.press("right", cx));
    act(cx, w, |win, cx| win.press("down", cx));
    let days: Vec<Date> = log(cx, &h).iter().map(|l| l.0).collect();
    assert_eq!(
        days,
        vec![
            date!(2026 - 10 - 21),
            date!(2026 - 10 - 22),
            date!(2026 - 10 - 21)
        ]
    );
    act(cx, w, |win, cx| win.press("end", cx));
    assert_eq!(log(cx, &h).last().map(|l| l.0), Some(TODAY));
    act(cx, w, |win, cx| win.press("enter", cx));
    assert!(pop(cx, w), "Enter opens the calendar");
}

#[gpui_kit::test]
fn range_keys_step_a_custom_span_by_its_length(cx: &mut TestAppContext) {
    let span = (date!(2026 - 10 - 20), date!(2026 - 10 - 29)); // 10 days
    let (w, h) = open(cx, TODAY, None, Some(Some(span)));
    act(cx, w, |win, cx| win.click("dt", cx));
    act(cx, w, |win, cx| win.press("escape", cx));
    act(cx, w, |win, cx| win.press("left", cx));
    assert_eq!(
        log(cx, &h),
        vec![(date!(2026 - 10 - 10), date!(2026 - 10 - 19), None)]
    );
    act(cx, w, |win, cx| win.press("home", cx));
    assert_eq!(
        log(cx, &h).last(),
        Some(&(date!(2026 - 10 - 26), TODAY, None)),
        "latest keeps the length"
    );
}

#[gpui_kit::test]
fn the_wheel_steps_once_per_40px(cx: &mut TestAppContext) {
    let (w, h) = open(cx, date!(2026 - 10 - 20), None, None);
    let wheel = |cx: &mut TestAppContext, dy: f32| {
        act(cx, w, |win, cx| {
            win.scroll("dt", ScrollDelta::Pixels(point(px(0.), px(dy))), cx)
        })
    };
    wheel(cx, -39.);
    assert!(log(cx, &h).is_empty(), "39 px: no step");
    wheel(cx, -1.);
    // GPUI's sign is the content's motion: a negative delta is a browser's +deltaY = later.
    assert_eq!(log(cx, &h).len(), 1, "40 px: one step");
    assert_eq!(log(cx, &h)[0].0, date!(2026 - 10 - 21));
}

#[gpui_kit::test]
fn dragging_the_capsule_scrubs_and_is_not_a_click(cx: &mut TestAppContext) {
    let (w, h) = open(cx, TODAY, Some(Unit::Week), None);
    let at = cx
        .update_window(w.into(), |_, win, cx| {
            win.render_frame(cx);
            win.find("dt").bounds().center()
        })
        .unwrap();
    // Right 50 px = two weeks earlier (24 px a week).
    act(cx, w, |win, cx| {
        win.drag(at, point(at.x + px(50.), at.y), cx)
    });
    assert_eq!(
        log(cx, &h).last(),
        Some(&(
            date!(2026 - 10 - 19),
            date!(2026 - 10 - 25),
            Some(Unit::Week)
        ))
    );
    assert!(!pop(cx, w), "a drag doesn't open the calendar");
}

fn tape_x(cx: &mut TestAppContext, w: WindowHandle<Root>) -> f32 {
    cx.update_window(w.into(), |_, win, cx| {
        win.render_frame(cx);
        f32::from(win.find(nid("dt-tape", 0)).bounds().origin.x)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_density_strip_slides_with_expo_or_snaps_under_reduce_motion(cx: &mut TestAppContext) {
    use crate::theme::motion;
    let (w, _) = open(cx, date!(2026 - 10 - 20), None, None);
    act(cx, w, |win, cx| win.click("dt", cx));
    act(cx, w, |win, cx| win.press("escape", cx));
    let x0 = tape_x(cx, w);
    // A day later: the tape ends one 7 px tick to the left.
    act(cx, w, |win, cx| win.press("right", cx));
    assert!((tape_x(cx, w) - x0).abs() < 0.1, "starts where it was");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(22));
    let mid = tape_x(cx, w);
    assert!(
        mid < x0 - 0.1 && mid > x0 - 6.9,
        "mid-slide at 22 ms: {mid} from {x0}"
    );
    cx.executor().advance_clock(motion::MED);
    assert!(
        (tape_x(cx, w) - (x0 - 7.)).abs() < 0.1,
        "settled after 220 ms"
    );
    cx.update(|cx| cx.set_reduce_motion(true));
    act(cx, w, |win, cx| win.press("right", cx));
    assert!(
        (tape_x(cx, w) - (x0 - 14.)).abs() < 0.1,
        "Reduce Motion snaps"
    );
}
