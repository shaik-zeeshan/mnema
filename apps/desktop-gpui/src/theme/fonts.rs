//! Embedded UI fonts. GPUI's Core Text loader takes one face per file and applies no
//! variation axes, so a variable TTF only ever yields its default (400) instance.
//! We ship static cuts made with `fonttools varLib.instancer --update-name-table`
//! (S0): 400/500/600 of both, plus mono 300 for Setup's big percentage.

use std::borrow::Cow;

use gpui_kit::{font, App, FontWeight};

use super::scale::{FONT_MONO, FONT_UI};

const STATIC: [&[u8]; 7] = [
    include_bytes!("../../assets/fonts/HankenGrotesk-400.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-500.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-600.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-300.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-400.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-500.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-600.ttf"),
];

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(STATIC.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
}

/// True when every weight we ship resolves to its own face, none the fallback.
pub fn weights_resolve(cx: &App) -> bool {
    let ts = cx.text_system();
    let fallback = ts.resolve_font(&font("Mnema No Such Family"));
    let (regular, medium, semibold) =
        (FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::SEMIBOLD);
    [
        (FONT_UI, vec![regular, medium, semibold]),
        (
            FONT_MONO,
            vec![FontWeight::LIGHT, regular, medium, semibold],
        ),
    ]
    .into_iter()
    .all(|(family, weights)| {
        let ids: Vec<_> = weights
            .into_iter()
            .map(|weight| {
                let mut f = font(family);
                f.weight = weight;
                ts.resolve_font(&f)
            })
            .collect();
        let distinct = ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id));
        distinct && !ids.contains(&fallback)
    })
}

/// `ui::tracking` matches CSS `letter-spacing`: a tracked line is wider by
/// chars × em × size (the space follows every character, the last included, and
/// font kerning is kept); the wrapper's per-character widths and `ch_advance`
/// agree with it. The error names what is off.
pub fn tracking_measures(window: &gpui_kit::Window) -> Result<(), String> {
    use crate::ui::tracking::with_tracking;
    use gpui_kit::{black, px, FontFeatures, SharedString, TextRun};
    const TEXT: &str = "AVATAR To Wo 0123";
    let size = px(20.);
    let ts = window.text_system();
    let line = |f: &gpui_kit::Font| -> f32 {
        let run = TextRun {
            len: TEXT.len(),
            font: f.clone(),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            ts.shape_line(SharedString::from(TEXT), size, &[run], None)
                .width,
        )
    };
    let chars = |f: &gpui_kit::Font| -> f32 {
        let id = ts.resolve_font(f);
        TEXT.chars()
            .map(|c| f32::from(ts.layout_width(id, size, c)))
            .sum()
    };
    let ch =
        |f: &gpui_kit::Font| f32::from(ts.ch_advance(ts.resolve_font(f), size).unwrap_or_default());
    for family in [FONT_UI, FONT_MONO] {
        for em in [0.12f32, -0.03] {
            let plain = font(family);
            let mut tracked = font(family);
            tracked.features = with_tracking(&FontFeatures::default(), em);
            let want = TEXT.chars().count() as f32 * em * f32::from(size);
            for (what, got, want) in [
                ("line", line(&tracked) - line(&plain), want),
                ("wrapper widths", chars(&tracked) - chars(&plain), want),
                (
                    "ch_advance",
                    ch(&tracked) - ch(&plain),
                    em * f32::from(size),
                ),
            ] {
                if (got - want).abs() > 0.05 {
                    return Err(format!(
                        "{family} {em}em: {what} +{got:.3}px, want +{want:.3}px"
                    ));
                }
            }
        }
    }
    Ok(())
}
