//! Every colour token, dark and light, ported verbatim from
//! `apps/desktop/src/routes/+layout.svelte` (`:431` dark, `:626` light; charts
//! `:594` / `:761`) plus kit.css's derived fills. Field = the CSS name without
//! `--app-`, in snake case (`--app-text-muted` -> `text_muted`).

use gpui_kit::{rgb, Hsla, Rgba};

/// `#rrggbb` at alpha `a` (CSS `color-mix(in srgb, X a%, transparent)`).
fn a(hex: u32, alpha: f32) -> Hsla {
    let mut c: Hsla = rgb(hex).into();
    c.a = alpha;
    c
}

fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (f, o) = (Rgba::from(from), Rgba::from(to));
    let l = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: l(f.r, o.r),
        g: l(f.g, o.g),
        b: l(f.b, o.b),
        a: l(f.a, o.a),
    }
    .into()
}

macro_rules! palette {
    ($( $name:ident: $dark:expr, $light:expr; )*) => {
        /// One mode's colours. Views read them through `theme::tokens(cx)`.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette { $(pub $name: Hsla,)* }

        impl Palette {
            pub fn dark() -> Self { Self { $($name: Hsla::from($dark),)* } }
            pub fn light() -> Self { Self { $($name: Hsla::from($light),)* } }
            /// Per-channel sRGB blend, `t` in 0..=1: the theme cross-fade.
            pub fn lerp(&self, to: &Self, t: f32) -> Self {
                if t <= 0. { return *self; }
                if t >= 1. { return *to; }
                Self { $($name: mix(self.$name, to.$name, t),)* }
            }
        }
    };
}

