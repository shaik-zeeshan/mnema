//! The Quick Recall window (`windows.rs` + `quick_recall_frame.rs`): GPUI's
//! `WindowKind::PopUp` NSPanel (non-activating, can become key, all Spaces,
//! accepts first mouse) plus floating level, transient + full-screen auxiliary,
//! not hide-on-deactivate and a no-op `cancelOperation:` (Esc stays ours). Built
//! once, then ordered in and out; never in the Dock decision. The bar's height
//! follows its content with the top edge pinned 15% down the work area.

use std::time::Instant;

use app_core::{DialogKind, MessageDialog};
use gpui_kit::base::Root;
use gpui_kit::prelude::*;
use gpui_kit::{
    point, px, size, transparent_black, AnyWindowHandle, App, Bounds, Context, Entity, Global,
    WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::Message as _;
use objc2::{msg_send, sel};
use objc2_app_kit::{NSScreen, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

use super::data::CoreBackend;
use super::model::{AudioHit, Cite, Ranked};
use super::{QuickRecall, BLUR_GRACE, CORNER, IDLE_H, TOP_FRACTION, WIDTH};

struct Panel {
    handle: AnyWindowHandle,
    qr: WeakEntity<QuickRecall>,
}

impl Global for Panel {}

/// The bar's frame in a work area, `top` measured down from the area's top edge:
/// `(x, top, w, h)`. A bar too tall to hang from the anchor is lifted until it fits.
pub fn bar_frame(area_x: f32, area_w: f32, area_h: f32, h: f32) -> (f32, f32, f32, f32) {
    let w = WIDTH.min(area_w);
    let h = h.min(area_h).max(1.);
    let top = (area_h * TOP_FRACTION).min(area_h - h).max(0.).round();
    ((area_x + (area_w - w) / 2.).round(), top, w, h.round())
}

fn options(cx: &App) -> WindowOptions {
    let origin = cx
        .primary_display()
        .map(|d| {
            let a = d.visible_bounds();
            let (x, top, _, _) = bar_frame(
                f32::from(a.origin.x),
                f32::from(a.size.width),
                f32::from(a.size.height),
                IDLE_H,
            );
            point(px(x), a.origin.y + px(top))
        })
        .unwrap_or(point(px(200.), px(160.)));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(WIDTH), px(IDLE_H)),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

extern "C-unwind" fn cancel_operation(_: &AnyObject, _: Sel, _: *mut AnyObject) {}

/// The Tauri panel's flags GPUI's PopUp doesn't set.
fn configure(window: &Window) {
    let Some(w) = crate::native::ns_window(window) else {
        return;
    };
    w.setLevel(3); // NSFloatingWindowLevel
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    w.setHidesOnDeactivate(false);
    // SAFETY: the window is GPUI's NSPanel subclass (WindowKind::PopUp): both
    // setters exist on NSPanel and take BOOL. `cancelOperation:` (v@:@) is added
    // to that subclass so NSPanel's Esc-closes default never runs; adding an
    // already-present method is a no-op.
    unsafe {
        let _: () = msg_send![w, setBecomesKeyOnlyIfNeeded: false];
        let _: () = msg_send![w, setFloatingPanel: true];
        let obj = w as *const _ as *const AnyObject;
        let cls = objc2::ffi::object_getClass(obj) as *mut AnyClass;
        let imp: objc2::runtime::Imp = std::mem::transmute(
            cancel_operation as extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject),
        );
        objc2::ffi::class_addMethod(cls, sel!(cancelOperation:), imp, c"v@:@".as_ptr());
    }
}

/// Resize to `h` with the top edge at its anchor (one native frame change:
/// `setContentSize` alone keeps Cocoa's bottom-left origin and grows upward).
pub fn fit(window: &mut Window, h: Option<f32>, native: bool, cx: &App) {
    let Some(w) = retained(window).filter(|_| native) else {
        if let Some(h) = h {
            window.resize(size(px(WIDTH), px(h)));
        }
        return;
    };
    outside_borrow(cx, move || set_frame(&w, h));
}

/// The NSWindow, kept past this borrow of the GPUI window.
fn retained(window: &Window) -> Option<Retained<NSWindow>> {
    crate::native::ns_window(window).map(|w| w.retain())
}

/// AppKit calls that call straight back into GPUI (resize, key, order out) run
/// on the next main-thread turn, outside the App borrow they were asked from
/// (inside it GPUI's callback fails with "RefCell already borrowed").
fn outside_borrow(cx: &App, f: impl FnOnce() + 'static) {
    cx.foreground_executor().spawn(async move { f() }).detach();
}

fn set_frame(w: &NSWindow, h: Option<f32>) {
    let screen = w
        .screen()
        .or_else(|| MainThreadMarker::new().and_then(NSScreen::mainScreen));
    let Some(screen) = screen else { return };
    let area = screen.visibleFrame();
    let current = w.frame();
    let (x, top, bw, bh) = bar_frame(
        area.origin.x as f32,
        area.size.width as f32,
        area.size.height as f32,
        h.unwrap_or(current.size.height as f32),
    );
    let y = area.origin.y + area.size.height - top as f64 - bh as f64;
    w.setFrame_display(
        NSRect::new(NSPoint::new(x as f64, y), NSSize::new(bw as f64, bh as f64)),
        true,
    );
    w.invalidateShadow();
}

fn panel(cx: &mut App) -> Option<(AnyWindowHandle, Entity<QuickRecall>)> {
    if let Some(p) = cx.try_global::<Panel>() {
        if let Some(qr) = p.qr.upgrade() {
            return Some((p.handle, qr));
        }
    }
    let core = crate::chat::core(cx)?;
    let developer = crate::shell::developer_options(cx);
    let mut built = None;
    let opened = cx.open_window(options(cx), |window, cx| {
        crate::native::apply_sharing_type(window, developer);
        crate::native::chrome::frameless(window, CORNER as f64);
        configure(window);
        let qr = cx.new(|cx| QuickRecall::new(std::sync::Arc::new(CoreBackend(core)), window, cx));
        built = Some(qr.clone());
        cx.new(|cx| Root::new(qr, window, cx).bg(transparent_black()))
    });
    let (handle, qr) = match (opened, built) {
        (Ok(h), Some(qr)) => (AnyWindowHandle::from(h), qr),
        (Err(e), _) => {
            log::error!("quick recall panel failed to open: {e:#}");
            return None;
        }
        _ => return None,
    };
    crate::native::dock::exclude(handle.window_id(), cx);
    cx.set_global(Panel {
        handle,
        qr: qr.downgrade(),
    });
    log::info!("quick recall panel built");
    Some((handle, qr))
}

/// ⌘⌥Space: dismiss when up, else summon.
pub fn toggle(cx: &mut App) {
    let Some((handle, qr)) = panel(cx) else {
        return;
    };
    let up = handle
        .update(cx, |_, w, _| crate::native::is_visible(w))
        .unwrap_or(false);
    if up {
        qr.update(cx, |qr, cx| qr.dismiss(cx));
    } else {
        summon(handle, &qr, cx);
    }
}

/// ↵ in the titlebar's recall field: summon with its text.
pub fn summon_with_query(text: String, cx: &mut App) {
    let Some((handle, qr)) = panel(cx) else {
        return;
    };
    qr.update(cx, |qr, cx| qr.take_query(&text, cx));
    summon(handle, &qr, cx);
}

fn summon(handle: AnyWindowHandle, qr: &Entity<QuickRecall>, cx: &mut App) {
    let now = cx.background_executor().now();
    qr.update(cx, |qr, _| qr.summoned(now));
    let input = qr.read(cx).input.clone();
    let w = handle
        .update(cx, |_, window, cx| {
            input.update(cx, |i, cx| {
                i.focus(window, cx);
                i.select_all(window, cx);
            });
            retained(window)
        })
        .ok()
        .flatten();
    if let Some(w) = w {
        outside_borrow(cx, move || {
            set_frame(&w, None);
            w.makeKeyAndOrderFront(None);
        });
    }
}

/// Order the panel out (no-op without one: tests).
pub fn order_out(cx: &mut App) {
    let Some(handle) = cx.try_global::<Panel>().map(|p| p.handle) else {
        return;
    };
    if let Some(w) = handle.update(cx, |_, w, _| retained(w)).ok().flatten() {
        outside_borrow(cx, move || w.orderOut(None));
    }
}

/// `ask_ai_update` for the panel's turn.
pub fn ask_update(ev: app_core::ask_ai::AskAiUpdateEvent, cx: &mut App) {
    let Some(qr) = cx.try_global::<Panel>().and_then(|p| p.qr.upgrade()) else {
        return;
    };
    qr.update(cx, |qr, cx| qr.on_ask_update(ev, cx));
}

/// A setting or download changed what the bar says: re-read it.
pub fn settings_changed(cx: &mut App) {
    let Some(qr) = cx.try_global::<Panel>().and_then(|p| p.qr.upgrade()) else {
        return;
    };
    qr.update(cx, |qr, cx| qr.load_ambient(cx));
}

pub fn message(title: &str, body: &str, cx: &mut App) {
    crate::native::dialogs::message(
        MessageDialog {
            title: title.into(),
            message: body.into(),
            kind: DialogKind::Info,
        },
        cx,
    );
}

/// Where a hand-off lands in the main window.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Frame(i64),
    Audio {
        segment: i64,
        span: u64,
        aligned: Option<i64>,
    },
}

