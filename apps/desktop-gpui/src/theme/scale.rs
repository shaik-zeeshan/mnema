//! Mode-independent scales from kit.css / tokens.css: type, spacing, radii,
//! control heights, motion. Views use these names, never literals.

use std::time::Duration;

use gpui_kit::{px, FontWeight, Pixels};

/// UI voice (titles, prose, buttons, labels).
pub const FONT_UI: &str = "Hanken Grotesk";
/// Record voice (data, timestamps, kickers, kbd). Numbers are tabular by design.
pub const FONT_MONO: &str = "Spline Sans Mono";

/// `--text-xs … --text-xl`: 10 / 11 / 12 / 13 / 16 / 20.
pub mod text {
    use super::*;
    pub const XS: Pixels = px(10.);
    pub const SM: Pixels = px(11.);
    pub const BASE: Pixels = px(12.);
    pub const MD: Pixels = px(13.);
    pub const LG: Pixels = px(16.);
    pub const XL: Pixels = px(20.);
    /// `.mx-prose`.
    pub const PROSE: Pixels = px(14.);
    pub const H1: Pixels = px(28.);
    pub const DISPLAY: Pixels = px(40.);
    /// The date capsule's mono caption.
    pub const MICRO: Pixels = px(9.5);
}

pub const W_REGULAR: FontWeight = FontWeight::NORMAL;
pub const W_MEDIUM: FontWeight = FontWeight::MEDIUM;
pub const W_SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;

/// Line heights as multiples (`body 13/1.6`, labels 1.35, headings 1.25).
pub mod leading {
    pub const BODY: f32 = 1.6;
    pub const SNUG: f32 = 1.35;
    pub const TIGHT: f32 = 1.25;
    pub const ONE: f32 = 1.0;
}

/// `--s-1 … --s-8`: 4 / 8 / 12 / 16 / 24 / 32 / 48 / 64.
pub mod space {
    use super::*;
    pub const S1: Pixels = px(4.);
    pub const S2: Pixels = px(8.);
    pub const S3: Pixels = px(12.);
    pub const S4: Pixels = px(16.);
    pub const S5: Pixels = px(24.);
    pub const S6: Pixels = px(32.);
    pub const S7: Pixels = px(48.);
    pub const S8: Pixels = px(64.);
}

/// `--r-xs … --r-pill`: 4 / 6 / 8 / 12 / 16 / pill.
pub mod radius {
    use super::*;
    pub const XS: Pixels = px(4.);
    pub const SM: Pixels = px(6.);
    pub const MD: Pixels = px(8.);
    pub const LG: Pixels = px(12.);
    pub const XL: Pixels = px(16.);
    pub const PILL: Pixels = px(999.);
    /// `.mx-btn--sm` corner (`--_r: 5px`): small buttons, icon buttons.
    pub const BTN_SM: Pixels = px(5.);
}

