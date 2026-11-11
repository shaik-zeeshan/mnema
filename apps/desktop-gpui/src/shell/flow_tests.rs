//! The status bar's controls, flags and keys in each capture state, and the
//! titlebar field, driven on the real view (no app core: nothing records).

use capture_types::AppearanceSetting;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, Entity, Point, TestAppContext, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use super::model::{RuntimeSource, StatusBarModel};
use super::*;

fn open(cx: &mut TestAppContext, status: StatusBarModel) -> (WindowHandle<Root>, Entity<Shell>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        keymap::bind(&Bindings::default(), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1280.), px(800.)),
            })),
            ..Default::default()
        };
        let model = ShellModel {
            status,
            ..Default::default()
        };
        let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::new(model, window, cx))
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), shell)
    })
}

/// `id`'s accessible name, or `None` when it isn't drawn.
fn label(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) -> Option<String> {
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id)
            .map(|e| e.label().unwrap_or_default().to_string())
    })
    .unwrap()
}

fn set_status(cx: &mut TestAppContext, shell: &Entity<Shell>, status: StatusBarModel) {
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            let mut model = s.model.clone();
            model.status = status;
            s.set_model(model, cx);
        })
    });
    cx.run_until_parked();
}

fn live(requested: bool) -> RuntimeSource {
    RuntimeSource {
        requested,
        live: requested,
        ..Default::default()
    }
}

#[gpui_kit::test]
fn pause_names_each_state_and_stop_waits_out_a_start(cx: &mut TestAppContext) {
    let running = StatusBarModel {
        running: true,
        runtime: Some([live(true), live(true), live(false)]),
        ..Default::default()
    };
    let (window, shell) = open(cx, running.clone());
    assert_eq!(
        label(cx, window, "status-pause").as_deref(),
        Some("Pause recording")
    );
    set_status(
        cx,
        &shell,
        StatusBarModel {
            inactivity_paused: true,
            ..running.clone()
        },
    );
    assert_eq!(
        label(cx, window, "status-pause").as_deref(),
        Some("Pause the whole session"),
        "an idle pause is still pausable as a whole"
    );
    set_status(
        cx,
        &shell,
        StatusBarModel {
            low_disk: true,
            ..running.clone()
        },
    );
    assert_eq!(
        label(cx, window, "status-pause").as_deref(),
        Some("Resumes on its own once space frees up")
    );
    set_status(
        cx,
        &shell,
        StatusBarModel {
            user_paused: true,
            low_disk: true,
            ..running.clone()
        },
    );
    assert_eq!(
        label(cx, window, "status-pause").as_deref(),
        Some("Resume recording")
    );
    // Starting: Stop shows (disabled) where it will be; no Record, no Pause.
    set_status(
        cx,
        &shell,
        StatusBarModel {
            loading_start: true,
            ..Default::default()
        },
    );
    assert!(label(cx, window, "status-stop").is_some());
    assert!(label(cx, window, "status-record").is_none());
    assert!(label(cx, window, "status-pause").is_none());
}

#[gpui_kit::test]
fn the_privacy_flag_restarts_or_only_reports(cx: &mut TestAppContext) {
    let mut failed = live(true);
    failed.privacy_failed = true;
    let mut status = StatusBarModel {
        running: true,
        runtime: Some([failed, live(true), live(false)]),
        ..Default::default()
    };
    let (window, shell) = open(cx, status.clone());
    assert_eq!(
        label(cx, window, "status-flag").as_deref(),
        Some("privacy filter failed · retrying")
    );
    status.runtime.as_mut().unwrap()[0].restart_required = true;
    set_status(cx, &shell, status);
    assert_eq!(
        label(cx, window, "status-flag").as_deref(),
        Some("restart to resume screen · Restart")
    );
}

#[gpui_kit::test]
fn source_keys_work_on_the_timeline_only(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, StatusBarModel::default());
    let selected = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).model.status.selected[1]);
    let press = |cx: &mut TestAppContext, keys: &str| {
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            w.press(keys, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    press(cx, "2");
    assert!(selected(cx), "2 toggles the mic on the Timeline");
    cx.update(|cx| shell.update(cx, |s, cx| s.go(Surface::Insights, cx)));
    press(cx, "2");
    assert!(selected(cx), "but not on Insights");
    assert!(
        !cx.read(|cx| shell.read(cx).help_state().can_toggle_sources),
        "and the sheet drops the source rows there"
    );
    // A click on the lane still works anywhere.
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("status-lane", 1usize), cx);
    })
    .unwrap();
    assert!(!selected(cx));
}

#[gpui_kit::test]
fn recall_enter_hands_the_text_over_and_clears_the_field(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, StatusBarModel::default());
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("cmd-k", cx);
        w.input("standup notes", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).command.read(cx).query(cx)), "");
}
