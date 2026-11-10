//! Hover on real pixels (`-- hover`): a pointer moved over an unselected
//! `ui::choice` option paints its hover state, list rows (`--mx-wash` fill) and
//! tiles (`--app-border-hover` edge) alike. The gallery's goldens pin the hover
//! look with `Force::Hover`; this pins that a real pointer reaches it.
//! `-- pointer`: the real system pointer over a golden window never paints
//! hover (see `pointer.rs`), while a simulated move still does.

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
    let window = crate::pointer::open(cx, size(px(600.), px(400.)), move |window, cx| {
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
    let window = crate::pointer::open(cx, size(px(1280.), px(800.)), |window, cx| {
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

/// Opens the list pick; `raw` skips `pointer::open`'s park (what goldens did before).
fn open_pick(cx: &mut VisualTestAppContext, raw: bool) -> anyhow::Result<AnyWindowHandle> {
    let build = |window: &mut Window, cx: &mut gpui_kit::App| {
        let v = cx.new(|_| Pick { list: true });
        cx.new(|cx| Root::new(v, window, cx))
    };
    let s = size(px(600.), px(400.));
    let window = if raw {
        cx.open_offscreen_window(s, build)?
    } else {
        crate::pointer::open(cx, s, build)?
    };
    Ok(window.into())
}

fn mouse(cx: &mut VisualTestAppContext, w: AnyWindowHandle) -> anyhow::Result<Point<Pixels>> {
    Ok(cx.update_window(w, |_, window, _| window.mouse_position())?)
}

/// Is option 0 hovered? Its row fill against option 2's (also unselected).
fn row0_hovered(cx: &mut VisualTestAppContext, w: AnyWindowHandle) -> anyhow::Result<bool> {
    settle(cx, w)?;
    let fill = |b: Bounds<Pixels>| point(b.right() - px(30.), b.top() + px(6.));
    let (b0, b2) = (bounds_of(cx, w, 0)?, bounds_of(cx, w, 2)?);
    let shot = cx.capture_screenshot(w)?;
    Ok(pixel(&shot, fill(b0)) != pixel(&shot, fill(b2)))
}

/// Puts the REAL pointer over option 0 of where a test window lands, opens one
/// window raw and one through `pointer::open`, and puts the pointer straight
/// back. The raw one must show the hover (else the check proves nothing), the
/// golden one must not, and a simulated move must still hover it.
fn real_pointer_case(cx: &mut VisualTestAppContext) -> anyhow::Result<Vec<String>> {
    let home = crate::pointer::system::location();
    let mut placed = None;
    for _ in 0..3 {
        // Where the window lands: its pointer is the real one, in its own points.
        let probe = open_pick(cx, true)?;
        settle(cx, probe)?;
        let (seen, b) = (mouse(cx, probe)?, bounds_of(cx, probe, 0)?);
        cx.update_window(probe, |_, window, _| window.remove_window())?;
        let home_now = crate::pointer::system::location();
        crate::pointer::system::warp(home_now + (b.center() - seen));
        let opened = (open_pick(cx, true), open_pick(cx, false));
        crate::pointer::system::warp(home_now);
        let (raw, golden) = (opened.0?, opened.1?);
        if b.contains(&mouse(cx, raw)?) {
            placed = Some((raw, golden));
            break;
        }
        // Someone moved the pointer in between: try again.
        for w in [raw, golden] {
            cx.update_window(w, |_, window, _| window.remove_window())?;
        }
    }
    let mut failures = Vec::new();
    let Some((raw, golden)) = placed else {
        let back = crate::pointer::system::location();
        failures.push(format!(
            "pointer: could not put the real pointer over a test window (now {back:?}, was {home:?})"
        ));
        return Ok(failures);
    };
    if !row0_hovered(cx, raw)? {
        failures.push("pointer: a raw window under the real pointer shows no hover, so this check proves nothing".into());
    }
    if row0_hovered(cx, golden)? {
        failures.push("pointer: the real pointer painted hover into a golden window".into());
    }
    let at = bounds_of(cx, golden, 0)?.center();
    cx.simulate_event(
        golden,
        MouseMoveEvent {
            position: at,
            modifiers: Modifiers::default(),
            pressed_button: None,
        },
    );
    if !row0_hovered(cx, golden)? {
        failures.push("pointer: a simulated move no longer hovers a golden window".into());
    }
    for w in [raw, golden] {
        cx.update_window(w, |_, window, _| window.remove_window())?;
    }
    println!(
        "  pointer: {}",
        if failures.is_empty() { "ok" } else { "FAILED" }
    );
    Ok(failures)
}

pub fn checks(cx: &mut VisualTestAppContext, filters: &[String]) -> anyhow::Result<Vec<String>> {
    let wants =
        |name: &str| filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()));
    cx.update(|cx| theme::set_setting(AppearanceSetting::Dark, cx));
    let mut failures = Vec::new();
    if wants("pointer") {
        failures.extend(real_pointer_case(cx)?);
    }
    if !wants("hover") {
        return Ok(failures);
    }
    let pointer_failures = failures.len();
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
        if failures.len() == pointer_failures {
            "ok"
        } else {
            "FAILED"
        }
    );
    Ok(failures)
}
