//! The app's asset source: gpui-kit's default icon bundle, the extra Lucide icons
//! the shell uses, and the two surface glyphs the Svelte titlebar draws itself.
//! Every icon is served at stroke 1.75 (the app's icon weight, §3.7), not Lucide's 2.

use std::borrow::Cow;

use gpui_kit::assets::{icon_assets, Assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(ExtraIcons, [MessageCircle, Sparkle, CircleQuestionMark, X]);

/// Path for an icon this source serves outside `IconName` (use with `svg().path(..)`).
pub const TIMELINE_GLYPH: &str = "icons/mnema-timeline.svg";
pub const INSIGHTS_GLYPH: &str = "icons/mnema-insights.svg";

// Exact paths from `lib/shell/Titlebar.svelte` (not in Lucide).
const TIMELINE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><path d="M3 12h18"/><path d="M7 8v8"/><path d="M12 5v14"/><path d="M17 9v6"/></svg>"#;
const INSIGHTS_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><path d="M4 20V10"/><path d="M10 20V4"/><path d="M16 20v-7"/><path d="M22 20H2"/></svg>"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct MnemaAssets;

impl AssetSource for MnemaAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            TIMELINE_GLYPH => return Ok(Some(Cow::Borrowed(TIMELINE_SVG.as_bytes()))),
            INSIGHTS_GLYPH => return Ok(Some(Cow::Borrowed(INSIGHTS_SVG.as_bytes()))),
            _ => {}
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
            [TIMELINE_GLYPH, INSIGHTS_GLYPH]
                .into_iter()
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
