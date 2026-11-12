//! Installing the bundled `mnema` CLI on the user's PATH (a symlink in
//! `~/.local/bin`) and reporting whether it is installed.

#[cfg(not(windows))]
use std::sync::{Mutex, OnceLock};
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

const MNEMA_CLI_COMMAND_NAME: &str = "mnema";
const MNEMA_CLI_SIDECAR_NAME: &str = "mnema-cli";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MnemaCliStatus {
    pub install_path: String,
    pub install_dir: String,
    pub bundled_cli_path: String,
    pub bundled_cli_exists: bool,
    pub installed: bool,
    pub install_dir_in_path: bool,
    pub existing_target: Option<String>,
}

fn mnema_cli_sidecar_name() -> String {
    #[cfg(windows)]
    {
        format!("{MNEMA_CLI_SIDECAR_NAME}.exe")
    }
    #[cfg(not(windows))]
    {
        MNEMA_CLI_SIDECAR_NAME.to_string()
    }
}

fn mnema_cli_sidecar_name_for_target_triple(target_triple: &str) -> String {
    #[cfg(windows)]
    {
        format!("{MNEMA_CLI_SIDECAR_NAME}-{target_triple}.exe")
    }
    #[cfg(not(windows))]
    {
        format!("{MNEMA_CLI_SIDECAR_NAME}-{target_triple}")
    }
}

