//! The GPUI updater's live record and its transitions (the state machine of
//! `AppUpdateState`), kept pure so tests drive every edge without a network.
//! The async flow in `flow.rs` only fetches, verifies and moves files between them.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::bundle::MainBundle;
use super::feed::{RemoteRelease, GPUI_PLATFORM_KEY};
use super::install_gate::{check_update_bundle, GateRefusal};
use super::settings::AppUpdateChannel;
use super::status::{
    AppUpdateAppInfo, AppUpdateError, AppUpdateErrorKind, AppUpdateInfo, AppUpdateProgress,
    AppUpdateState, RemoteUpdateDecision,
};
use super::verify::UPDATER_PUBKEY;

/// What a refused update shows (BUILD-PROMPT §9.2): a bad minisign signature or
/// a failed code-signature gate.
pub const NOT_SIGNED_BY_MNEMA: &str = "This update isn't signed by Mnema.";

/// How one app instance updates: who it is, which feed entry is its archive, and
/// where its bundle lives. Tests swap the feed, key and gate.
#[derive(Debug, Clone)]
pub struct UpdaterConfig {
    pub app: AppUpdateAppInfo,
    /// Replaces the channel's feed URL (a test feed baked in at build time).
    pub feed_override: Option<String>,
    pub platform_key: &'static str,
    pub pubkey: &'static str,
    /// The running `.app`; `None` (not run from a bundle) can't install.
    pub installed_app: Option<PathBuf>,
    pub gate: fn(&Path) -> Result<(), GateRefusal>,
}

impl UpdaterConfig {
    /// The running bundle's identity, the release key and the real install gate.
    /// `fallback_id` / `fallback_version` stand in outside a bundle.
    pub fn for_main_bundle(fallback_id: &str, fallback_version: &str) -> Self {
        let bundle = MainBundle::read();
        let name = bundle.name.unwrap_or_else(|| "Mnema".into());
        Self {
            app: AppUpdateAppInfo {
                product_name: name,
                version: bundle.version.unwrap_or_else(|| fallback_version.into()),
                identifier: bundle.identifier.unwrap_or_else(|| fallback_id.into()),
                platform: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            // A test feed, baked in by whoever builds (`bundle-gpui-app.sh` passes it
            // through); release CI never sets it. The archive still has to pass
            // the minisign key and the code-signature gate.
            feed_override: option_env!("MNEMA_UPDATE_FEED_URL")
                .filter(|u| !u.is_empty())
                .map(Into::into),
            platform_key: GPUI_PLATFORM_KEY,
            pubkey: UPDATER_PUBKEY,
            installed_app: bundle.path,
            gate: check_update_bundle,
        }
    }

    pub fn feed_url(&self, channel: AppUpdateChannel) -> String {
        self.feed_override
            .clone()
            .unwrap_or_else(|| channel.endpoint().to_string())
    }
}

/// The updater, managed on the `AppCore` by the GPUI host.
pub struct Updater {
    pub config: UpdaterConfig,
    pub runtime: Mutex<AppUpdateRuntime>,
}

impl Updater {
    pub fn new(config: UpdaterConfig) -> Self {
        Self {
            config,
            runtime: Mutex::default(),
        }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, AppUpdateRuntime> {
        self.runtime.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Default)]
pub struct AppUpdateRuntime {
    pub state: AppUpdateState,
    /// The release Install would fetch; only an in-window one is kept.
    pub pending: Option<RemoteRelease>,
    pub update: Option<AppUpdateInfo>,
    pub progress: Option<AppUpdateProgress>,
    pub error: Option<AppUpdateError>,
    pub last_checked_at_unix_ms: Option<u64>,
    pub restart_required: bool,
    /// Version the "update available" notice was last pushed for, so the 2 h
    /// re-check doesn't re-push (and un-dismiss) it.
    pub notified_version: Option<String>,
}

pub fn update_info(release: &RemoteRelease, channel: AppUpdateChannel) -> AppUpdateInfo {
    AppUpdateInfo {
        version: release.version.clone(),
        date: release.date.clone(),
        notes: release.notes.clone(),
        channel,
        size_bytes: release.size_bytes,
    }
}

impl AppUpdateRuntime {
    fn busy(&self) -> bool {
        self.restart_required
            || matches!(
                self.state,
                AppUpdateState::Checking | AppUpdateState::Downloading | AppUpdateState::Installing
            )
    }