/// Control heights (`--h-sm` 26, `--h-md` 32) plus the shell's fixed bars (§3.6).
pub mod size {
    use super::*;
    pub const H_SM: Pixels = px(26.);
    pub const H_MD: Pixels = px(32.);
    /// The status bar's mini buttons (`--_h: 22px`).
    pub const H_MINI: Pixels = px(22.);
    pub const TITLEBAR: Pixels = px(38.);
    pub const STATUS_BAR: Pixels = px(30.);
    /// Left inset where macOS draws the traffic lights.
    pub const TRAFFIC_LIGHTS: Pixels = px(78.);
    pub const NAV_TAB_W: Pixels = px(32.);
    pub const NAV_TAB_H: Pixels = px(26.);
    pub const CMD_H: Pixels = px(28.);
    pub const CMD_MIN_W: Pixels = px(260.);
    pub const CMD_MAX_W: Pixels = px(440.);
    pub const ICON: Pixels = px(15.);
    pub const ICON_SM: Pixels = px(13.);
    pub const ICON_XS: Pixels = px(11.);
    pub const KBD: Pixels = px(18.);
    pub const REC_PILL_H: Pixels = px(20.);
    pub const POP_KEYS_W: Pixels = px(560.);
    pub const POP_NOTIFS_W: Pixels = px(380.);
    pub const POP_STATUS_W: Pixels = px(340.);
    /// Popovers sit 8px off their trigger.
    pub const POP_GAP: Pixels = px(8.);
    pub const FOCUS_RING: Pixels = px(3.);
    /// Button / tile / switch focus: `outline: 2px solid; outline-offset: 2px`.
    pub const OUTLINE: Pixels = px(2.);
    pub const OUTLINE_OFFSET: Pixels = px(2.);
    /// Icon sizes inside md / sm buttons.
    pub const BTN_ICON_MD: Pixels = px(15.);
    pub const BTN_ICON_SM: Pixels = px(13.);
    /// `.mx-chip`, `.mx-rec` and status pills.
    pub const CHIP_H: Pixels = px(20.);
    pub const SWITCH_W: Pixels = px(30.);
    pub const SWITCH_H: Pixels = px(18.);
    pub const SWITCH_KNOB: Pixels = px(14.);
    pub const SLIDER_THUMB: Pixels = px(14.);
    pub const PROGRESS_H: Pixels = px(4.);
    pub const SPIN: Pixels = px(14.);
    pub const SPIN_SM: Pixels = px(10.);
    pub const SPIN_LG: Pixels = px(20.);
    pub const TOAST_MIN_H: Pixels = px(38.);
    pub const TOAST_MAX_W: Pixels = px(520.);
    /// Toasts float above the 30 px status bar (`bottom: 42px`).
    pub const TOAST_BOTTOM: Pixels = px(42.);
    pub const BANNER_MIN_H: Pixels = px(38.);
    pub const MODAL_MAX_W: Pixels = px(640.);
    /// The licence deep-link receipt (`.license-receipt__panel`).
    /// Room for a month per ~30 px across a year's update window on the ruler.
    pub const RECEIPT_MAX_W: Pixels = px(440.);
    pub const STRIP_LIGHT: Pixels = px(12.);
    pub const CAL_CELL_W: Pixels = px(34.);
    pub const CAL_CELL_H: Pixels = px(32.);
    pub const DATE_RULER_W: Pixels = px(84.);
    pub const DATE_RULER_H: Pixels = px(22.);
    pub const SELECT_MAX_H: Pixels = px(220.);
    pub const TIP_MAX_W: Pixels = px(260.);
    /// Chat (`routes/chat/+page.svelte`): list 276, 240 below 1180 px wide.
    pub const CHAT_LIST_W: Pixels = px(276.);
    pub const CHAT_LIST_W_NARROW: Pixels = px(240.);
    pub const CHAT_LIST_BREAKPOINT: Pixels = px(1180.);
    pub const CHAT_THREAD_MAX_W: Pixels = px(760.);
    pub const CHAT_COMPOSER_DOCKED_MAX_W: Pixels = px(696.);
    pub const CHAT_HEAD_H: Pixels = px(48.);
}

/// Popovers: CSS `backdrop-filter: blur(22px) saturate(1.4)` over a 96% raised
/// surface. GPUI can't blur inside a window, so the popover surface (`pop_bg`)
/// is this opaque instead; the founder's open question decides it (one line).
pub const POP_SURFACE_ALPHA: f32 = 1.0;

/// Toasts stay up this long (kit: 4-6 s); an Undo toast drains over it.
pub const TOAST_TTL: Duration = Duration::from_secs(5);

