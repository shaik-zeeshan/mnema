//! Where an app instance keeps its files, from its bundle id plus the existing
//! `MNEMA_*` overrides (the same ones app-infra, the CLI and `dev-app.sh` use).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Default capture root (DB + recordings): `MNEMA_SAVE_DIRECTORY`, else
    /// `~/.mnema`. A `save_directory` persisted in recording settings wins at runtime.
    pub data: PathBuf,
    /// App config + model files: `MNEMA_APP_CONFIG_DIR`, else
    /// `~/Library/Application Support/<bundle id>`.
    pub config: PathBuf,
    /// `~/Library/Caches/<bundle id>`.
    pub cache: PathBuf,
    /// `~/Library/Logs/<bundle id>` (holds `rust.log`).
    pub log: PathBuf,
}

impl Paths {
    /// Resolve against the real home dir and process environment.
    pub fn from_env(bundle_id: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::resolve(bundle_id, &home, |key| std::env::var(key).ok())
    }

    /// Env values are taken verbatim (no trimming, no `~` expansion), matching
    /// `default_save_directory` and app-infra's `default_app_config_dir`.
    pub fn resolve(bundle_id: &str, home: &Path, env: impl Fn(&str) -> Option<String>) -> Self {
        let library = home.join("Library");
        Self {
            data: env("MNEMA_SAVE_DIRECTORY")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".mnema")),
            config: env("MNEMA_APP_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| library.join("Application Support").join(bundle_id)),
            cache: library.join("Caches").join(bundle_id),
            log: library.join("Logs").join(bundle_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn tauri_dev_sandbox_layout_matches_dev_app_sh() {
        let home = Path::new("/Users/ana");
        let support = "/Users/ana/Library/Application Support/day.mnema.dev".to_string();
        let env = [
            ("MNEMA_SAVE_DIRECTORY", "/Users/ana/.mnema-dev".to_string()),
            ("MNEMA_APP_CONFIG_DIR", support.clone()),
        ];
        let paths = Paths::resolve("day.mnema.dev", home, env_of(&env));
        assert_eq!(paths.data, PathBuf::from("/Users/ana/.mnema-dev"));
        assert_eq!(paths.config, PathBuf::from(support));
        assert_eq!(
            paths.cache,
            PathBuf::from("/Users/ana/Library/Caches/day.mnema.dev")
        );
        assert_eq!(
            paths.log,
            PathBuf::from("/Users/ana/Library/Logs/day.mnema.dev")
        );
    }

    #[test]
    fn gpui_dev_layout_uses_its_own_identity_and_data_dir() {
        let home = Path::new("/Users/ana");
        let env = [(
            "MNEMA_SAVE_DIRECTORY",
            "/Users/ana/.mnema-gpui-dev".to_string(),
        )];
        let paths = Paths::resolve("day.mnema.gpui-dev", home, env_of(&env));
        assert_eq!(paths.data, PathBuf::from("/Users/ana/.mnema-gpui-dev"));
        assert_eq!(
            paths.config,
            PathBuf::from("/Users/ana/Library/Application Support/day.mnema.gpui-dev")
        );
        assert_eq!(
            paths.cache,
            PathBuf::from("/Users/ana/Library/Caches/day.mnema.gpui-dev")
        );
        assert_eq!(
            paths.log,
            PathBuf::from("/Users/ana/Library/Logs/day.mnema.gpui-dev")
        );
    }

    #[test]
    fn production_layout_without_overrides() {
        let paths = Paths::resolve("day.mnema", Path::new("/Users/ana"), |_| None);
        assert_eq!(paths.data, PathBuf::from("/Users/ana/.mnema"));
        assert_eq!(
            paths.config,
            PathBuf::from("/Users/ana/Library/Application Support/day.mnema")
        );
    }
}
