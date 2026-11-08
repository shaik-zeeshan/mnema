//! Embedded UI fonts. GPUI's Core Text loader takes one face per file and applies no
//! variation axes, so a variable TTF only ever yields its default (400) instance.
//! We ship static 400/500/600 cuts made with `fonttools varLib.instancer` (S0).

use std::borrow::Cow;

use gpui_kit::{font, App, FontWeight};

use super::scale::{FONT_MONO, FONT_UI};

const STATIC: [&[u8]; 6] = [
    include_bytes!("../../assets/fonts/HankenGrotesk-400.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-500.ttf"),
    include_bytes!("../../assets/fonts/HankenGrotesk-600.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-400.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-500.ttf"),
    include_bytes!("../../assets/fonts/SplineSansMono-600.ttf"),
];

pub fn register(cx: &mut App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(STATIC.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
}

/// True when 400/500/600 of both families are three distinct faces, none the fallback.
pub fn weights_resolve(cx: &App) -> bool {
    let ts = cx.text_system();
    let fallback = ts.resolve_font(&font("Mnema No Such Family"));
    [FONT_UI, FONT_MONO].into_iter().all(|family| {
        let ids = [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::SEMIBOLD].map(|weight| {
            let mut f = font(family);
            f.weight = weight;
            ts.resolve_font(&f)
        });
        ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2] && !ids.contains(&fallback)
    })
}
