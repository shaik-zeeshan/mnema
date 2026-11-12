//! The surfaces are cached views: a plain draw (no `refresh`, unlike the kit's
//! `render_frame`) reuses every view nobody notified. These check that what a
//! surface shows still redraws it when its data changes underneath.

use std::sync::Arc;

use capture_types::{AppearanceSetting, TurnUpdate};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, Bounds, Entity, Point, TestAppContext, WindowAppearance, WindowBounds, WindowHandle,
    WindowOptions,
};

use super::*;
use crate::chat::{AskAiUpdateEvent, ChatTurn, Pane};
use crate::timeline::fixture::FixtureSource;

fn open(cx: &mut TestAppContext, fixture: &str) -> (WindowHandle<Root>, Entity<Shell>) {
    let (window, shell, _) = open_src(cx, fixture);
    (window, shell)
}

fn open_src(
    cx: &mut TestAppContext,
    fixture: &str,
) -> (WindowHandle<Root>, Entity<Shell>, Arc<FixtureSource>) {
    let source = Arc::new(FixtureSource::load(fixture).expect("fixture"));
    let src = source.clone();
    let (window, shell) = cx.update(|cx| {
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
        let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Shell::with_timeline_source(ShellModel::default(), source, window, cx))
        })
        .expect("window");
        (window.downcast::<Root>().expect("root"), shell)
    });
    (window, shell, src)
}

/// A plain draw: only dirty views re-render.
fn draw(cx: &mut TestAppContext, window: WindowHandle<Root>) {
    cx.update_window(window.into(), |_, w, cx| w.draw(cx).clear(cx))
        .unwrap();
}

fn shows(cx: &mut TestAppContext, window: WindowHandle<Root>, id: &'static str) -> bool {
    cx.update_window(window.into(), |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

fn set_status(cx: &mut TestAppContext, shell: &Entity<Shell>, f: impl FnOnce(&mut ShellModel)) {
    shell.update(cx, |s, cx| {
        let mut model = s.model.clone();
        f(&mut model);
        s.set_model(model, cx);
    });
}

#[gpui_kit::test]
fn frames_landing_on_the_shown_timeline_redraw_it(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, "default");
    draw(cx, window);
    let timeline = cx.read(|cx| shell.read(cx).timeline.clone());
    assert!(cx.read(|cx| timeline.read(cx).paging.frames.is_empty()));
    assert!(!shows(cx, window, "readout-name"), "no frame yet");
    cx.run_until_parked();
    draw(cx, window);
    assert!(cx.read(|cx| !timeline.read(cx).paging.frames.is_empty()));
    assert!(
        shows(cx, window, "readout-name"),
        "the landed frames are drawn"
    );
}

#[gpui_kit::test]
fn set_model_redraws_insights_and_settings_only_for_what_they_show(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, "first-run");
    cx.run_until_parked();
    let insights = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).insights.read(cx).renders);
    let settings = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).settings.read(cx).renders);

    shell.update(cx, |s, cx| s.go(Surface::Insights, cx));
    draw(cx, window);
    let before = insights(cx);
    set_status(cx, &shell, |m| m.status.running = true);
    draw(cx, window);
    assert!(insights(cx) > before, "recording changes the Journal");

    shell.update(cx, |s, cx| s.go(Surface::Settings, cx));
    draw(cx, window);
    let before = settings(cx);
    // The 2 s live feed: Settings shows none of it.
    set_status(cx, &shell, |m| m.status.health = Some(Default::default()));
    draw(cx, window);
    assert_eq!(settings(cx), before, "a health reading skips Settings");
    set_status(cx, &shell, |m| m.status.running = false);
    draw(cx, window);
    assert!(settings(cx) > before, "its REC pill and locked sources");
}

