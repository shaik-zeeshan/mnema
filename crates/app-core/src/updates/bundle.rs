//! The running `.app` (its identity and where it lives), unpacking a verified
//! update archive, and the atomic bundle swap.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use core_foundation::bundle::CFBundle;
use core_foundation::string::CFString;

/// What the running process's main bundle says about itself. Outside a `.app`
/// (`cargo run`, tests) every field is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MainBundle {
    pub identifier: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    /// The `.app` directory, only when the process runs from one.
    pub path: Option<PathBuf>,
}

impl MainBundle {
    pub fn read() -> Self {
        let bundle = CFBundle::main_bundle();
        let info = bundle.info_dictionary();
        let get = |key: &'static str| {
            info.find(CFString::from_static_string(key))
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        };
        let path = bundle
            .path()
            .filter(|p| p.extension().is_some_and(|e| e == "app"));
        Self {
            identifier: get("CFBundleIdentifier"),
            name: get("CFBundleName"),
            version: get("CFBundleShortVersionString"),
            path,
        }
    }
}

/// Unpacks a (signature-verified) `.app.tar.gz` into `staging` and returns the
/// one `.app` at its top level. Entries can't land outside `staging`
/// (`unpack_in`), and AppleDouble `._*` entries a macOS `tar` adds for extended
/// attributes are skipped: written as files they'd break the bundle's seal.
pub fn extract_app(archive: &[u8], staging: &Path) -> io::Result<PathBuf> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let apple_double = entry
            .path()?
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("._"));
        if !apple_double {
            entry.unpack_in(staging)?;
        }
    }
    let apps: Vec<PathBuf> = fs::read_dir(staging)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.extension().is_some_and(|e| e == "app"))
        .collect();
    match apps.as_slice() {
        [app] => Ok(app.clone()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the update archive must hold exactly one .app",
        )),
    }
}

/// Puts `new_app` where `installed` is. The old bundle is renamed to `aside`
/// first and renamed back if the move-in fails, so `installed` always holds
/// a whole app, old or new. All three paths must be on one volume (renames).
/// The caller deletes `aside` (it lives in the staging dir).
pub fn replace_bundle(installed: &Path, new_app: &Path, aside: &Path) -> io::Result<()> {
    fs::rename(installed, aside)?;
    if let Err(error) = fs::rename(new_app, installed) {
        if let Err(restore) = fs::rename(aside, installed) {
            log::error!(
                "update swap failed and the previous app could not be restored to {}: {restore}",
                installed.display()
            );
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_app(root: &Path, name: &str, marker: &str) -> PathBuf {
        let app = root.join(name);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/bin"), marker).unwrap();
        app
    }

    fn marker(app: &Path) -> String {
        fs::read_to_string(app.join("Contents/MacOS/bin")).unwrap()
    }

    /// A `.app.tar.gz` of `dir`'s `.app`, as `tar -czf x.tar.gz -C dir Mnema.app`.
    pub(crate) fn archive_of(app: &Path) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        tar.append_dir_all(app.file_name().unwrap(), app).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn replace_swaps_the_new_app_in() {
        let dir = tempfile::tempdir().unwrap();
        let installed = write_app(dir.path(), "Mnema.app", "old");
        let staging = dir.path().join("stage");
        fs::create_dir(&staging).unwrap();
        let new_app = write_app(&staging, "Mnema.app", "new");
        replace_bundle(&installed, &new_app, &staging.join("previous.app")).unwrap();
        assert_eq!(marker(&installed), "new");
        assert_eq!(marker(&staging.join("previous.app")), "old");
        assert!(!new_app.exists());
    }

    #[test]
    fn a_failed_move_in_restores_the_old_app() {
        let dir = tempfile::tempdir().unwrap();
        let installed = write_app(dir.path(), "Mnema.app", "old");
        let aside = dir.path().join("previous.app");
        // The new app is missing, so the move-in fails after the old one moved aside.
        let err = replace_bundle(&installed, &dir.path().join("gone.app"), &aside).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(marker(&installed), "old");
        assert!(!aside.exists());
    }

    #[test]
    fn a_failed_move_aside_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let installed = write_app(dir.path(), "Mnema.app", "old");
        let new_app = write_app(dir.path(), "New.app", "new");
        let aside = dir.path().join("no/such/dir/previous.app");
        assert!(replace_bundle(&installed, &new_app, &aside).is_err());
        assert_eq!(marker(&installed), "old");
        assert_eq!(marker(&new_app), "new");
    }

    #[test]
    fn extract_returns_the_one_app_and_skips_apple_double() {
        let src = tempfile::tempdir().unwrap();
        let app = write_app(src.path(), "Mnema.app", "v2");
        fs::write(app.join("Contents/._Info.plist"), "xattrs").unwrap();
        let staging = tempfile::tempdir().unwrap();
        let out = extract_app(&archive_of(&app), staging.path()).unwrap();
        assert_eq!(out, staging.path().join("Mnema.app"));
        assert_eq!(marker(&out), "v2");
        assert!(!out.join("Contents/._Info.plist").exists());
    }

    #[test]
    fn an_archive_without_an_app_is_refused() {
        let src = tempfile::tempdir().unwrap();
        let not_app = write_app(src.path(), "payload", "x");
        let staging = tempfile::tempdir().unwrap();
        let err = extract_app(&archive_of(&not_app), staging.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