palette! {
    bg: rgb(0x0c0c0e), rgb(0xf6f6f4);
    fg: rgb(0xe2e2e8), rgb(0x14141a);
    fg_muted: rgb(0x8a8aaa), rgb(0x5a5a6a);
    fg_subtle: rgb(0x45455a), rgb(0x8a8a9a);

    titlebar_bg: rgb(0x08080c), rgb(0xececea);
    titlebar_border: rgb(0x15151f), rgb(0xd4d4d2);
    titlebar_title: rgb(0x45455a), rgb(0x9a9aa8);

    status_bg: rgb(0x0a0a10), rgb(0xffffff);
    status_border: rgb(0x161624), rgb(0xd8d8dc);
    status_fg: rgb(0x6f6f90), rgb(0x5a5a6a);
    status_dot: rgb(0x2a2a3a), rgb(0xc4c4cc);
    status_running_fg: rgb(0xff5d6c), rgb(0xc81d2e);
    status_running_border: rgb(0x3a1820), rgb(0xf1b9bf);
    status_running_dot: rgb(0xff3148), rgb(0xd62236);
    status_running_dot_glow: a(0xff3148, 0.18), a(0xd62236, 0.22);
    status_paused_fg: rgb(0xd6a14a), rgb(0x8a5a10);
    status_paused_border: rgb(0x3a2818), rgb(0xecd9b0);
    status_paused_dot: rgb(0xd6a14a), rgb(0xc08018);
    status_paused_dot_glow: a(0xd6a14a, 0.16), a(0xc08018, 0.22);

    record_start_bg: rgb(0x1a0f12), rgb(0xffffff);
    record_start_fg: rgb(0xff8a96), rgb(0xc81d2e);
    record_start_border: rgb(0x3a1820), rgb(0xecbcc2);
    record_start_bg_hover: rgb(0x2a1218), rgb(0xfff0f2);
    record_start_fg_hover: rgb(0xffb0b9), rgb(0xa01624);
    record_start_border_hover: rgb(0x5a2030), rgb(0xd68c95);
    record_stop_bg: rgb(0x170d0f), rgb(0xc81d2e);
    record_stop_fg: rgb(0xf0f0f5), rgb(0xffffff);
    record_stop_border: rgb(0x4a1c26), rgb(0xa01624);
    record_stop_bg_hover: rgb(0x2a1218), rgb(0xa01624);
    record_stop_border_hover: rgb(0x6a2434), rgb(0x7a1019);
    record_glyph_start: rgb(0xff3148), rgb(0xc81d2e);
    record_glyph_stop: rgb(0xff8a96), rgb(0xffffff);

    icon_fg: rgb(0x8a8aaa), rgb(0x5a5a6a);
    icon_fg_hover: rgb(0xe2e2e8), rgb(0x14141a);
    icon_bg_hover: rgb(0x1a1a2a), rgb(0xe2e2e0);
    icon_border_hover: rgb(0x2a2a3a), rgb(0xc8c8c6);
    icon_bg_active: rgb(0x14141f), rgb(0xdcdcda);
    icon_border_active: rgb(0x2a2a3a), rgb(0xb8b8b6);

    surface: rgb(0x0e0e16), rgb(0xffffff);
    surface_subtle: rgb(0x101018), rgb(0xf6f6f4);
    surface_raised: rgb(0x13131a), rgb(0xfbfbfa);
    surface_hover: rgb(0x1a1a2a), rgb(0xeeeeec);
    surface_active: rgb(0x131320), rgb(0xe8f1ea);
    border: rgb(0x1e1e2e), rgb(0xd8d8d4);
    border_strong: rgb(0x2a2a3a), rgb(0xc4c4c0);
    border_hover: rgb(0x3a3a5a), rgb(0xa4a4a0);
    text_strong: rgb(0xe2e2e8), rgb(0x14141a);
    text: rgb(0xc0c0d0), rgb(0x2a2a32);
    text_muted: rgb(0x9696ae), rgb(0x5a5a6a);
    text_subtle: rgb(0x7e7e98), rgb(0x5e5e6a);
    text_faint: rgb(0x33334a), rgb(0x9a9aa4);
    accent: rgb(0x3dffa0), rgb(0x1f7a4a);
    accent_strong: rgb(0x2a8a60), rgb(0x155a36);
    accent_bg: rgb(0x0d1f15), rgb(0xe6f4ec);
    accent_border: rgb(0x1a4a30), rgb(0x9bd3b4);
    accent_glow: a(0x3dffa0, 0.18), a(0x1f7a4a, 0.16);
    accent_contrast: rgb(0x07120c), rgb(0xffffff);

    warn: rgb(0xd6a14a), rgb(0x9a5a12);
    warn_strong: rgb(0xc47a30), rgb(0x7f4300);
    warn_bg: rgb(0x1a1208), rgb(0xfff1df);
    warn_border: rgb(0x7a4a18), rgb(0xdfbc8a);
    danger: rgb(0xff6b7a), rgb(0xc43a48);
    danger_strong: rgb(0xff4455), rgb(0xb42332);
    danger_bg: rgb(0x2e0f14), rgb(0xfff0f2);
    danger_bg_soft: rgb(0x0e0a0a), rgb(0xfff6f7);
    danger_border: rgb(0x4a1a20), rgb(0xe4b6be);
    danger_text: rgb(0xff8090), rgb(0xd24a59);
    info: rgb(0x60b0ff), rgb(0x2b78c5);
    info_strong: rgb(0x4a6aaa), rgb(0x225fa3);
    info_bg: rgb(0x0c1a2e), rgb(0xeef5ff);
    info_border: rgb(0x1a3050), rgb(0xbdd3ef);
    neutral_bg: rgb(0x1a1a2a), rgb(0xf2f3f6);
    neutral_border: rgb(0x2a2a3a), rgb(0xd5d7de);
    neutral_text: rgb(0x7070a0), rgb(0x636a79);

    source_screen: rgb(0xc0b0ff), rgb(0x6f5ed1);
    source_screen_strong: rgb(0x5a4aaa), rgb(0x5949b8);
    source_screen_bg: rgb(0x1a1a3a), rgb(0xf1edff);
    source_screen_border: rgb(0x2a2a5a), rgb(0xcdc3f2);
    source_mic: rgb(0x80d0a8), rgb(0x2f8e59);
    source_mic_strong: rgb(0x4a8a6a), rgb(0x287a4a);
    source_mic_bg: rgb(0x0f2e1f), rgb(0xe8f5ec);
    source_mic_border: rgb(0x1a4a30), rgb(0xafd8bf);
    source_sysaudio: rgb(0xb0c080), rgb(0x8b7a2c);
    source_sysaudio_strong: rgb(0x6a7a4a), rgb(0x786821);
    source_sysaudio_bg: rgb(0x2a2010), rgb(0xfaf4df);
    source_sysaudio_border: rgb(0x4a3a18), rgb(0xdbc98a);

    overlay_bg: a(0x0a0a10, 0.78), a(0xffffff, 0.78);
    overlay_bg_strong: a(0x0a0a10, 0.82), a(0xffffff, 0.86);
    overlay_border: a(0xffffff, 0.06), a(0x141820, 0.12);
    input_recess: a(0x000000, 0.25), a(0x000000, 0.08);
    shadow_popover: a(0x000000, 0.22), a(0x000000, 0.22);

    ocr_box: a(0x78dca0, 0.45), a(0x1f7a4a, 0.42);
    ocr_box_hover: a(0x78dca0, 0.95), a(0x1f7a4a, 0.88);
    ocr_box_fill: a(0x78dca0, 0.10), a(0x000000, 0.0);
    ocr_chip_bg: a(0x080e0a, 0.96), a(0xffffff, 0.92);
    ocr_chip_text: rgb(0xeaffef), rgb(0x155a36);
    ocr_chip_border: a(0x78dca0, 0.6), a(0x1f7a4a, 0.24);
    ocr_hover_shadow: a(0x000000, 0.45), a(0x151c26, 0.18);
    ocr_hover_inset: a(0xffffff, 0.04), a(0x000000, 0.0);

    chart_grey_1: rgb(0x2c2c3a), rgb(0xd8d8de);
    chart_grey_2: rgb(0x3e3e50), rgb(0xb6b6c0);
    chart_grey_3: rgb(0x565669), rgb(0x909099);
    chart_grey_4: rgb(0x757589), rgb(0x6a6a74);
    chart_grey_5: rgb(0x9a9ab0), rgb(0x46464e);
    cat_creating: rgb(0x5fe07a), rgb(0x2f8a3f);
    cat_communication: rgb(0xc0b0ff), rgb(0x5949b8);
    cat_meetings: rgb(0xff9fd0), rgb(0xc2407f);
    cat_research: rgb(0x60b0ff), rgb(0x2b78c5);
    cat_learning: rgb(0x4fd8c8), rgb(0x1f8579);
    cat_organizing: rgb(0xb0c080), rgb(0x6f7a2e);
    cat_personal: rgb(0xd6a14a), rgb(0x9a5a12);
    cat_entertainment: rgb(0xff7a4d), rgb(0xc2542b);
    focus_deep: rgb(0x3dffa0), rgb(0x1f7a4a);
    focus_mid: rgb(0xd6a14a), rgb(0x9a5a12);
    focus_distracted: rgb(0xff6b7a), rgb(0xc43a48);

    // kit.css derived fills: hairline = border 80%; wash = fg 4% (5% light);
    // wash-strong = fg 8% (9% light) = the selected-row fill. No left accent bars.
    hairline: a(0x1e1e2e, 0.8), a(0xd8d8d4, 0.8);
    wash: a(0xe2e2e8, 0.04), a(0x14141a, 0.05);
    wash_strong: a(0xe2e2e8, 0.08), a(0x14141a, 0.09);
    titlebar_glass: a(0x08080c, 0.82), a(0xececea, 0.82);
    // kit: raised 96% + blur 22; see `POP_SURFACE_ALPHA` (GPUI can't blur).
    pop_bg: a(0x13131a, super::scale::POP_SURFACE_ALPHA), a(0xfbfbfa, super::scale::POP_SURFACE_ALPHA);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_ends_on_each_palette_and_mixes_between() {
        let (d, l) = (Palette::dark(), Palette::light());
        assert_eq!(d.lerp(&l, 0.0), d);
        let end = d.lerp(&l, 1.0);
        let close = |x: Hsla, y: Hsla| {
            let (x, y) = (Rgba::from(x), Rgba::from(y));
            (x.r - y.r).abs() < 1e-3 && (x.g - y.g).abs() < 1e-3 && (x.b - y.b).abs() < 1e-3
        };
        assert!(close(end.bg, l.bg) && close(end.accent, l.accent));
        let mid = Rgba::from(d.lerp(&l, 0.5).bg);
        let (rd, rl) = (Rgba::from(d.bg), Rgba::from(l.bg));
        assert!((mid.r - (rd.r + rl.r) / 2.).abs() < 1e-3);
    }

    #[test]
    fn spot_check_section_3_7_values() {
        let hex = |c: Hsla| {
            let c = Rgba::from(c);
            ((c.r * 255.).round() as u32) << 16
                | ((c.g * 255.).round() as u32) << 8
                | (c.b * 255.).round() as u32
        };
        let (d, l) = (Palette::dark(), Palette::light());
        assert_eq!(hex(d.surface_hover), 0x1a1a2a);
        assert_eq!(hex(l.border_hover), 0xa4a4a0);
        assert_eq!(hex(d.source_sysaudio), 0xb0c080);
        assert_eq!(hex(l.status_paused_dot), 0xc08018);
        assert!((d.accent_glow.a - 0.18).abs() < 1e-6);
    }
}