impl Target {
    pub fn of(r: &Ranked, span: Option<u64>) -> Target {
        match r {
            Ranked::Frame(f) => Target::Frame(f.representative_frame.id),
            Ranked::Audio(a) => Target::Audio {
                segment: a.audio_segment.id,
                span: span.unwrap_or(a.span_start_ms),
                aligned: a.aligned_frame.as_ref().map(|f| f.id),
            },
        }
    }

    pub fn of_cite(c: &Cite) -> Target {
        match (c.kind.as_str(), c.frame_id, c.audio_segment_id) {
            ("frame", Some(id), _) => Target::Frame(id),
            (_, _, Some(segment)) => Target::Audio {
                segment,
                span: c.span_start_ms.unwrap_or(0),
                aligned: c.aligned_frame_id,
            },
            (_, id, _) => Target::Frame(id.unwrap_or_default()),
        }
    }

    pub fn or_aligned(self, a: &AudioHit) -> Target {
        match self {
            Target::Audio {
                segment,
                span,
                aligned: None,
            } => Target::Audio {
                segment,
                span,
                aligned: a.aligned_frame.as_ref().map(|f| f.id),
            },
            t => t,
        }
    }
}

impl QuickRecall {
    pub(super) fn summoned(&mut self, now: Instant) {
        self.summoned_at = Some(now);
        self.visible = true;
    }

