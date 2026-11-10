//! The app's asset source: gpui-kit's default icon bundle, the extra Lucide icons
//! the shell uses, and the two surface glyphs the Svelte titlebar draws itself.
//! Every icon is served at stroke 1.75 (the app's icon weight, §3.7), not Lucide's 2.

use std::borrow::Cow;

use gpui_kit::assets::{icon_assets, Assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    ExtraIcons,
    [
        MessageCircle,
        Sparkle,
        CircleQuestionMark,
        X,
        Download,
        Trash,
        CloudOff,
        Cloud,
        Lock,
        AudioWaveform,
        Feather,
        CircleOff,
        RotateCcw,
        ArrowRightToLine,
        ScanText,
        Monitor,
        Headphones,
        Pin,
        Pencil,
        Laptop,
        Volume2,
        Clock,
        SlidersHorizontal,
        Database,
        Sparkles,
        Terminal,
        Plug,
        Shield,
        Link,
        Speech,
        CornerDownRight,
        List,
        OctagonAlert,
        Power,
        ShieldCheck,
        Minimize2,
        Maximize2,
        RotateCcwClock,
        Code,
        ImageOff,
        ChartNoAxesGantt,
        Speaker,
        Circle,
        CircleDashed,
        Rewind,
        ArrowUpRight,
    ]
);

/// Path for an icon this source serves outside `IconName` (use with `svg().path(..)`).
pub const TIMELINE_GLYPH: &str = "icons/mnema-timeline.svg";
pub const INSIGHTS_GLYPH: &str = "icons/mnema-insights.svg";
/// Lucide `square`, filled: the composer's Stop (`.ch-send--stop rect { fill: currentColor }`).
pub const STOP_SQUARE: &str = "icons/mnema-stop-square.svg";
const STOP_SQUARE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2" fill="currentColor"/></svg>"#;

/// Solid transport glyphs: the status bar's own (`StatusBar.svelte`) and the
/// developer strip's Lucide pause/play drawn solid (`.dv-pause svg g { fill }`).
pub const FILL_PLAY: &str = "icons/mnema-fill-play.svg";
pub const FILL_PAUSE: &str = "icons/mnema-fill-pause.svg";
pub const FILL_STOP: &str = "icons/mnema-fill-stop.svg";
pub const FILL_LUCIDE_PAUSE: &str = "icons/mnema-fill-lucide-pause.svg";
pub const FILL_LUCIDE_PLAY: &str = "icons/mnema-fill-lucide-play.svg";
const FILLED: [(&str, &str); 5] = [
    (
        FILL_PLAY,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M7 5.5v13a1 1 0 0 0 1.5.9l10.4-6.5a1 1 0 0 0 0-1.8L8.5 4.6A1 1 0 0 0 7 5.5z"/></svg>"#,
    ),
    (
        FILL_PAUSE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>"#,
    ),
    (
        FILL_STOP,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>"#,
    ),
    (
        FILL_LUCIDE_PAUSE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="14" y="3" width="5" height="18" rx="1"/><rect x="5" y="3" width="5" height="18" rx="1"/></svg>"#,
    ),
    (
        FILL_LUCIDE_PLAY,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"/></svg>"#,
    ),
];

// Exact paths from `lib/shell/Titlebar.svelte` (not in Lucide).
const TIMELINE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><path d="M3 12h18"/><path d="M7 8v8"/><path d="M12 5v14"/><path d="M17 9v6"/></svg>"#;
const INSIGHTS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><path d="M4 20V10"/><path d="M10 20V4"/><path d="M16 20v-7"/><path d="M22 20H2"/></svg>"#;

/// `.mx-spin`'s lit quarter (the CSS `border-top`) at each spinner size; the
/// track ring is a plain border. Stroke = border width in the 24-unit box.
pub const SPIN_ARC_SM: &str = "icons/mnema-spin-arc-10.svg";
pub const SPIN_ARC: &str = "icons/mnema-spin-arc-14.svg";
pub const SPIN_ARC_LG: &str = "icons/mnema-spin-arc-20.svg";

fn spin_arc(border_px: f32, size_px: f32) -> String {
    let sw = border_px * 24. / size_px;
    let r = 12. - sw / 2.;
    let d = r * std::f32::consts::FRAC_1_SQRT_2;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="{sw}"><path d="M{x1} {y} A{r} {r} 0 0 1 {x2} {y}"/></svg>"#,
        x1 = 12. - d,
        x2 = 12. + d,
        y = 12. - d,
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MnemaAssets;

impl AssetSource for MnemaAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            TIMELINE_GLYPH => return Ok(Some(Cow::Borrowed(TIMELINE_SVG.as_bytes()))),
            INSIGHTS_GLYPH => return Ok(Some(Cow::Borrowed(INSIGHTS_SVG.as_bytes()))),
            STOP_SQUARE => return Ok(Some(Cow::Borrowed(STOP_SQUARE_SVG.as_bytes()))),
            SPIN_ARC_SM => return Ok(Some(Cow::Owned(spin_arc(1.5, 10.).into_bytes()))),
            SPIN_ARC => return Ok(Some(Cow::Owned(spin_arc(1.5, 14.).into_bytes()))),
            SPIN_ARC_LG => return Ok(Some(Cow::Owned(spin_arc(2., 20.).into_bytes()))),
            _ => {}
        }
        if let Some((_, svg)) = FILLED.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        let bytes = match ExtraIcons.load(path)? {
            Some(bytes) => Some(bytes),
            None => Assets.load(path)?,
        };
        Ok(bytes.map(|b| match std::str::from_utf8(&b) {
            Ok(svg) if path.ends_with(".svg") => Cow::Owned(
                svg.replace(r#"stroke-width="2""#, r#"stroke-width="1.75""#)
                    .into(),
            ),
            _ => b,
        }))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = Assets.list(path)?;
        all.extend(ExtraIcons.list(path)?);
        all.extend(
            [
                TIMELINE_GLYPH,
                INSIGHTS_GLYPH,
                STOP_SQUARE,
                SPIN_ARC_SM,
                SPIN_ARC,
                SPIN_ARC_LG,
            ]
            .into_iter()
            .chain(FILLED.iter().map(|(p, _)| *p))
            .filter(|p| p.starts_with(path))
            .map(SharedString::from),
        );
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;

    use super::*;

    #[test]
    fn serves_default_extra_and_glyph_icons_at_stroke_1_75() {
        for path in [
            IconName::Bell.path(),
            IconName::MessageCircle.path(),
            SharedString::from(TIMELINE_GLYPH),
        ] {
            let svg = MnemaAssets.load(&path).unwrap().expect("icon present");
            let svg = std::str::from_utf8(&svg).unwrap();
            assert!(svg.contains(r#"stroke-width="1.75""#), "{path}");
        }
    }
}
