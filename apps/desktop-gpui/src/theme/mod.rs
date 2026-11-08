//! The one theme: §3.7 tokens for both modes, the appearance setting
//! (system / light / dark, following the OS), the 180 ms cross-fade, and the
//! same colours pushed into gpui-kit's `ThemeColor` so kit components match.
//!
//! Views read `theme::tokens(cx)` (a [`Palette`]) and the scales in [`scale`].

pub mod fonts;
mod kit;
mod palette;
pub mod reduce_motion;
pub mod scale;

use std::time::Instant;

use capture_types::AppearanceSetting;
use gpui_kit::{App, Global, Window, WindowAppearance};

pub use palette::Palette;
pub use scale::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    pub fn from_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Mode::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Mode::Dark => Palette::dark(),
            Mode::Light => Palette::light(),
        }
    }
}

struct ThemeState {
    setting: AppearanceSetting,
    /// What the OS last reported; used while `setting` is `System`.
    system: Mode,
    mode: Mode,
    /// The palette we are fading from, and when the fade started.
    fade: Option<(Palette, Instant)>,
}

impl Global for ThemeState {}

/// Install fonts, the theme global and the kit mapping. Call once after `gpui_kit::init`.
pub fn init(setting: AppearanceSetting, system: WindowAppearance, cx: &mut App) {
    if let Err(e) = fonts::register(cx) {
        log::error!("theme: embedded fonts failed to load: {e:#}");
    }
    let system = Mode::from_appearance(system);
    let mode = resolve(&setting, system);
    cx.set_global(ThemeState {
        setting,
        system,
        mode,
        fade: None,
    });
    kit::apply(mode, cx);
}

fn resolve(setting: &AppearanceSetting, system: Mode) -> Mode {
    match setting {
        AppearanceSetting::System => system,
        AppearanceSetting::Light => Mode::Light,
        AppearanceSetting::Dark => Mode::Dark,
    }
}

pub fn mode(cx: &App) -> Mode {
    cx.global::<ThemeState>().mode
}

pub fn setting(cx: &App) -> AppearanceSetting {
    cx.global::<ThemeState>().setting.clone()
}

/// The palette to paint with now: mid-fade it blends old into new.
pub fn tokens(cx: &App) -> Palette {
    let state = cx.global::<ThemeState>();
    let target = state.mode.palette();
    match state.fade {
        Some((from, started)) => match fade_progress(started, cx) {
            Some(t) => from.lerp(&target, motion::quart(t)),
            None => target,
        },
        None => target,
    }
}

/// `Some(0..1)` while a cross-fade is running.
fn fade_progress(started: Instant, cx: &App) -> Option<f32> {
    let elapsed = cx
        .background_executor()
        .now()
        .saturating_duration_since(started);
    (elapsed < motion::THEME_FADE).then(|| elapsed.as_secs_f32() / motion::THEME_FADE.as_secs_f32())
}

/// A cross-fade is still running.
pub fn fading(cx: &App) -> bool {
    cx.global::<ThemeState>()
        .fade
        .is_some_and(|(_, started)| fade_progress(started, cx).is_some())
}

/// Keep repainting while a cross-fade runs. Call from the root view's render.
pub fn request_fade_frames(window: &mut Window, cx: &App) {
    if let Some((_, started)) = cx.global::<ThemeState>().fade {
        if fade_progress(started, cx).is_some() {
            window.request_animation_frame();
        }
    }
}

/// The Settings › Appearance choice changed.
pub fn set_setting(setting: AppearanceSetting, cx: &mut App) {
    cx.global_mut::<ThemeState>().setting = setting;
    reresolve(cx);
}

/// The OS appearance changed (wire to `observe_window_appearance`).
pub fn set_system(appearance: WindowAppearance, cx: &mut App) {
    cx.global_mut::<ThemeState>().system = Mode::from_appearance(appearance);
    reresolve(cx);
}

fn reresolve(cx: &mut App) {
    let from = tokens(cx);
    let reduce = cx.reduce_motion();
    let now = cx.background_executor().now();
    let state = cx.global_mut::<ThemeState>();
    let next = resolve(&state.setting, state.system);
    if next == state.mode {
        return;
    }
    state.mode = next;
    state.fade = (!reduce).then_some((from, now));
    kit::apply(next, cx);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn setting_and_system_resolve_with_a_180ms_fade(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(AppearanceSetting::System, WindowAppearance::Dark, cx);
            assert_eq!(mode(cx), Mode::Dark);
            set_system(WindowAppearance::Light, cx);
            assert_eq!(mode(cx), Mode::Light);
            // Mid-fade: neither palette yet.
            assert_ne!(tokens(cx).bg, Palette::light().bg);
        });
        cx.executor().advance_clock(Duration::from_millis(179));
        cx.update(|cx| assert!(fading(cx), "still fading at 179 ms"));
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.update(|cx| {
            assert!(!fading(cx), "done at 180 ms");
            assert_eq!(tokens(cx), Palette::light());
            // A pinned setting ignores the OS.
            set_setting(AppearanceSetting::Dark, cx);
            set_system(WindowAppearance::Light, cx);
            assert_eq!(mode(cx), Mode::Dark);
            assert_eq!(
                gpui_kit::component::ActiveTheme::theme(cx).background,
                Palette::dark().bg
            );
        });
    }

    #[gpui_kit::test]
    fn reduce_motion_skips_the_fade(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(AppearanceSetting::Dark, WindowAppearance::Dark, cx);
            cx.set_reduce_motion(true);
            set_setting(AppearanceSetting::Light, cx);
            assert_eq!(tokens(cx), Palette::light());
        });
    }
}