#[gpui_kit::test]
fn a_chat_update_while_chat_shows_redraws_it(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, "first-run");
    cx.run_until_parked();
    shell.update(cx, |s, cx| s.go(Surface::Chat, cx));
    let chat = cx.read(|cx| shell.read(cx).chat.clone());
    chat.update(cx, |c, cx| {
        c.active = Some("c1".into());
        c.pane = Pane::Thread;
        let mut t = ChatTurn::new(0, "What did I ship?", "streaming");
        t.version = 1;
        c.turns.push(t);
        c.streaming = true;
        cx.notify();
    });
    draw(cx, window);
    assert!(!shows(cx, window, "chat-copy-0"), "nothing to copy yet");
    let ev = |version, update| AskAiUpdateEvent {
        conversation_id: "c1".into(),
        version,
        turn_index: 0,
        update,
    };
    chat.update(cx, |c, cx| {
        c.on_update(
            ev(
                2,
                TurnUpdate::AppendProse {
                    text: "Shipped the parser.".into(),
                },
            ),
            cx,
        );
        c.on_update(ev(3, TurnUpdate::Done), cx);
    });
    draw(cx, window);
    assert!(shows(cx, window, "chat-copy-0"), "the streamed answer drew");
}

#[gpui_kit::test]
fn screens_waking_redraws_the_cached_surface(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, "first-run");
    cx.run_until_parked();
    let renders = |cx: &mut TestAppContext| {
        draw(cx, window);
        cx.read(|cx| shell.read(cx).timeline.read(cx).renders)
    };
    let before = renders(cx);
    cx.update(|cx| crate::window::set_screens_asleep(true, cx));
    assert_eq!(renders(cx), before, "sleeping redraws nothing");
    cx.update(|cx| crate::window::set_screens_asleep(false, cx));
    assert_eq!(renders(cx), before + 1, "waking refreshes stale labels");
}

// GPUI's test window can simulate occlusion, but only from inside gpui-pre
// (`test_window` is crate-private), so these drive the shell's observer.
fn visibility(cx: &mut TestAppContext, shell: &Entity<Shell>, v: WindowVisibility) {
    shell.update(cx, |s, cx| s.on_visibility(v, cx));
}

#[gpui_kit::test]
fn a_covered_window_skips_the_head_poll_and_polls_once_uncovered(cx: &mut TestAppContext) {
    let (window, shell, src) = open_src(cx, "default");
    cx.run_until_parked();
    assert!(
        !cx.update(crate::window::render_idle),
        "no visibility report yet reads as visible"
    );
    let polls = || {
        src.calls()
            .iter()
            .filter(|c| matches!(c, crate::timeline::fixture::Call::ListFrames { .. }))
            .count()
    };
    let renders = |cx: &mut TestAppContext| {
        draw(cx, window);
        cx.read(|cx| shell.read(cx).timeline.read(cx).renders)
    };
    visibility(cx, &shell, WindowVisibility::Hidden);
    assert!(cx.update(crate::window::render_idle));
    let (before, drawn) = (polls(), renders(cx));
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    assert_eq!(polls(), before, "four head polls skipped while covered");
    visibility(cx, &shell, WindowVisibility::Visible);
    cx.run_until_parked();
    assert_eq!(
        polls(),
        before + 1,
        "uncovered: the head is checked at once"
    );
    assert!(renders(cx) > drawn, "and the cached Timeline redraws");
}

#[gpui_kit::test]
fn a_covered_window_skips_the_rec_tick(cx: &mut TestAppContext) {
    let (window, shell) = open(cx, "first-run");
    set_status(cx, &shell, |m| {
        m.status.running = true;
        m.status.session_start_ms = Some(0);
    });
    shell.update(cx, |s, cx| s.go(Surface::Settings, cx));
    cx.run_until_parked();
    let renders = |cx: &mut TestAppContext| {
        draw(cx, window);
        cx.read(|cx| shell.read(cx).settings.read(cx).renders)
    };
    let before = renders(cx);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        renders(cx),
        before + 1,
        "visible: the tick redraws the rail clock"
    );
    visibility(cx, &shell, WindowVisibility::Hidden);
    let before = renders(cx);
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(renders(cx), before, "covered: the tick does nothing");
}
