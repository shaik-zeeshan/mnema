//! Hover on real pixels (`-- hover`): a pointer moved over an unselected
//! `ui::choice` option paints its hover state, list rows (`--mx-wash` fill) and
//! tiles (`--app-border-hover` edge) alike. The gallery's goldens pin the hover
//! look with `Force::Hover`; this pins that a real pointer reaches it.

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, point, px, size, AnyWindowHandle, AppContext, Bounds, Context, Modifiers, MouseMoveEvent,
    Pixels, Point, Render, VisualTestAppContext, Window,
};
use mnema_gpui::theme;
use mnema_gpui::ui::choice::{Choice, ChoiceOption};

struct Pick {
    list: bool,
}

impl Render for Pick {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::tokens(cx);
        let options = ["Parakeet", "Local Whisper", "Apple Speech"]
            .map(|l| ChoiceOption::new(l).desc("A line under it."))
            .to_vec();
        let choice = Choice::new("pick", "Provider", options, 1).on_change(|_, _, _| {});
        div()
            .size_full()
            .bg(p.bg)
            .p(px(20.))
            .child(
                div()
                    .w(px(560.))
                    .child(if self.list { choice.list() } else { choice }),
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

fn bounds_of(
    cx: &mut VisualTestAppContext,
    w: AnyWindowHandle,
    ix: u64,
) -> anyhow::Result<Bounds<Pixels>> {
    let id = mnema_gpui::ui::nid("pick-opt", ix);
    Ok(cx.update_window(w, |_, window, _| {
        gpui_kit::test::TestWindowExt::find(window, id).bounds()
    })?)
}

fn pixel(shot: &image::RgbaImage, at: Point<Pixels>) -> [u8; 4] {
    let scale = shot.width() as f32 / 600.;
    shot.get_pixel(
        (f32::from(at.x) * scale) as u32,
        (f32::from(at.y) * scale) as u32,
    )
    .0
}

/// Option 0's pixel at `probe` (from its top-left) before and after the pointer
/// moves onto it.
fn hover_case(
    cx: &mut VisualTestAppContext,
    list: bool,
    probe: impl Fn(Bounds<Pixels>) -> Point<Pixels>,
) -> anyhow::Result<([u8; 4], [u8; 4])> {
    let window = cx.open_offscreen_window(size(px(600.), px(400.)), move |window, cx| {
        let v = cx.new(|_| Pick { list });
        cx.new(|cx| Root::new(v, window, cx))
    })?;
    let w: AnyWindowHandle = window.into();
    settle(cx, w)?;
    let b = bounds_of(cx, w, 0)?;
    let at = probe(b);
    let before = pixel(&cx.capture_screenshot(w)?, at);
    cx.simulate_event(
        w,
        MouseMoveEvent {
            position: b.center(),
            modifiers: Modifiers::default(),
            pressed_button: None,
        },
    );
    settle(cx, w)?;
    let after = pixel(&cx.capture_screenshot(w)?, at);
    cx.update_window(w, |_, window, _| window.remove_window())?;
    Ok((before, after))
}

/// The same list inside the real Settings page (Intelligence › Transcription),
/// where the founder's pointer found no hover: Parakeet's row (unselected).
fn settings_case(cx: &mut VisualTestAppContext) -> anyhow::Result<([u8; 4], [u8; 4])> {
    use mnema_gpui::settings::fixture::{settings_data, FixtureBackend};
    use mnema_gpui::shell::{Shell, ShellModel, Surface};
    let data = settings_data()?;
    let mut settings = None;
    let window = cx.open_offscreen_window(size(px(1280.), px(800.)), |window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = Shell::new(ShellModel::default(), window, cx);
            shell.go(Surface::Settings, cx);
            shell
        });
        let s = shell.read(cx).settings.clone();
        s.update(cx, |s, cx| {
            s.set_backend(std::sync::Arc::new(FixtureBackend::default()));
            s.set_data(data, cx)
        });
        settings = Some(s);
        cx.new(|cx| Root::new(shell, window, cx))
    })?;
    let w: AnyWindowHandle = window.into();
    let settings = settings.expect("settings");
    for step in 0..4 {
        cx.run_until_parked();
        if step == 1 {
            cx.update(|cx| settings.update(cx, |s, cx| s.open_tab("transcription", cx)));
        }
        cx.update_window(w, |_, window, _| window.refresh())?;
    }
    settle(cx, w)?;
    let row = |cx: &mut VisualTestAppContext| {
        cx.update_window(w, |_, window, _| {
            let id = mnema_gpui::ui::nid("tx-provider-opt", 0);
            gpui_kit::test::TestWindowExt::find(window, id).bounds()
        })
    };
    let b = row(cx)?;
    // The founder wheeled down to the section (that arms the overlay scroll
    // bar); a deep link alone doesn't.
    let away = point(b.left() + px(200.), b.top() - px(40.));
    cx.simulate_event(
        w,
        MouseMoveEvent {
            position: away,
            modifiers: Modifiers::default(),
            pressed_button: None,
        },
    );
    settle(cx, w)?;
    cx.simulate_event(
        w,
        gpui_kit::ScrollEvent {
            position: away,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-1.))),
            ..Default::default()
        },
    );
    settle(cx, w)?;
    let b = row(cx)?;
    let at = point(b.right() - px(30.), b.top() + px(6.));
    let read = |shot: &image::RgbaImage| {
        let k = shot.width() as f32 / 1280.;
        shot.get_pixel((f32::from(at.x) * k) as u32, (f32::from(at.y) * k) as u32)
            .0
    };
    let before = read(&cx.capture_screenshot(w)?);
    cx.simulate_event(
        w,
        MouseMoveEvent {
            position: b.center(),
            modifiers: Modifiers::default(),
            pressed_button: None,
        },
    );
    settle(cx, w)?;
    let after = read(&cx.capture_screenshot(w)?);
    cx.update_window(w, |_, window, _| window.remove_window())?;
    Ok((before, after))
}

pub fn checks(cx: &mut VisualTestAppContext, filters: &[String]) -> anyhow::Result<Vec<String>> {
    if !filters.is_empty() && !filters.iter().any(|f| "hover".contains(f.as_str())) {
        return Ok(Vec::new());
    }
    cx.update(|cx| theme::set_setting(AppearanceSetting::Dark, cx));
    let mut failures = Vec::new();
    // A list row's fill, well clear of its text.
    let (before, after) = hover_case(cx, true, |b| point(b.right() - px(30.), b.top() + px(6.)))?;
    if before == after {
        failures.push(format!(
            "hover: a list row shows no hover fill ({before:?})"
        ));
    }
    let (before, after) = settings_case(cx)?;
    if before == after {
        failures.push(format!(
            "hover: Settings' provider list shows no hover fill ({before:?})"
        ));
    }
    // A tile's edge, halfway down its left border.
    let (before, after) = hover_case(cx, false, |b| point(b.left() + px(0.5), b.top() + px(30.)))?;
    if before == after {
        failures.push(format!("hover: a tile shows no hover border ({before:?})"));
    }
    println!(
        "  hover: {}",
        if failures.is_empty() { "ok" } else { "FAILED" }
    );
    Ok(failures)
}
