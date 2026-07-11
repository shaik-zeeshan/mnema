//! Node runtime detection for local (stdio) MCP connector presets.
//!
//! Settings asks "is Node installed?" before offering local presets that spawn
//! via `npx`. Resolution mirrors how stdio connectors find their commands
//! (`transport::login_shell_path`): a packaged macOS app inherits launchd's
//! minimal PATH (no Homebrew/nvm), so a bare `node` probe against the inherited
//! environment would report "missing" for most real installs.

/// Run `node --version` with the given PATH (the login-shell PATH when
/// resolution succeeded; `None` → inherit the app's PATH, same fallback as
/// transport.rs). Returns the trimmed version (e.g. `"v22.11.0"`); spawn
/// failure, non-zero exit, or empty output → `None`.
fn detect_node(path: Option<&str>) -> Option<String> {
    let mut command = std::process::Command::new("node");
    command.arg("--version");
    if let Some(path) = path {
        // Rust resolves the program against the PATH set on the Command (Unix),
        // exactly like the stdio connector spawn.
        command.env("PATH", path);
    }
    let output = command
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let version = String::from_utf8(output.stdout).ok()?;
    let version = version.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// The installed Node version (e.g. `"v22.11.0"`), or `None` when Node is not
/// on the user's login-shell PATH. Blocking work (shell PATH resolution + the
/// probe spawn) runs off the async runtime.
#[tauri::command]
pub async fn mcp_check_node() -> Option<String> {
    tokio::task::spawn_blocking(|| detect_node(super::transport::login_shell_path()))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a dev box, Node is on the login-shell PATH and reports a `vX.Y.Z`
    /// version. If this fails, either the box has no Node or the login-shell
    /// PATH resolution regressed (see transport.rs `login_shell_path_resolves`).
    ///
    /// macOS-only: the login-shell PATH mechanism is Unix (`$SHELL -l -c`) and
    /// SUPPORTS.md marks Windows unaddressed for it.
    #[cfg(target_os = "macos")]
    #[test]
    fn detect_node_finds_node_on_the_login_shell_path() {
        let path = crate::ask_ai::mcp::transport::login_shell_path();
        let version = detect_node(path).expect("dev box should have node on the login-shell PATH");
        assert!(version.starts_with('v'), "unexpected node version: {version}");
    }

    /// A PATH with no node in it must yield None (spawn failure), not an error
    /// or a fallback to the inherited environment.
    #[test]
    fn detect_node_returns_none_for_a_bogus_path() {
        assert_eq!(detect_node(Some("/nonexistent-mnema-node-check")), None);
    }
}