fn current_target_triple_candidates() -> Vec<String> {
    let mut candidates = Vec::new();

    for key in ["CARGO_BUILD_TARGET", "TAURI_ENV_TARGET_TRIPLE", "TARGET"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() && !candidates.contains(&value) {
                candidates.push(value);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let arch = std::env::consts::ARCH;
        let host_triple = format!("{arch}-apple-darwin");
        if !candidates.contains(&host_triple) {
            candidates.push(host_triple);
        }
        let universal_triple = "universal-apple-darwin".to_string();
        if !candidates.contains(&universal_triple) {
            candidates.push(universal_triple);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let env = if cfg!(target_env = "gnu") {
            "pc-windows-gnu"
        } else {
            "pc-windows-msvc"
        };
        let host_triple = format!("{}-{env}", std::env::consts::ARCH);
        if !candidates.contains(&host_triple) {
            candidates.push(host_triple);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let env = if cfg!(target_env = "musl") {
            "unknown-linux-musl"
        } else {
            "unknown-linux-gnu"
        };
        let host_triple = format!("{}-{env}", std::env::consts::ARCH);
        if !candidates.contains(&host_triple) {
            candidates.push(host_triple);
        }
    }

    candidates
}

fn bundled_mnema_cli_path_in_dir(exe_dir: &Path) -> PathBuf {
    let default_path = exe_dir.join(mnema_cli_sidecar_name());
    if default_path.is_file() {
        return default_path;
    }

    for target_triple in current_target_triple_candidates() {
        let candidate = exe_dir.join(mnema_cli_sidecar_name_for_target_triple(&target_triple));
        if candidate.is_file() {
            return candidate;
        }
    }

    default_path
}

fn bundled_mnema_cli_path() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current exe: {error}"))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| format!("current exe has no parent: {}", current_exe.display()))?;
    Ok(bundled_mnema_cli_path_in_dir(exe_dir))
}

fn mnema_cli_install_path() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir().ok_or_else(|| "failed to resolve home dir".to_string())?;
    Ok(mnema_cli_install_path_for_home(&home_dir))
}

#[cfg(windows)]
fn mnema_cli_install_path_for_home(home_dir: &Path) -> PathBuf {
    home_dir
        .join("AppData")
        .join("Local")
        .join("Microsoft")
        .join("WindowsApps")
        .join(format!("{MNEMA_CLI_COMMAND_NAME}.exe"))
}

#[cfg(not(windows))]
fn mnema_cli_install_path_for_home(home_dir: &Path) -> PathBuf {
    home_dir
        .join(".local")
        .join("bin")
        .join(MNEMA_CLI_COMMAND_NAME)
}

// Resolving the login-shell PATH spawns the user's full profile (100ms-1s). PATH is
// stable for the life of the process under normal use (nothing in this tree calls
// std::env::set_var), so the resolved dirs are memoized once and cloned on later
// calls. Used by Mnema CLI install-dir-on-PATH detection.
#[cfg(not(windows))]
static TERMINAL_SHELL_PATH_DIRS_CACHE: OnceLock<Mutex<Option<Vec<PathBuf>>>> = OnceLock::new();

#[cfg(not(windows))]
fn compute_terminal_shell_path_dirs() -> Vec<PathBuf> {
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let shell_path = std::process::Command::new(shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());

    let mut path_dirs: Vec<PathBuf> = shell_path
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect();
    if let Some(process_path) = std::env::var_os("PATH") {
        path_dirs.extend(std::env::split_paths(&process_path));
    }
    path_dirs
}

#[cfg(not(windows))]
fn terminal_shell_path_dirs() -> Vec<PathBuf> {
    let cache = TERMINAL_SHELL_PATH_DIRS_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .get_or_insert_with(compute_terminal_shell_path_dirs)
        .clone()
}

#[cfg(windows)]
fn terminal_shell_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

fn resolve_link_target(link_path: &Path, target: PathBuf) -> PathBuf {
    if target.is_absolute() {
        target
    } else {
        link_path
            .parent()
            .map(|parent| parent.join(&target))
            .unwrap_or(target)
    }
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn existing_cli_symlink_is_safe_to_replace(
    existing_target: &Path,
    bundled_cli_path: &Path,
) -> bool {
    paths_refer_to_same_file(existing_target, bundled_cli_path)
}

#[cfg(windows)]
fn files_have_same_contents(left: &Path, right: &Path) -> bool {
    let Ok(left_metadata) = fs::metadata(left) else {
        return false;
    };
    let Ok(right_metadata) = fs::metadata(right) else {
        return false;
    };
    left_metadata.is_file()
        && right_metadata.is_file()
        && left_metadata.len() == right_metadata.len()
        && fs::read(left)
            .ok()
            .zip(fs::read(right).ok())
            .is_some_and(|(left, right)| left == right)
}

fn path_dir_in_shell_path(dir: &Path) -> bool {
    terminal_shell_path_dirs()
        .into_iter()
        .any(|entry| paths_refer_to_same_file(&entry, dir))
}

fn mnema_cli_status_for_paths(install_path: PathBuf, bundled_cli_path: PathBuf) -> MnemaCliStatus {
    let existing_target = fs::read_link(&install_path)
        .ok()
        .map(|target| resolve_link_target(&install_path, target));
    let symlink_installed = existing_target
        .as_deref()
        .is_some_and(|target| paths_refer_to_same_file(target, &bundled_cli_path));
    #[cfg(windows)]
    let installed = symlink_installed || files_have_same_contents(&install_path, &bundled_cli_path);
    #[cfg(not(windows))]
    let installed = symlink_installed;
    let install_dir_in_path = install_path.parent().is_some_and(path_dir_in_shell_path);

    MnemaCliStatus {
        install_dir: install_path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        install_path: install_path.display().to_string(),
        bundled_cli_exists: bundled_cli_path.is_file(),
        bundled_cli_path: bundled_cli_path.display().to_string(),
        installed,
        install_dir_in_path,
        existing_target: existing_target.map(|path| path.display().to_string()),
    }
}

#[cfg(unix)]
fn create_mnema_cli_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_mnema_cli_link(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, destination)
        .or_else(|_| fs::copy(source, destination).map(|_| ()))
}

pub async fn get_cli_status_inner() -> Result<MnemaCliStatus, String> {
    let install_path = mnema_cli_install_path()?;
    let bundled_cli_path = bundled_mnema_cli_path()?;
    Ok(mnema_cli_status_for_paths(install_path, bundled_cli_path))
}

/// Links the bundled CLI onto the PATH, on the blocking pool (filesystem work).
pub async fn install_cli_inner() -> Result<MnemaCliStatus, String> {
    tokio::task::spawn_blocking(install_cli)
        .await
        .map_err(|error| format!("CLI install task failed: {error}"))?
}

fn install_cli() -> Result<MnemaCliStatus, String> {
    let install_path = mnema_cli_install_path()?;
    let bundled_cli_path = bundled_mnema_cli_path()?;

    if !bundled_cli_path.is_file() {
        return Err(format!(
            "bundled Mnema CLI is missing at {}",
            bundled_cli_path.display()
        ));
    }

    let install_dir = install_path
        .parent()
        .ok_or_else(|| format!("CLI install path has no parent: {}", install_path.display()))?;
    fs::create_dir_all(install_dir).map_err(|error| {
        format!(
            "failed to create CLI install directory {}: {error}",
            install_dir.display()
        )
    })?;

    if let Ok(metadata) = fs::symlink_metadata(&install_path) {
        if metadata.file_type().is_symlink() {
            let existing_target = fs::read_link(&install_path)
                .map(|target| resolve_link_target(&install_path, target))
                .map_err(|error| {
                    format!(
                        "failed to inspect existing CLI symlink {}: {error}",
                        install_path.display()
                    )
                })?;
            if !existing_cli_symlink_is_safe_to_replace(&existing_target, &bundled_cli_path) {
                return Err(format!(
                    "refusing to overwrite existing CLI symlink {} -> {}",
                    install_path.display(),
                    existing_target.display()
                ));
            }
            fs::remove_file(&install_path).map_err(|error| {
                format!(
                    "failed to replace existing CLI symlink {}: {error}",
                    install_path.display()
                )
            })?;
        } else {
            #[cfg(windows)]
            if metadata.is_file() && files_have_same_contents(&install_path, &bundled_cli_path) {
                fs::remove_file(&install_path).map_err(|error| {
                    format!(
                        "failed to replace existing CLI file {}: {error}",
                        install_path.display()
                    )
                })?;
            } else {
                return Err(format!(
                    "refusing to overwrite existing non-symlink at {}",
                    install_path.display()
                ));
            }
            #[cfg(not(windows))]
            return Err(format!(
                "refusing to overwrite existing non-symlink at {}",
                install_path.display()
            ));
        }
    }

    create_mnema_cli_link(&bundled_cli_path, &install_path).map_err(|error| {
        format!(
            "failed to link {} to {}: {error}",
            install_path.display(),
            bundled_cli_path.display()
        )
    })?;

    Ok(mnema_cli_status_for_paths(install_path, bundled_cli_path))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("desktop-tauri-{label}-{unique}"));

            fs::create_dir_all(&path).expect("test directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn mnema_cli_install_path_for_home_uses_user_local_bin() {
        assert_eq!(
            mnema_cli_install_path_for_home(Path::new("/Users/tester")),
            PathBuf::from("/Users/tester/.local/bin/mnema")
        );
    }

    #[test]
    fn bundled_mnema_cli_path_uses_plain_sidecar_name_when_present() {
        let dir = TestDir::new("mnema-cli-plain-sidecar");
        let plain_cli = dir.path().join(mnema_cli_sidecar_name());
        let target_cli = dir.path().join(mnema_cli_sidecar_name_for_target_triple(
            current_target_triple_candidates()
                .first()
                .expect("current target triple should be known"),
        ));
        fs::write(&plain_cli, b"plain").expect("plain sidecar should be written");
        fs::write(&target_cli, b"target").expect("target sidecar should be written");

        assert_eq!(bundled_mnema_cli_path_in_dir(dir.path()), plain_cli);
    }

    #[test]
    fn bundled_mnema_cli_path_falls_back_to_target_triple_sidecar_name() {
        let dir = TestDir::new("mnema-cli-target-sidecar");
        let target_cli = dir.path().join(mnema_cli_sidecar_name_for_target_triple(
            current_target_triple_candidates()
                .first()
                .expect("current target triple should be known"),
        ));
        fs::write(&target_cli, b"target").expect("target sidecar should be written");

        assert_eq!(bundled_mnema_cli_path_in_dir(dir.path()), target_cli);
    }

    #[test]
    fn cli_symlink_replacement_requires_bundled_target_path() {
        let dir = TestDir::new("mnema-cli-symlink");
        let bundled_dir = dir.path().join("bundle");
        let other_dir = dir.path().join("other");
        fs::create_dir_all(&bundled_dir).expect("bundled dir should be created");
        fs::create_dir_all(&other_dir).expect("other dir should be created");
        let bundled_cli = bundled_dir.join(mnema_cli_sidecar_name());
        let other_cli = other_dir.join(mnema_cli_sidecar_name());
        fs::write(&bundled_cli, b"bundled").expect("bundled cli should be written");
        fs::write(&other_cli, b"other").expect("other cli should be written");

        assert!(existing_cli_symlink_is_safe_to_replace(
            &bundled_cli,
            &bundled_cli
        ));
        assert!(!existing_cli_symlink_is_safe_to_replace(
            &other_cli,
            &bundled_cli
        ));
    }
}