/// Breakpoints the Svelte shell uses: command hints vanish below 1000px, the
/// status bar's wide readouts below 900px.
pub mod breakpoint {
    use super::*;
    pub const CMD_HINTS: Pixels = px(1000.);
    pub const STATUS_WIDE: Pixels = px(900.);
    /// Insights' two-column grids (`.ex` / `.feed` / `.in-feed` / `.in-ex`) stack at or below.
    pub const INSIGHTS_ONE_COL: Pixels = px(1180.);
    /// `.in-read`: the stats drop under the headline, four across, at or below.
    pub const READ_STACK: Pixels = px(1000.);
    /// `.in-read` stats two across at or below; the subject page's first fold (`@container sd`).
    pub const READ_NARROW: Pixels = px(760.);
    /// The subject page's second fold (`@container sd`).
    pub const SUBJECT_TIGHT: Pixels = px(560.);
    /// Below this Insights pane width the evidence drawer covers the pane as a sheet.
    pub const DRAWER_SHEET: Pixels = px(700.);
    /// `TranscriptReader`: a turn's speaker stacks over its text at or below.
    pub const TRANSCRIPT_STACK: Pixels = px(820.);
    /// Settings: the rail narrows and the content padding tightens at or below.
    pub const SETTINGS_TIGHT: Pixels = px(800.);
    /// Settings' page column starts sliding to the centre past this window width.
    pub const SETTINGS_CENTRE: Pixels = px(1280.);
}

pub const DISABLED_OPACITY: f32 = 0.4;
pub const BUSY_OPACITY: f32 = 0.6;

/// Motion: ease-out only, never bounce.
pub mod motion {
    use super::*;
    pub const FAST: Duration = Duration::from_millis(120);
    pub const MED: Duration = Duration::from_millis(220);
    pub const SLOW: Duration = Duration::from_millis(420);
    /// Theme change cross-fade.
    pub const THEME_FADE: Duration = Duration::from_millis(180);
    /// Tooltip dwell; instant on keyboard focus.
    pub const TOOLTIP_DWELL: Duration = Duration::from_millis(350);
    /// `.mx-spin`: one turn, linear, forever (the app's only looping motion).
    pub const SPIN: Duration = Duration::from_millis(800);

    /// `cubic-bezier(.16, 1, .3, 1)`.
    pub fn expo(t: f32) -> f32 {
        cubic_bezier(0.16, 1.0, 0.3, 1.0, t)
    }

    /// `cubic-bezier(.25, 1, .5, 1)`.
    pub fn quart(t: f32) -> f32 {
        cubic_bezier(0.25, 1.0, 0.5, 1.0, t)
    }

    /// CSS cubic-bezier: solve x(s) = t by Newton then bisection, return y(s).
    pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        let curve = |a: f32, b: f32, s: f32| {
            3. * a * s * (1. - s).powi(2) + 3. * b * s * s * (1. - s) + s.powi(3)
        };
        let slope = |a: f32, b: f32, s: f32| {
            3. * a * (1. - s).powi(2) + 6. * (b - a) * s * (1. - s) + 3. * (1. - b) * s * s
        };
        let mut s = t;
        for _ in 0..8 {
            let dx = curve(x1, x2, s) - t;
            let d = slope(x1, x2, s);
            if dx.abs() < 1e-5 || d.abs() < 1e-6 {
                break;
            }
            s = (s - dx / d).clamp(0., 1.);
        }
        if (curve(x1, x2, s) - t).abs() > 1e-4 {
            let (mut lo, mut hi) = (0f32, 1f32);
            for _ in 0..30 {
                s = (lo + hi) / 2.;
                if curve(x1, x2, s) < t {
                    lo = s;
                } else {
                    hi = s;
                }
            }
        }
        curve(y1, y2, s)
    }
}

#[cfg(test)]
mod tests {
    use super::motion::*;

    #[test]
    fn easings_are_ease_out_and_end_exact() {
        for f in [expo, quart] {
            assert_eq!(f(0.), 0.);
            assert!((f(1.) - 1.).abs() < 1e-4);
            // Ease-out: well past linear early on, and never overshoots (no bounce).
            assert!(f(0.25) > 0.5);
            assert!((0..=100).all(|i| f(i as f32 / 100.) <= 1.0001));
        }
        assert!(expo(0.1) > quart(0.1), "expo leads quart");
    }
}
