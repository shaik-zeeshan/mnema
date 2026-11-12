//! The `latest.json` feed: the format tauri-action writes and the promote
//! workflow rewrites to R2 (`{ version, notes, pub_date, platforms: { key:
//! { url, signature, size } } }`). The GPUI app reads its own platform entry,
//! [`GPUI_PLATFORM_KEY`], never the Tauri app's (`darwin-aarch64[-app]`): that
//! archive is the Tauri bundle, which carries the same team and bundle id and
//! would pass the install gate.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::status::{user_facing_error_message, AppUpdateError, AppUpdateErrorKind};

/// The GPUI app's archive in the shared feed (written by `macos-release.yml`).
pub const GPUI_PLATFORM_KEY: &str = "darwin-aarch64-gpui";

/// A feed response or archive that takes longer than this is a network failure.
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

/// The archive download fails after this long with no bytes, so a stalled
/// stream ends in `Failed` (retryable) instead of `Downloading` until relaunch.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// A release the feed names for this app that is newer than the running one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRelease {
    pub version: String,
    pub notes: Option<String>,
    /// `pub_date`, re-formatted as RFC 3339.
    pub date: Option<String>,
    pub date_ms: Option<i64>,
    pub url: String,
    /// Base64 of the minisign signature file, as tauri's signer writes it.
    pub signature: String,
    pub size_bytes: Option<u64>,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    #[serde(default)]
    platforms: HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
    signature: String,
    size: Option<u64>,
}

fn error(kind: AppUpdateErrorKind) -> AppUpdateError {
    AppUpdateError::new(kind, user_facing_error_message(kind))
}

/// The release in `raw` for `platform_key` when it is newer than `current`
/// (semver, a leading `v` ignored), `None` when it isn't. A newer release with
/// no entry for this app is `Incompatible`, as in the Tauri updater.
pub fn newer_release(
    raw: &[u8],
    platform_key: &str,
    current: &str,
) -> Result<Option<RemoteRelease>, AppUpdateError> {
    let feed: Feed = serde_json::from_slice(raw).map_err(|_| error(AppUpdateErrorKind::Feed))?;
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v'));
    let remote = parse(&feed.version).map_err(|_| error(AppUpdateErrorKind::Feed))?;
    let running = parse(current).map_err(|_| error(AppUpdateErrorKind::Unknown))?;
    if remote <= running {
        return Ok(None);
    }
    let platform = feed
        .platforms
        .get(platform_key)
        .ok_or_else(|| error(AppUpdateErrorKind::Incompatible))?;
    let date = feed
        .pub_date
        .as_deref()
        .and_then(|d| OffsetDateTime::parse(d, &Rfc3339).ok());
    Ok(Some(RemoteRelease {
        version: remote.to_string(),
        notes: feed.notes.filter(|n| !n.trim().is_empty()),
        date: date.and_then(|d| d.format(&Rfc3339).ok()),
        date_ms: date.map(|d| (d.unix_timestamp_nanos() / 1_000_000) as i64),
        url: platform.url.clone(),
        signature: platform.signature.clone(),
        size_bytes: platform.size,
    }))
}

pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(FEED_TIMEOUT)
        .read_timeout(READ_IDLE_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// GETs the feed. Unreachable is `Network`; a non-2xx answer is `Feed`.
pub async fn fetch_feed(url: &str) -> Result<Vec<u8>, AppUpdateError> {
    let response = client()
        .get(url)
        .timeout(FEED_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            log::warn!("app update check failed: kind=Network; error={e}");
            error(AppUpdateErrorKind::Network)
        })?;
    if !response.status().is_success() {
        log::warn!(
            "app update check failed: kind=Feed; status={}",
            response.status()
        );
        return Err(error(AppUpdateErrorKind::Feed));
    }
    response.bytes().await.map(|b| b.to_vec()).map_err(|e| {
        log::warn!("app update check failed: kind=Network; error={e}");
        error(AppUpdateErrorKind::Network)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(version: &str, key: &str) -> Vec<u8> {
        serde_json::json!({
            "version": version,
            "notes": "## Fixes\n- faster",
            "pub_date": "2026-11-01T10:00:00Z",
            "platforms": {
                "darwin-aarch64-app": { "url": "https://x/tauri.tar.gz", "signature": "tauri" },
                key: { "url": "https://x/gpui.tar.gz", "signature": "gpui", "size": 42 }
            }
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_newer_release_reads_this_apps_entry_only() {
        let r = newer_release(
            &feed("0.2.0", GPUI_PLATFORM_KEY),
            GPUI_PLATFORM_KEY,
            "0.1.27",
        )
        .unwrap()
        .expect("newer");
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.url, "https://x/gpui.tar.gz");
        assert_eq!(r.signature, "gpui");
        assert_eq!(r.size_bytes, Some(42));
        assert_eq!(r.date.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert_eq!(r.date_ms, Some(1_793_527_200_000));
    }

    #[test]
    fn same_or_older_is_up_to_date() {
        let raw = feed("v0.1.27", GPUI_PLATFORM_KEY);
        assert_eq!(newer_release(&raw, GPUI_PLATFORM_KEY, "0.1.27"), Ok(None));
        assert_eq!(newer_release(&raw, GPUI_PLATFORM_KEY, "0.2.0"), Ok(None));
        // Semver, not string order: 0.1.10 > 0.1.9.
        let raw = feed("0.1.10", GPUI_PLATFORM_KEY);
        assert!(newer_release(&raw, GPUI_PLATFORM_KEY, "0.1.9")
            .unwrap()
            .is_some());
    }

    #[test]
    fn a_newer_release_without_a_gpui_archive_is_incompatible() {
        // A feed with only the Tauri app's archive must never be installed here.
        let raw = feed("0.2.0", "darwin-aarch64");
        let err = newer_release(&raw, GPUI_PLATFORM_KEY, "0.1.27").unwrap_err();
        assert_eq!(err.kind, AppUpdateErrorKind::Incompatible);
    }

    #[test]
    fn garbage_is_a_feed_error() {
        let err = newer_release(b"<html>", GPUI_PLATFORM_KEY, "0.1.0").unwrap_err();
        assert_eq!(err.kind, AppUpdateErrorKind::Feed);
        let err = newer_release(&feed("latest", "k"), "k", "0.1.0").unwrap_err();
        assert_eq!(err.kind, AppUpdateErrorKind::Feed);
    }
}
