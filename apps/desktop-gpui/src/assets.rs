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
        SlidersVertical,
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
/// The empty Timeline's "Start recording" dot (`+page.svelte`: `<circle r="6">`).
pub const REC_DOT: &str = "icons/mnema-rec-dot.svg";

/// The connector catalog's monochrome brand marks (`mcp-presets.ts` `brandSvg`,
/// from the mockup): the spec's own drawings, not Lucide.
pub const BRAND_GITHUB: &str = "icons/mnema-brand-github.svg";
pub const BRAND_LINEAR: &str = "icons/mnema-brand-linear.svg";
pub const BRAND_STRIPE: &str = "icons/mnema-brand-stripe.svg";
pub const BRAND_NOTION: &str = "icons/mnema-brand-notion.svg";
const FILLED: [(&str, &str); 10] = [
    (
        REC_DOT,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="12" r="6"/></svg>"#,
    ),
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
    (
        BRAND_GITHUB,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/></svg>"#,
    ),
    (
        BRAND_LINEAR,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z"/></svg>"#,
    ),
    (
        BRAND_STRIPE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M13.976 9.15c-2.172-.806-3.356-1.426-3.356-2.409 0-.831.683-1.305 1.901-1.305 2.227 0 4.515.858 6.09 1.631l.89-5.494C18.252.975 15.697 0 12.165 0 9.667 0 7.589.654 6.104 1.872 4.56 3.147 3.757 4.992 3.757 7.218c0 4.039 2.467 5.76 6.476 7.219 2.585.92 3.445 1.574 3.445 2.583 0 .98-.84 1.545-2.354 1.545-1.875 0-4.965-.921-6.99-2.109l-.9 5.555C5.175 22.99 8.385 24 11.714 24c2.641 0 4.843-.624 6.328-1.813 1.664-1.305 2.525-3.236 2.525-5.732 0-4.128-2.524-5.851-6.594-7.305h.003z"/></svg>"#,
    ),
    (
        BRAND_NOTION,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.374.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.887l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952L12.21 19s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM1.936 1.035l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.934c-.98.047-1.448-.093-1.962-.747l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V2.667c0-.839.374-1.54 1.447-1.632z"/></svg>"#,
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
