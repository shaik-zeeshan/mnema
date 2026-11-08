//! The Mnema GPUI desktop app. Behaviour lives in `app-core`; this crate holds
//! view state, the theme, and the macOS glue. Conventions: the `gpui-mnema` skill.

pub mod assets;
pub mod chat;
pub mod bridge {
    pub mod events;
    pub mod tokio;
}
pub mod feel;
pub mod gallery;
pub mod host;
pub mod keymap;
pub mod logging;
pub mod native;
pub mod settings;
pub mod shell;
pub mod theme;
pub mod timeline;
pub mod ui;
pub mod window;

/// The GPUI dev app's bundle id (S16 settles release identities).
pub const BUNDLE_ID: &str = "day.mnema.gpui-dev";