    /// The titlebar's text replaces the query (and leaves Ask mode, closes a look).
    pub fn take_query(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.trim().is_empty() {
            return;
        }
        self.forced = None;
        self.look = None;
        self.pills.clear();
        self.set_text(text, cx);
        self.schedule_commit(cx);
        self.refresh(cx);
    }

    /// Answer links open in the browser: the next blur must not dismiss.
    pub fn suppress_next_blur(&mut self) {
        self.suppress_blur = true;
    }

    /// The setup blur within 300 ms of a summon, or a one-shot suppress, keeps the panel.
    pub fn should_dismiss_on_blur(&mut self, now: Instant) -> bool {
        if std::mem::take(&mut self.suppress_blur) {
            return false;
        }
        !self
            .summoned_at
            .is_some_and(|at| now.duration_since(at) < BLUR_GRACE)
    }

    /// The panel became (or stopped being) the key window.
    pub fn on_activation(&mut self, active: bool, cx: &mut Context<Self>) {
        self.focused = active;
        if active {
            self.visible = true;
            self.ask.handoff = false;
            self.mark_seen();
            // The user may have fixed the engine, or new apps were captured, since the last summon.
            self.load_ambient(cx);
            self.apps = None;
            self.ensure_apps(cx);
        } else {
            if !self.is_empty() {
                let ask = self.mode() == super::Mode::Ask;
                self.remember_query(ask, cx);
            }
            let now = cx.background_executor().now();
            if self.visible && self.should_dismiss_on_blur(now) {
                self.dismiss(cx);
            }
        }
        self.refresh(cx);
    }

    /// Order out; the state stays for a quick resume (the 5 s reset clears it).
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.focused = false;
        self.dismissals += 1;
        cx.defer(order_out);
        crate::native::dock::refresh(cx);
        self.refresh(cx);
    }

    /// Open in the main window's Timeline, then close.
    pub(super) fn hand_off(&mut self, target: Target, query: String, cx: &mut Context<Self>) {
        let meaning = self
            .look
            .as_ref()
            .map(|l| l.result.meaning())
            .unwrap_or_else(|| {
                self.results
                    .selected
                    .and_then(|i| self.visible_results().get(i))
                    .is_some_and(Ranked::meaning)
            });
        cx.defer(move |cx| open_in_timeline(target, query, meaning, cx));
        self.dismiss(cx);
    }
}

/// A frame hit lands on its frame; an audio hit opens its segment in the drawer
/// at the matched span, on its aligned frame.
fn open_in_timeline(target: Target, query: String, by_meaning: bool, cx: &mut App) {
    let frame_id = match target {
        Target::Frame(frame_id) => frame_id,
        Target::Audio {
            segment,
            span,
            aligned,
        } => {
            let recall = Some((query, by_meaning));
            return crate::timeline::open_audio(segment, span as i64, aligned, recall, cx);
        }
    };
    crate::window::show_main(cx);
    let Some(shell) = crate::window::shell(cx) else {
        return;
    };
    shell.update(cx, |s, cx| {
        s.go(crate::shell::Surface::Timeline, cx);
        let landing = crate::timeline::model::Landing {
            query,
            frame_id,
            screen: true,
            by_meaning,
        };
        s.timeline.update(cx, |t, cx| t.land(landing, cx));
    });
}

/// "Continue in Chat ↵": Chat opens the thread.
pub fn open_in_chat(id: String, cx: &mut App) {
    cx.defer(move |cx| {
        crate::window::show_main(cx);
        let Some(shell) = crate::window::shell(cx) else {
            return;
        };
        shell.update(cx, |s, cx| {
            s.go(crate::shell::Surface::Chat, cx);
            s.chat.update(cx, |c, cx| c.open(id, cx));
        });
    });
}

/// Settings at a tab (an ask notice's action, "Turn on in Settings →").
pub fn open_settings(tab: &'static str, cx: &mut App) {
    cx.defer(move |cx| crate::window::open_settings(Some(tab), cx));
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_bar_hangs_15_percent_down_and_lifts_when_too_tall() {
        assert_eq!(
            super::bar_frame(0., 1440., 875., 60.),
            (320., 131., 800., 60.)
        );
        assert_eq!(
            super::bar_frame(0., 1440., 875., 640.),
            (320., 131., 800., 640.)
        );
        // 800 tall in an 875 area: lifted to 75 so the bottom fits.
        assert_eq!(
            super::bar_frame(0., 1440., 875., 800.),
            (320., 75., 800., 800.)
        );
    }
}
