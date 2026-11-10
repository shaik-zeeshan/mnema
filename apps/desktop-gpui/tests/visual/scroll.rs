//! `ui::scroll::bar` on real pixels (`-- scroll-bar`): no bar after a
//! programmatic scroll; after the pointer wheels the area, a 6 px thumb whose
//! length and place follow the scroll; never a bar on content that fits.

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, point, px, size, AnyWindowHandle, AppContext, Context, Modifiers, MouseMoveEvent, Render,
    ScrollDelta, ScrollEvent, ScrollHandle, VisualTestAppContext, Window,
};
use mnema_gpui::theme;
use mnema_gpui::ui::scroll;

struct Area {
    handle: ScrollHandle,
    rows: usize,
}

impl Render for Area {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        div().size_full().bg(p.bg).child(
            div()
                .id("area")
                .w(px(200.))
                .h(px(200.))
                .overflow_y_scroll()
                .track_scroll(&self.handle)
                .children((0..self.rows).map(|_| div().flex_none().h(px(40.))))
                .child(scroll::bar(&self.handle)),
        )
    }
}

fn settle(cx: &mut VisualTestAppContext, w: AnyWindowHandle) -> anyhow::Result<()> {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(w, |_, window, _| window.refresh())?;
    }
    cx.run_until_parked();
    Ok(())
}

/// The thumb's painted rows (logical px from the area's top) in the column
/// 6 px in from the area's right edge, at 2x: whatever isn't the page colour.
fn thumb_rows(
    cx: &mut VisualTestAppContext,
    w: AnyWindowHandle,
    h: &ScrollHandle,
) -> anyhow::Result<Option<(f32, f32)>> {
    let shot = cx.capture_screenshot(w)?;
    let b = h.bounds();
    let scale = shot.width() as f32 / 300.;
    let x = ((f32::from(b.right()) - 6.) * scale) as u32;
    let bg = *shot.get_pixel(((f32::from(b.left()) + 20.) * scale) as u32, 10);
    let top = f32::from(b.top());
    let hit: Vec<u32> = (0..(200. * scale) as u32)
        .filter(|y| {
            let px = shot.get_pixel(x, (top * scale) as u32 + y);
            (0..3).any(|c| (px[c] as i32 - bg[c] as i32).abs() > 12)
        })
        .collect();
    Ok(hit
        .first()
        .zip(hit.last())
        .map(|(a, z)| (*a as f32 / scale, (*z + 1) as f32 / scale)))
}

fn case(
    cx: &mut VisualTestAppContext,
    rows: usize,
) -> anyhow::Result<(AnyWindowHandle, ScrollHandle)> {
    let handle = ScrollHandle::new();
    let h = handle.clone();
    let window = cx.open_offscreen_window(size(px(300.), px(300.)), move |window, cx| {
        let v = cx.new(|_| Area { handle: h, rows });
        cx.new(|cx| Root::new(v, window, cx))
    })?;
    let w: AnyWindowHandle = window.into();
    settle(cx, w)?;
    Ok((w, handle))
}

fn wheel(
    cx: &mut VisualTestAppContext,
    w: AnyWindowHandle,
    h: &ScrollHandle,
    dy: f32,
) -> anyhow::Result<()> {
    let b = h.bounds();
    let at = point(b.left() + px(100.), b.top() + px(100.));
    cx.simulate_event(
        w,
        MouseMoveEvent {
            position: at,
            modifiers: Modifiers::default(),
            pressed_button: None,
        },
    );
    settle(cx, w)?;
    cx.simulate_event(
        w,
        ScrollEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        },
    );
    settle(cx, w)
}

pub fn checks(cx: &mut VisualTestAppContext, filters: &[String]) -> anyhow::Result<Vec<String>> {
    if !filters.is_empty() && !filters.iter().any(|f| "scroll-bar".contains(f.as_str())) {
        return Ok(Vec::new());
    }
    cx.update(|cx| theme::set_setting(AppearanceSetting::Dark, cx));
    let mut failures = Vec::new();
    let mut check = |ok: bool, what: String| {
        if !ok {
            failures.push(format!("scroll-bar: {what}"));
        }
    };

    // 10 x 40 px rows in a 200 px area: the thumb is half the track.
    let (w, h) = case(cx, 10)?;
    h.set_offset(point(px(0.), px(-100.)));
    settle(cx, w)?;
    let rows = thumb_rows(cx, w, &h)?;
    check(
        rows.is_none(),
        format!("a programmatic scroll showed a bar: {rows:?}"),
    );

    wheel(cx, w, &h, -20.)?;
    let off = -f32::from(h.offset().y);
    // `ThumbGeometry`: 100 px logical, 3 px inset each end, travel 100 px.
    let want = (3. + off / 200. * 100., 97. + off / 200. * 100.);
    let rows = thumb_rows(cx, w, &h)?;
    check(
        rows.is_some_and(|(a, z)| (a - want.0).abs() <= 1.5 && (z - want.1).abs() <= 1.5),
        format!("at offset {off}: thumb {rows:?}, want {want:?}"),
    );
    cx.update_window(w, |_, window, _| window.remove_window())?;

    // 4 rows fit: wheeling never shows a bar.
    let (w, h) = case(cx, 4)?;
    wheel(cx, w, &h, -40.)?;
    wheel(cx, w, &h, -40.)?;
    let rows = thumb_rows(cx, w, &h)?;
    check(
        rows.is_none(),
        format!("a bar on content that fits: {rows:?}"),
    );
    cx.update_window(w, |_, window, _| window.remove_window())?;
    Ok(failures)
}
