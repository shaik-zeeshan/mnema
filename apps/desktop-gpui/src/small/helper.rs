//! The permission helper (`routes/permission-helper/+page.svelte`,
//! `windows/permission_helper.rs`): every "Open System Settings" opens the right
//! Privacy & Security pane and floats this 460×62 card bottom-centre, 28 pt above
//! the work area, as a non-activating panel. It polls its one permission every
//! second and closes itself a beat after the switch flips.

use std::sync::Arc;
use std::time::Duration;

use capture_types::CapturePermissionState;
use gpui_kit::prelude::*;
use gpui_kit::{div, img, px, relative, App, AppContext, Context, Image, Render, Task, Window};

use crate::theme::{self, radius, size, space, text, FONT_MONO};
use crate::ui::{icon, progress, Button};

pub const SIZE: (f32, f32) = (460., 62.);
pub const POLL: Duration = Duration::from_millis(1000);
/// Long enough to read "On", short enough that it feels like it closed itself.
pub const CLOSE_AFTER_GRANT: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermKind {
    Screen,
    Microphone,
    SystemAudio,
    Accessibility,
}

impl PermKind {
    /// Unknown kinds read as Screen (`permissionHelperKind`).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "microphone" => Self::Microphone,
            "systemAudio" => Self::SystemAudio,
            "accessibility" => Self::Accessibility,
            _ => Self::Screen,
        }
    }
    /// The System Settings list it lives in (`PERMISSION_HELPER_LIST`).
    pub fn list(self) -> &'static str {
        match self {
            Self::Screen => "Screen Recording",
            Self::Microphone => "Microphone",
            Self::SystemAudio => "Screen & System Audio Recording",
            Self::Accessibility => "Accessibility",
        }
    }
    fn pane(self) -> &'static str {
        match self {
            Self::Screen | Self::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            Self::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            Self::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
        }
    }
}

/// Has the user flipped the switch? A failed read is just "not yet".
pub trait GrantCheck: Send + Sync + 'static {
    fn granted(&self, kind: PermKind) -> Result<bool, String>;
}

pub struct LiveGrant;

impl GrantCheck for LiveGrant {
    fn granted(&self, kind: PermKind) -> Result<bool, String> {
        let ok = |s: CapturePermissionState| {
            matches!(
                s,
                CapturePermissionState::Granted | CapturePermissionState::AssumedWorking
            )
        };
        Ok(match kind {
            PermKind::Screen => ok(capture_screen::screen_permission_state()),
            PermKind::Microphone => ok(capture_microphone::microphone_permission_state()),
            // shortcut: system audio has no authorization query (ADR 0052); its
            // "assumed working" evidence row is read by native_capture (S3), so
            // until then this helper waits until dismissed, as it usually does.
            PermKind::SystemAudio => false,
            // shortcut: the AXIsProcessTrusted read is S7's native layer.
            PermKind::Accessibility => false,
        })
    }
}

pub struct PermissionHelper {
    pub kind: PermKind,
    pub granted: bool,
    checker: Arc<dyn GrantCheck>,
    icon: Arc<Image>,
    _poll: Task<()>,
}

impl PermissionHelper {
    pub fn new(kind: PermKind, checker: Arc<dyn GrantCheck>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            kind,
            granted: false,
            checker,
            icon: super::update::app_icon(),
            _poll: Task::ready(()),
        };
        this.start(kind, cx);
        this
    }

    /// (Re)point at `kind`: check now, then every second until granted.
    pub fn start(&mut self, kind: PermKind, cx: &mut Context<Self>) {
        self.kind = kind;
        self.granted = false;
        let checker = self.checker.clone();
        self._poll = cx.spawn(async move |this, cx| loop {
            let c = checker.clone();
            let granted = cx
                .background_spawn(async move { c.granted(kind).unwrap_or(false) })
                .await;
            if granted {
                if this
                    .update(cx, |h, cx| {
                        h.granted = true;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(CLOSE_AFTER_GRANT).await;
                let _ = this.update(cx, |_, cx| cx.emit(Closed));
                return;
            }
            cx.background_executor().timer(POLL).await;
        });
        cx.notify();
    }
}

/// The helper asks its window to close (granted, then the beat passed).
pub struct Closed;

impl gpui_kit::EventEmitter<Closed> for PermissionHelper {}

impl Render for PermissionHelper {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::request_fade_frames(window, cx);
        let p = theme::tokens(cx);
        let state = if self.granted {
            div()
                .text_color(p.accent)
                .child(icon("icons/check.svg", px(13.)))
                .child("On \u{2014} you can close System Settings")
        } else {
            div()
                .child(progress::spinner(size::SPIN_SM))
                .child("Waiting for the switch \u{2014} this closes by itself")
        };
        div()
            .id("permission-helper")
            .role(gpui_kit::Role::Status)
            .size_full()
            .flex()
            .items_center()
            .gap(space::S3)
            .pl(px(10.))
            .pr(px(8.))
            .py(px(10.))
            .rounded(radius::MD)
            .bg(p.pop_bg)
            .border_1()
            .border_color(p.overlay_border)
            .font_family(theme::FONT_UI)
            .child(
                img(self.icon.clone())
                    .flex_none()
                    .size(px(40.))
                    .rounded(px(9.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(theme::W_MEDIUM)
                            .text_size(text::MD)
                            .line_height(relative(1.3))
                            .text_color(p.text_strong)
                            .child(format!("Turn on Mnema in the {} list", self.kind.list())),
                    )
                    .child(
                        state
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .mt(px(3.))
                            .font_family(FONT_MONO)
                            .text_size(text::XS)
                            .line_height(relative(1.3))
                            .when(!self.granted, |d| d.text_color(p.text_subtle)),
                    ),
            )
            .child(
                Button::icon_only("helper-dismiss", "icons/x.svg", "Dismiss")
                    .ghost()
                    .sm()
                    .on_click(|_, window, _| window.remove_window()),
            )
    }
}

/// Open the pane for `kind` and float the helper beside it (re-pointing an open one).
pub fn open(kind: &str, cx: &mut App) {
    let kind = PermKind::from_wire(kind);
    // Pane first: System Settings activates on top; the helper's panel level keeps it above.
    cx.open_url(kind.pane());
    super::open_or_focus::<PermissionHelper>(
        super::Kind::Helper,
        cx,
        |window, cx| {
            let view = cx.new(|cx| PermissionHelper::new(kind, Arc::new(LiveGrant), cx));
            let handle = window.window_handle();
            cx.subscribe(&view, move |_, _: &Closed, cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            })
            .detach();
            view
        },
        |view, _, cx| view.update(cx, |h, cx| h.start(kind, cx)),
    );
}
