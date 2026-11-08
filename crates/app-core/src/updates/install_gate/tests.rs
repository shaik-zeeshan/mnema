use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::*;

const FIXTURE_ID: &str = "day.mnema.gate-fixture";
const SOME_TEAM: &str = "ABCDE12345";

fn own(team: Option<&str>, bundle_id: &str) -> OwnIdentity {
    OwnIdentity {
        team: team.map(str::to_owned),
        bundle_id: bundle_id.to_owned(),
    }
}

fn write_bundle(root: &Path, id: &str, exe: &str) {
    let macos = root.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{id}</string>
<key>CFBundleExecutable</key><string>{exe}</string>
<key>CFBundleName</key><string>{exe}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
</dict></plist>
"#
    );
    std::fs::write(root.join("Contents/Info.plist"), plist).unwrap();
    let mut cc = Command::new("/usr/bin/cc")
        .args(["-x", "c", "-", "-o"])
        .arg(macos.join(exe))
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    cc.stdin
        .take()
        .unwrap()
        .write_all(b"int main(void) { return 0; }\n")
        .unwrap();
    assert!(cc.wait().unwrap().success(), "cc failed");
}

fn codesign(path: &Path, identity: &str) {
    let out = Command::new("/usr/bin/codesign")
        .args(["--force", "--timestamp=none", "--sign", identity])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "codesign failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `Fixture.app` with `Helpers/mnema-cli.app`, each signed with its own identity.
fn fixture(dir: &Path, bundle_id: &str, app_identity: &str, helper_identity: &str) -> PathBuf {
    let app = dir.join("Fixture.app");
    let helper = app.join("Contents/Helpers/mnema-cli.app");
    write_bundle(&helper, "day.mnema.cli", "mnema-cli");
    write_bundle(&app, bundle_id, "Fixture");
    codesign(&helper, helper_identity);
    codesign(&app, app_identity);
    app
}

fn adhoc_fixture(dir: &Path, bundle_id: &str) -> PathBuf {
    fixture(dir, bundle_id, "-", "-")
}

#[test]
fn a_running_app_without_a_team_cannot_install() {
    let dir = tempfile::tempdir().unwrap();
    let app = adhoc_fixture(dir.path(), FIXTURE_ID);
    assert_eq!(
        check_update_bundle_as(&app, &own(None, FIXTURE_ID)),
        Err(GateRefusal::NoOwnTeam)
    );
}

#[test]
fn the_unsigned_test_binary_has_no_team_so_production_check_refuses() {
    assert_eq!(OwnIdentity::current().team, None);
    let dir = tempfile::tempdir().unwrap();
    let app = adhoc_fixture(dir.path(), FIXTURE_ID);
    assert_eq!(check_update_bundle(&app), Err(GateRefusal::NoOwnTeam));
}

#[test]
fn an_adhoc_signed_bundle_fails_on_team() {
    let dir = tempfile::tempdir().unwrap();
    let app = adhoc_fixture(dir.path(), FIXTURE_ID);
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(SOME_TEAM), FIXTURE_ID)),
        Err(GateRefusal::TeamMismatch)
    );
}

#[test]
fn one_changed_byte_in_the_executable_fails_the_signature() {
    let dir = tempfile::tempdir().unwrap();
    let app = adhoc_fixture(dir.path(), FIXTURE_ID);
    let exe = app.join("Contents/MacOS/Fixture");
    let mut bytes = std::fs::read(&exe).unwrap();
    // Inside the first hashed page, well before the embedded signature.
    bytes[0x1000] ^= 0xff;
    std::fs::write(&exe, bytes).unwrap();
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(SOME_TEAM), FIXTURE_ID)),
        Err(GateRefusal::InvalidSignature)
    );
}

#[test]
fn a_different_bundle_id_fails() {
    let dir = tempfile::tempdir().unwrap();
    let app = adhoc_fixture(dir.path(), "day.mnema.someone-else");
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(SOME_TEAM), FIXTURE_ID)),
        Err(GateRefusal::BundleIdMismatch)
    );
}

#[test]
fn a_missing_bundle_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        check_update_bundle_as(
            &dir.path().join("Nope.app"),
            &own(Some(SOME_TEAM), FIXTURE_ID)
        ),
        Err(GateRefusal::Unreadable)
    );
}

fn test_identity() -> String {
    std::env::var("MNEMA_TEST_SIGNING_IDENTITY")
        .expect("set MNEMA_TEST_SIGNING_IDENTITY to an Apple Development identity")
}

/// The team `codesign` itself reports, independent of the gate's own reading.
fn codesign_team(path: &Path) -> String {
    let out = Command::new("/usr/bin/codesign")
        .arg("-dv")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .filter(|team| *team != "not set")
        .expect("signed fixture has a team")
        .to_owned()
}

#[test]
#[ignore = "needs MNEMA_TEST_SIGNING_IDENTITY (an Apple Development identity)"]
fn a_validly_signed_bundle_passes() {
    let identity = test_identity();
    let dir = tempfile::tempdir().unwrap();
    let app = fixture(dir.path(), FIXTURE_ID, &identity, &identity);
    let team = codesign_team(&app);
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(&team), FIXTURE_ID)),
        Ok(())
    );
    // The same bundle, ad-hoc re-signed, then fails on team.
    codesign(&app, "-");
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(&team), FIXTURE_ID)),
        Err(GateRefusal::TeamMismatch)
    );
}

#[test]
#[ignore = "needs MNEMA_TEST_SIGNING_IDENTITY (an Apple Development identity)"]
fn a_helper_signed_by_another_team_fails() {
    let identity = test_identity();
    let dir = tempfile::tempdir().unwrap();
    // Ad-hoc has no team, so it stands in for "some other team".
    let app = fixture(dir.path(), FIXTURE_ID, &identity, "-");
    let team = codesign_team(&app);
    assert_eq!(
        check_update_bundle_as(&app, &own(Some(&team), FIXTURE_ID)),
        Err(GateRefusal::HelperTeamMismatch)
    );
}

#[test]
fn the_team_is_proven_by_an_apple_anchored_requirement() {
    // The CodeDirectory team id is self-asserted; only a requirement on the
    // Apple-issued leaf certificate proves it. It must build for a real team id...
    assert!(apple_team_requirement(SOME_TEAM).is_ok());
    // ...and refuse text that could rewrite the requirement.
    for bad in ["", "ABC\" or anchor apple", "ABC DE", "ABCDE1234;"] {
        assert_eq!(apple_team_requirement(bad).err(), Some(GateRefusal::NoOwnTeam), "{bad}");
    }
}
