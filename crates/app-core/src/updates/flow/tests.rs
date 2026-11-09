//! The whole flow against a local feed: check → install → swap, on temp dirs,
//! with a throwaway minisign key (legacy, non-prehashed signatures; the
//! prehashed release format is covered in `verify.rs`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::test_support::{core_in, FakeHost};
use crate::updates::runtime::UpdaterConfig;
use crate::updates::{AppUpdateAppInfo, AppUpdateSettingsState, GateRefusal};
use crate::CoreEvent;

const KEYNUM: [u8; 8] = [9; 8];

fn key() -> SigningKey {
    SigningKey::from_bytes(&[3; 32])
}

fn b64(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn pubkey() -> &'static str {
    let pk = [&b"Ed"[..], &KEYNUM, key().verifying_key().as_bytes()].concat();
    let file = format!(
        "untrusted comment: minisign public key: test\n{}\n",
        b64(pk)
    );
    Box::leak(b64(file).into_boxed_str())
}

/// A legacy (`Ed`) minisign signature file of `data`, base64'd like the feed's.
fn sign(data: &[u8]) -> String {
    let sig = key().sign(data).to_bytes();
    let trusted = "timestamp:1\tfile:Mnema.app.tar.gz";
    let global = key()
        .sign(&[&sig[..], trusted.as_bytes()].concat())
        .to_bytes();
    let blob = [&b"Ed"[..], &KEYNUM, &sig].concat();
    b64(format!(
        "untrusted comment: test\n{}\ntrusted comment: {trusted}\n{}\n",
        b64(blob),
        b64(global)
    ))
}

fn write_app(root: &Path, marker: &str) -> PathBuf {
    let app = root.join("Mnema.app");
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(app.join("Contents/MacOS/mnema-gpui"), marker).unwrap();
    app
}

fn marker(app: &Path) -> String {
    fs::read_to_string(app.join("Contents/MacOS/mnema-gpui")).unwrap()
}

fn archive_of(app: &Path) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all("Mnema.app", app).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

/// Serves `/latest.json` (built from the server's own base URL) and the archive
/// at any other path, until the runtime ends.
async fn serve(archive: Vec<u8>, feed: impl FnOnce(&str) -> Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let feed = feed(&base);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (feed, archive) = (feed.clone(), archive.clone());
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let body = if head.starts_with("GET /latest.json") {
                    feed
                } else {
                    archive
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    base
}

struct Rig {
    core: Arc<AppCore>,
    host: Arc<FakeHost>,
    installed: PathBuf,
    _dir: tempfile::TempDir,
}

/// Running app `running` at `<tmp>/Applications/Mnema.app` ("old"); the feed
/// offers 0.2.0 ("new"), signed — or, with `tamper`, with one archive byte changed.
fn rig(running: &str, tamper: bool, gate: fn(&Path) -> Result<(), GateRefusal>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let installed = write_app(&dir.path().join("Applications"), "old");
    let mut archive = archive_of(&write_app(&dir.path().join("build"), "new"));
    let signature = sign(&archive);
    let size = archive.len();
    if tamper {
        archive[size / 2] ^= 1;
    }
    let host = Arc::new(FakeHost::default());
    let core = Arc::new(core_in(host.clone(), dir.path()));
    let base = core.block_on(serve(archive, |base| {
        serde_json::json!({
            "version": "0.2.0",
            "pub_date": "2026-11-01T10:00:00Z",
            "platforms": { GPUI_KEY: {
                "url": format!("{base}/Mnema.app.tar.gz"),
                "signature": signature,
                "size": size,
            }}
        })
        .to_string()
        .into_bytes()
    }));
    core.manage(AppUpdateSettingsState::default());
    core.manage(Updater::new(UpdaterConfig {
        app: AppUpdateAppInfo {
            product_name: "Mnema".into(),
            version: running.into(),
            identifier: "day.mnema.test".into(),
            platform: "macos".into(),
            arch: "aarch64".into(),
        },
        feed_override: Some(format!("{base}/latest.json")),
        platform_key: GPUI_KEY,
        pubkey: pubkey(),
        installed_app: Some(installed.clone()),
        gate,
    }));
    Rig {
        core,
        host,
        installed,
        _dir: dir,
    }
}

const GPUI_KEY: &str = crate::updates::GPUI_PLATFORM_KEY;

fn states(host: &FakeHost) -> Vec<AppUpdateState> {
    let mut out: Vec<AppUpdateState> = Vec::new();
    for e in host.events.lock().unwrap().iter() {
        if let CoreEvent::AppUpdateStatusChanged(s) = e {
            if out.last() != Some(&s.state) {
                out.push(s.state);
            }
        }
    }
    out
}

/// No `.mnema-update-*` staging dir (or the old app inside it) is left behind.
fn staging_left(installed: &Path) -> bool {
    fs::read_dir(installed.parent().unwrap()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mnema-update-")
    })
}

