//! The picker's flows on a real window: the trigger opens it, keys and clicks
//! reach the parent's callbacks (`ModelPickerMenu.svelte`).

use std::cell::RefCell;
use std::rc::Rc;

use capture_types::{AiProviderConfig, AiProviderKind, AppearanceSetting};
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    div, px, size, Bounds, Context, Entity, Point, TestAppContext, Window, WindowAppearance,
    WindowBounds, WindowHandle, WindowOptions,
};

use super::*;
use crate::theme;
use crate::ui::nid;

struct Host {
    spec: Spec,
    calls: Rc<RefCell<Vec<String>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let log = |tag: &'static str, calls: &Rc<RefCell<Vec<String>>>| {
            let calls = calls.clone();
            move |e: EngineRef, _: &mut Window, _: &mut App| {
                calls.borrow_mut().push(format!("{tag} {}/{}", e.0, e.1))
            }
        };
        let (retry, link) = (self.calls.clone(), self.calls.clone());
        div().size_full().p(px(20.)).child(
            ModelPicker::new("mp", self.spec.clone())
                .on_favorite(log("star", &self.calls))
                .on_make_default(log("default", &self.calls))
                .on_retry(move |_, _| retry.borrow_mut().push("retry".into()))
                .foot_link("Settings → AI ↗", move |_, _| {
                    link.borrow_mut().push("settings".into())
                }),
        )
    }
}

fn provider(id: &str, kind: AiProviderKind) -> AiProviderConfig {
    AiProviderConfig {
        id: id.into(),
        kind,
        label: String::new(),
        base_url: String::new(),
    }
}

fn engine(m: &str) -> EngineRef {
    ("anthropic".into(), m.into())
}

fn spec() -> Spec {
    let pool = ["claude-sonnet-4-5", "claude-haiku-4-5"].map(|id| PoolModel {
        id: id.into(),
        provider: "anthropic".into(),
        context_window: None,
    });
    Spec {
        providers: vec![provider("anthropic", AiProviderKind::Anthropic)],
        pool: pool.to_vec(),
        favorites: vec![engine("claude-sonnet-4-5"), engine("claude-haiku-4-5")],
        sentinel: Some(("Default".into(), Some(engine("claude-sonnet-4-5")))),
        sentinel_selected: true,
        foot: Some("applies to this chat only".into()),
        can_star: true,
        make_default: Some(Some(engine("claude-sonnet-4-5"))),
        ..Default::default()
    }
}

fn open(
    cx: &mut TestAppContext,
    spec: Spec,
) -> (WindowHandle<Root>, Entity<Host>, Rc<RefCell<Vec<String>>>) {
    let calls: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = calls.clone();
    let (w, host) = cx.update(|cx| {
        gpui_kit::init(cx);
        theme::init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(700.), px(700.)),
            })),
            ..Default::default()
        };
        let (w, host) =
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Host { spec, calls: log }))
                .expect("window");
        (w.downcast::<Root>().expect("root"), host)
    });
    act(cx, w, |w, cx| w.click("mp", cx));
    (w, host, calls)
}

fn act(cx: &mut TestAppContext, w: WindowHandle<Root>, f: impl FnOnce(&mut Window, &mut App)) {
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shown(
    cx: &mut TestAppContext,
    w: WindowHandle<Root>,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    let id = id.into();
    cx.update_window(w.into(), |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn cmd_d_stars_the_lit_row_and_typing_relights_the_top(cx: &mut TestAppContext) {
    let (w, _, calls) = open(cx, spec());
    assert!(shown(cx, w, "mp-pop"), "the trigger opens it");
    // Quick view: Default (0), sonnet (1), haiku (2).
    act(cx, w, |w, cx| w.press("down", cx));
    act(cx, w, |w, cx| w.press("cmd-d", cx));
    assert_eq!(*calls.borrow(), ["star anthropic/claude-sonnet-4-5"]);
    act(cx, w, |w, cx| w.press("down", cx));
    // Typing re-filters ("h": both models, anthropic matches): the top row is
    // lit again, so ⌘D stars sonnet, not a row past the end of the list.
    act(cx, w, |w, cx| w.input("h", cx));
    act(cx, w, |w, cx| w.press("cmd-d", cx));
    assert_eq!(
        *calls.borrow(),
        [
            "star anthropic/claude-sonnet-4-5",
            "star anthropic/claude-sonnet-4-5"
        ]
    );
}

#[gpui_kit::test]
fn make_default_is_offered_on_every_row_but_the_current_default(cx: &mut TestAppContext) {
    let (w, _, calls) = open(cx, spec());
    // Rows: Default (0), sonnet (1, the current default), haiku (2).
    assert!(
        !shown(cx, w, nid("mp-mkdef", 1)),
        "never on the current default"
    );
    assert!(shown(cx, w, nid("mp-mkdef", 2)));
    act(cx, w, |w, cx| w.press("down", cx));
    act(cx, w, |w, cx| w.press("down", cx));
    act(cx, w, |w, cx| w.click(nid("mp-mkdef", 2), cx));
    assert_eq!(*calls.borrow(), ["default anthropic/claude-haiku-4-5"]);
    assert!(shown(cx, w, "mp-pop"), "making a default doesn't pick it");
}

#[gpui_kit::test]
fn the_footer_link_closes_the_picker_and_goes(cx: &mut TestAppContext) {
    let (w, _, calls) = open(cx, spec());
    act(cx, w, |w, cx| w.click("mp-foot-link", cx));
    assert_eq!(*calls.borrow(), ["settings"]);
    assert!(!shown(cx, w, "mp-pop"));
}

#[gpui_kit::test]
fn a_provider_that_failed_to_list_offers_retry(cx: &mut TestAppContext) {
    let mut s = spec();
    s.favorites.clear();
    s.providers.push(provider("openai", AiProviderKind::Openai));
    s.failures = vec![ProviderFailure {
        provider: "openai".into(),
        reason: "timed out".into(),
    }];
    let (w, host, calls) = open(cx, s);
    act(cx, w, |w, cx| w.click("mp-retry", cx));
    assert_eq!(*calls.borrow(), ["retry"]);
    // "Retrying…" while it lists: a second click does nothing.
    host.update(cx, |h, cx| {
        h.spec.loading = true;
        cx.notify();
    });
    act(cx, w, |w, cx| w.click("mp-retry", cx));
    assert_eq!(*calls.borrow(), ["retry"]);
}
