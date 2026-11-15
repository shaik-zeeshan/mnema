//! The status bar at narrow widths: what gives way, and that nothing overlaps.

use capture_types::AppearanceSetting;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, AppContext as _, Bounds, Point, TestAppContext, Window, WindowAppearance,
    WindowBounds, WindowOptions,
};

use crate::keymap::{self, Bindings};
use crate::shell::{fixture, model, Shell, ShellModel};
use crate::theme;

/// The populated recording bar plus the engine pill and an idle lane: the real
/// app's row that ran out of room at 640 px.
fn full_bar_model() -> ShellModel {
    let mut m = fixture::shell_model(&fixture::load("shell", "populated").unwrap()).unwrap();
    let empty = fixture::shell_model(&fixture::load("shell", "empty").unwrap()).unwrap();
    m.status.engine = empty.status.engine;
    if let Some(rt) = m.status.runtime.as_mut() {
        rt[2].paused = true;
    }
    m
}

/// Which of `model`'s readouts show whole at `width`; panics if one shows only
/// in part, or if the `?` or a flag runs out of the bar.
fn readouts_at(cx: &mut TestAppContext, model: ShellModel, width: f32) -> Vec<usize> {
    let mut shown = Vec::new();
    readouts_at_then(cx, model, width, |window| {
        let help = window.find("status-shortcuts").bounds();
        let row = window.find("status-readouts").bounds();
        for i in 0..3usize {
            let Some(el) = window.try_find(("status-readout", i)) else {
                break;
            };
            let b = el.bounds();
            if b.top() >= row.bottom() {
                continue; // wrapped onto the clipped line: gone whole
            }
            assert!(
                b.left() >= row.left() && b.right() <= row.right() && b.right() <= help.left(),
                "readout {i} is cut at {width}: {b:?} in {row:?}"
            );
            shown.push(i);
        }
    });
    shown
}

/// Renders `model`'s bar at `width`, checks nothing in it overlaps or leaves the
/// bar, then hands the window to `read`.
fn readouts_at_then<R>(
    cx: &mut TestAppContext,
    model: ShellModel,
    width: f32,
    read: impl FnOnce(&mut Window) -> R,
) -> R {
    let (window, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(480.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(model, window, cx))
        })
        .expect("window")
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let help = window.find("status-shortcuts").bounds();
        assert!(help.right() <= px(width), "the ? stays inside at {width}");
        // Left to right: flag, engine pill, `?`, none under the next.
        let mut edge = help.left();
        for id in ["status-engine", "status-flag"] {
            if let Some(el) = window.try_find(id) {
                let b = el.bounds();
                assert!(
                    b.right() <= edge,
                    "{id} runs under its neighbour at {width}: {b:?}"
                );
                edge = b.left();
            }
        }
        read(window)
    })
    .unwrap()
}

#[gpui_kit::test]
fn status_readouts_drop_out_whole_never_cut(cx: &mut TestAppContext) {
    assert_eq!(
        readouts_at(cx, full_bar_model(), 640.),
        [0usize; 0],
        "640: 'today' doesn't fit"
    );
    // Storage and "local only" go below 900 px.
    assert_eq!(
        readouts_at(cx, full_bar_model(), 800.),
        [0],
        "800: today only"
    );
    assert_eq!(
        readouts_at(cx, full_bar_model(), 1280.),
        [0, 1, 2],
        "wide: today, storage, local only"
    );
    // The longest flag at the window's minimum width: its text gives way, not
    // the `?` or its own action.
    let mut flagged = full_bar_model();
    flagged.status.running = true;
    flagged.status.engine = None;
    let rt = flagged.status.runtime.get_or_insert_with(Default::default);
    for i in [0, 2] {
        rt[i].requested = true;
        rt[i].privacy_failed = true;
        rt[i].restart_required = true;
    }
    readouts_at(cx, flagged.clone(), crate::window::MAIN_MIN.0);
    // With the engine pill up too, the lanes drop their "· stopped" and the pill
    // shrinks to its dot; at the minimum width it goes, for the flag's action.
    flagged.status.engine = Some(model::EngineFlag {
        label: "needs reconnect",
        error: true,
        tip: "Sign in again".into(),
    });
    let widths = |cx: &mut TestAppContext, m: &ShellModel, width| {
        readouts_at_then(cx, m.clone(), width, |window| {
            ["status-flag", "status-engine"]
                .map(|id| window.try_find(id).map(|e| e.bounds().size.width))
        })
    };
    let [flag, engine] = widths(cx, &flagged, 800.);
    assert!(
        flag >= Some(px(120.)) && engine >= Some(px(20.)),
        "{flag:?} {engine:?}"
    );
    let [_, engine] = widths(cx, &flagged, crate::window::MAIN_MIN.0);
    assert_eq!(engine, None, "640: no engine dot beside a flag");

    // Every flag at the minimum width shows its action whole: the pill is 29 px
    // around 6 px letters. No flag: the engine pill shows whole beside idle lanes.
    let stat = |running: bool, edit: fn(&mut model::StatusBarModel)| {
        let mut m = flagged.clone();
        let s = &mut m.status;
        (s.running, s.selected) = (running, [true; 3]);
        s.runtime = Some(Default::default()).filter(|_| running);
        edit(s);
        m
    };
    let flags: [(bool, fn(&mut model::StatusBarModel)); 6] = [
        (true, |s| s.low_disk = true),
        (false, |s| {
            s.screen_permission = Some(capture_types::CapturePermissionState::Denied)
        }),
        (false, |s| s.stopped_reason = Some("disk full".into())),
        (false, |s| s.start_error = Some("boom".into())),
        (false, |s| s.read_only = true),
        (false, |s| s.waiting_for_display = true),
    ];
    for (running, edit) in flags {
        let m = stat(running, edit);
        let action = model::status_flag(&m.status).expect("a flag").action;
        let [flag, _] = widths(cx, &m, crate::window::MAIN_MIN.0);
        let whole = px(29. + 6. * action.chars().count() as f32);
        assert!(flag >= Some(whole), "\"{action}\" is cut at 640: {flag:?}");
    }
    let idle = stat(true, |s| {
        let rt = s.runtime.as_mut().unwrap();
        *rt = rt.map(|r| model::RuntimeSource {
            requested: true,
            paused: true,
            ..r
        });
        rt[0].paused = false;
    });
    let at = |cx: &mut TestAppContext, width| widths(cx, &idle, width)[1];
    assert_eq!(
        at(cx, crate::window::MAIN_MIN.0),
        at(cx, 1280.),
        "engine pill cut"
    );
}