fn check_then_install(rig: &Rig) -> AppUpdateStatus {
    let checked = rig
        .core
        .block_on(run_update_check(&rig.core, false))
        .unwrap();
    assert_eq!(checked.state, AppUpdateState::Available);
    assert_eq!(checked.update.as_ref().unwrap().version, "0.2.0");
    assert!(installable(&rig.core));
    rig.core.block_on(install_app_update(&rig.core)).unwrap()
}

#[test]
fn a_signed_update_is_swapped_in_and_waits_for_restart() {
    let rig = rig("0.1.0", false, |_| Ok(()));
    let status = check_then_install(&rig);
    assert_eq!(
        status.state,
        AppUpdateState::RestartRequired,
        "{:?}",
        status.error
    );
    assert_eq!(marker(&rig.installed), "new");
    assert!(!staging_left(&rig.installed));
    assert_eq!(
        states(&rig.host),
        [
            AppUpdateState::Checking,
            AppUpdateState::Available,
            AppUpdateState::Downloading,
            AppUpdateState::Installing,
            AppUpdateState::RestartRequired,
        ]
    );
    // Restart now would hand over to the graceful exit (not run here).
    assert!(rig.core.state::<Updater>().lock().restart_required);
}

#[test]
fn a_tampered_archive_is_refused_and_the_old_app_kept() {
    let rig = rig("0.1.0", true, |_| {
        panic!("the gate never runs on a bad signature")
    });
    let status = check_then_install(&rig);
    assert_eq!(status.state, AppUpdateState::Failed);
    let error = status.error.unwrap();
    assert_eq!(error.kind, AppUpdateErrorKind::Verification);
    assert_eq!(error.message, NOT_SIGNED_BY_MNEMA);
    assert_eq!(marker(&rig.installed), "old");
    assert!(!staging_left(&rig.installed));
    // Retry stays armed, and Restart has nothing to finish.
    assert!(installable(&rig.core));
    assert!(restart_after_app_update(&rig.core).is_err());
}

#[test]
fn a_gate_refusal_keeps_the_old_app() {
    let rig = rig("0.1.0", false, |_| Err(GateRefusal::TeamMismatch));
    let status = check_then_install(&rig);
    assert_eq!(status.state, AppUpdateState::Failed);
    assert_eq!(status.error.unwrap().message, NOT_SIGNED_BY_MNEMA);
    assert_eq!(marker(&rig.installed), "old");
    assert!(!staging_left(&rig.installed));
}

#[test]
fn the_real_gate_refuses_from_an_unsigned_running_app() {
    // A test binary has no team, so the real gate disables installs (NoOwnTeam).
    let rig = rig("0.1.0", false, crate::updates::check_update_bundle);
    let status = check_then_install(&rig);
    assert_eq!(status.state, AppUpdateState::Failed);
    assert_eq!(status.error.unwrap().kind, AppUpdateErrorKind::Install);
    assert_eq!(marker(&rig.installed), "old");
}

#[test]
fn a_running_version_at_the_feed_is_up_to_date() {
    let rig = rig("0.2.0", false, |_| Ok(()));
    let status = rig
        .core
        .block_on(run_update_check(&rig.core, false))
        .unwrap();
    assert_eq!(status.state, AppUpdateState::UpToDate);
    assert!(!installable(&rig.core));
}

#[test]
fn install_before_a_check_fails_and_a_staged_restart_is_never_redone() {
    let rig = rig("0.1.0", false, |_| Ok(()));
    let early = rig.core.block_on(install_app_update(&rig.core)).unwrap();
    assert_eq!(early.state, AppUpdateState::Failed);
    assert_eq!(early.error.unwrap().kind, AppUpdateErrorKind::Install);
    assert_eq!(
        check_then_install(&rig).state,
        AppUpdateState::RestartRequired
    );
    // The tray's "Restart to Finish" and a 2 h tick both leave the staged swap alone.
    let again = rig.core.block_on(install_app_update(&rig.core)).unwrap();
    assert_eq!(again.state, AppUpdateState::RestartRequired);
    let tick = rig
        .core
        .block_on(run_update_check(&rig.core, true))
        .unwrap();
    assert_eq!(tick.state, AppUpdateState::RestartRequired);
    assert_eq!(marker(&rig.installed), "new");
}