    /// Idle/settled → `Checking`. Refused while a check or install runs, or a
    /// restart is staged (the caller returns the current status).
    pub fn begin_check(&mut self, now_ms: u64) -> bool {
        if self.busy() {
            return false;
        }
        self.state = AppUpdateState::Checking;
        self.pending = None;
        self.update = None;
        self.progress = None;
        self.error = None;
        self.last_checked_at_unix_ms = Some(now_ms);
        true
    }

    /// `Checking` → `Available` / `AvailableOutOfWindow` (kept but not installable).
    pub fn found(&mut self, release: RemoteRelease, info: AppUpdateInfo, d: &RemoteUpdateDecision) {
        self.state = d.state;
        self.pending = d.installable.then_some(release);
        self.update = Some(info);
        self.progress = None;
        self.error = None;
    }

    /// `Checking` → `UpToDate`.
    pub fn up_to_date(&mut self) {
        self.state = AppUpdateState::UpToDate;
        self.pending = None;
        self.update = None;
        self.progress = None;
        self.error = None;
    }

    /// A check against a channel that changed mid-flight: back to `Idle`, the
    /// result dropped (the caller starts a fresh check).
    pub fn discard_stale_check(&mut self) {
        self.state = AppUpdateState::Idle;
        self.pending = None;
        self.update = None;
        self.error = None;
    }

    /// Any failure: `Failed` (or `Incompatible`) with the error. The pending
    /// release stays, so Install reads "Try again".
    pub fn fail(&mut self, error: AppUpdateError) {
        self.state = if error.kind == AppUpdateErrorKind::Incompatible {
            AppUpdateState::Incompatible
        } else {
            AppUpdateState::Failed
        };
        self.error = Some(error);
        self.progress = None;
    }

    /// Whether the first "update available" notice for `version` is still due.
    pub fn take_notice(&mut self, version: &str) -> bool {
        if self.notified_version.as_deref() == Some(version) {
            return false;
        }
        self.notified_version = Some(version.to_string());
        true
    }

    /// `Available`/`Failed` → `Downloading`, handing back the release to fetch.
    /// `Ok(None)`: already downloading/installing or a restart is staged.
    pub fn begin_install(&mut self) -> Result<Option<RemoteRelease>, ()> {
        if self.busy() {
            return Ok(None);
        }
        let Some(release) = self.pending.clone() else {
            self.state = AppUpdateState::Failed;
            self.error = Some(AppUpdateError::new(
                AppUpdateErrorKind::Install,
                "Check for updates before installing.",
            ));
            return Err(());
        };
        self.state = AppUpdateState::Downloading;
        self.progress = Some(AppUpdateProgress {
            downloaded_bytes: 0,
            content_length_bytes: release.size_bytes,
        });
        self.error = None;
        Ok(Some(release))
    }

    pub fn downloaded(&mut self, bytes: u64, total: Option<u64>) {
        self.progress = Some(AppUpdateProgress {
            downloaded_bytes: bytes,
            content_length_bytes: total,
        });
    }

    /// Download verified → `Installing` (unpack, code-signature gate, swap).
    pub fn installing(&mut self) {
        self.state = AppUpdateState::Installing;
        self.progress = None;
    }

    /// The new bundle is in place → `RestartRequired`.
    pub fn installed(&mut self) {
        self.state = AppUpdateState::RestartRequired;
        self.progress = None;
        self.error = None;
        self.restart_required = true;
    }
}

/// The error a refused, unreadable or unswappable update shows.
pub fn gate_error(refusal: GateRefusal) -> AppUpdateError {
    match refusal {
        // The running app can't vouch for anything; the update itself may be fine.
        GateRefusal::NoOwnTeam => AppUpdateError::new(
            AppUpdateErrorKind::Install,
            "This copy of Mnema can't install updates. Download the newest version from mnema.day.",
        ),
        _ => AppUpdateError::new(AppUpdateErrorKind::Verification, NOT_SIGNED_BY_MNEMA),
    }
}

#[cfg(test)]
mod tests;
