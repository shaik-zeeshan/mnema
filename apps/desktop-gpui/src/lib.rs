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
pub mod insights;
pub mod keymap;
pub mod logging;
pub mod native;
pub mod quick_recall;
pub mod onboarding;
pub mod settings;
pub mod shell;
pub mod small;
pub mod theme;
pub mod timeline;
pub mod ui;
pub mod window;

/// The GPUI dev app's bundle id, used when the binary runs outside a bundle.
/// A bundle's own id wins (`bundle-gpui-app.sh`: `day.mnema` for --release).
pub const BUNDLE_ID: &str = "day.mnema.gpui-dev";
