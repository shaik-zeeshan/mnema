//! Keeps the real mouse pointer out of goldens. GPUI seeds a new window's
//! pointer from the platform (`Window::new` reads `PlatformWindow::mouse_position`,
//! i.e. AppKit's `mouseLocationOutsideOfEventStream`), and AppKit clamps the
//! "offscreen" test window back onto a display, so whatever sat under the
//! person's pointer painted hovered. `open` parks the window's pointer far
//! outside it before the first frame; tests that want hover still move it
//! there with `simulate_event`, which overrides this.

use gpui_kit::{
    point, px, App, Entity, InputEvent, Modifiers, MouseMoveEvent, Pixels, Point, Render, Size,
    VisualTestAppContext, Window, WindowHandle,
};

/// Outside every window the suite opens: no hitbox contains it.
pub const AWAY: Point<Pixels> = Point {
    x: px(-10_000.),
    y: px(-10_000.),
};

/// `open_offscreen_window`, with the window's pointer parked at `AWAY` before
/// its first draw. Every golden window opens through this.
pub fn open<V: Render + 'static>(
    cx: &mut VisualTestAppContext,
    size: Size<Pixels>,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    cx.open_offscreen_window(size, |window, cx| {
        park(window, cx);
        build(window, cx)
    })
}

fn park(window: &mut Window, cx: &mut App) {
    let away = MouseMoveEvent {
        position: AWAY,
        modifiers: Modifiers::default(),
        pressed_button: None,
    };
    window.dispatch_event(away.to_platform_input(), cx);
}

/// The real system pointer, in global top-left points (CoreGraphics).
pub mod system {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGEventGetLocation(event: *mut std::ffi::c_void) -> CGPoint;
        fn CGWarpMouseCursorPosition(to: CGPoint) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *mut std::ffi::c_void);
    }

    pub fn location() -> Point<Pixels> {
        // SAFETY: a null source makes a fresh event we own and release.
        unsafe {
            let e = CGEventCreate(std::ptr::null());
            let p = CGEventGetLocation(e);
            CFRelease(e);
            point(px(p.x as f32), px(p.y as f32))
        }
    }

    /// Moves the real pointer (no event is posted). Only `hover::pointer_checks` calls it.
    pub fn warp(to: Point<Pixels>) {
        let to = CGPoint {
            x: f32::from(to.x) as f64,
            y: f32::from(to.y) as f64,
        };
        // SAFETY: plain C call with a value argument.
        unsafe { CGWarpMouseCursorPosition(to) };
    }
}
