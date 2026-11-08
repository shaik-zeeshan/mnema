//! A few §3.7 tokens, enough for the spike. S5 replaces this with the real `theme.rs`.

use gpui_kit::{rgb, rgba, Hsla};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Clone, Copy)]
pub struct Tokens {
    pub bg: Hsla,
    pub fg: Hsla,
    pub surface: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
}

impl Tokens {
    pub fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self {
                bg: rgb(0x0c0c0e).into(),
                fg: rgb(0xe2e2e8).into(),
                surface: rgb(0x0e0e16).into(),
                border: rgb(0x1e1e2e).into(),
                muted: rgb(0x9696ae).into(),
                accent: rgb(0x3dffa0).into(),
            },
            Mode::Light => Self {
                bg: rgb(0xf6f6f4).into(),
                fg: rgb(0x14141a).into(),
                surface: rgba(0xffffffff).into(),
                border: rgb(0xd8d8d4).into(),
                muted: rgb(0x5a5a6a).into(),
                accent: rgb(0x1f7a4a).into(),
            },
        }
    }
}
