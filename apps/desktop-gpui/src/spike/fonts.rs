//! Embedded UI fonts. GPUI's Core Text loader takes one face per file and applies no
//! variation axes, so a variable TTF only ever yields its default (400) instance.
//! We ship static 400/500/600 cuts made with `fonttools varLib.instancer`.

use std::borrow::Cow;

use gpui_kit::{font, App, FontId, FontWeight};

pub const UI: &str = "Hanken Grotesk";
pub const MONO: &str = "Spline Sans Mono";
pub const WEIGHTS: [FontWeight; 3] = [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::SEMIBOLD];

const STATIC: [&[u8]; 6] = [
    include_bytes!("../../assets/fonts/HankenGrotesk-400.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-500.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-600.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-400.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-500.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-600.ttf"),
];

const VARIABLE: [&[u8]; 2] = [
    include_bytes!("../../assets/fonts/HankenGrotesk-Variable.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-Variable.ttf"),
];

/// `variable_only` exists to prove the variable files alone don't resolve weights.
pub fn register(cx: &mut App, variable_only: bool) -> anyhow::Result<()> {
    let set: &[&'static [u8]] = if variable_only { &VARIABLE } else { &STATIC };
    cx.text_system()
        .add_fonts(set.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
}

/// Font ids for 400/500/600 of `family`, plus the id a missing family falls back to.
pub fn resolved_ids(cx: &App, family: &'static str) -> ([FontId; 3], FontId) {
    let ts = cx.text_system();
    let ids = WEIGHTS.map(|weight| {
        let mut f = font(family);
        f.weight = weight;
        ts.resolve_font(&f)
    });
    (ids, ts.resolve_font(&font("Mnema No Such Family")))
}

/// PASS when the three weights are three distinct faces and none is the fallback.
pub fn weights_resolve(cx: &App, family: &'static str) -> bool {
    let (ids, fallback) = resolved_ids(cx, family);
    ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2] && !ids.contains(&fallback)
}
