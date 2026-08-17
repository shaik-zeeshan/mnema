//! ADR 0059 regression: the one wire that makes idle expiry *idle*.
//!
//! `BrokeredCaptureAccess::execute*` stamps the permission it just used, as a
//! bare `let _ = touch_last_used(..)` with nothing asserting it. Every stamping
//! test in `brokered_access/tests.rs` calls `touch_last_used` DIRECTLY, so
//! dropping that one line from the read path leaves them all green while every
//! standing permission quietly becomes a 30-day ticket counted from approval —
//! a tool in daily use gets re-prompted mid-task, which is the exact failure the
//! ADR replaced the 24-hour grant to stop.
//!
//! Public API only, no database: the stamp is taken before the request is served
//! (a use is a use even if the read then fails), so the outcome is irrelevant.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantScope, BrokerSearchRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest,
};
use std::path::{Path, PathBuf};

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_millis() as u64
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-broker-standing-{name}-{}-{}",
        std::process::id(),
        now_unix_ms()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir should create");
    dir
}

/// Back-date every row's last use, so the next read is past the coarse
/// one-hour stamp interval and owes a rewrite.
fn set_last_used(config_dir: &Path, unix_ms: u64) {
    let path = config_dir.join("broker-grants.json");
    let raw = std::fs::read_to_string(&path).expect("permission file should exist");
    let mut file: serde_json::Value = serde_json::from_str(&raw).expect("permission file parses");
    for grant in file["grants"]
        .as_array_mut()
        .expect("the file carries a grants array")
    {
        grant["lastUsedAtUnixMs"] = serde_json::json!(unix_ms);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap())
        .expect("permission file should rewrite");
}

fn search_request() -> BrokeredCaptureRequest {
    BrokeredCaptureRequest::Search(BrokerSearchRequest {
        query: "quarterly plan".to_string(),
        from: None,
        to: None,
        limit: None,
        app: None,
        window_title: None,
        url: None,
        url_regex: None,
        cursor: None,
        speaker: None,
    })
}

#[test]
fn a_brokered_read_stamps_the_permission_it_used() {
    let config_dir = temp_dir("read-stamps-last-use");
    let access = BrokeredCaptureAccess::from_config_dir(&config_dir);
    let who = BrokerClientIdentity::new("Claude Code", BrokerClientIdentitySource::Explicit)
        .expect("label normalizes");
    let created = access
        .upsert_grant_for_identity(who.clone(), BrokerGrantScope::LAST_DAY)
        .expect("approval stores")
        .grant;

    let stale_at = now_unix_ms().saturating_sub(3 * 60 * 60 * 1000);
    set_last_used(&config_dir, stale_at);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds");
    let _ = runtime.block_on(access.execute_for_identity(who, search_request()));

    let rows = access.list_grants().expect("permissions load").grants;
    assert_eq!(rows.len(), 1, "one row per identity: {rows:?}");
    assert_eq!(rows[0].id, created.id, "a read never re-mints the row id");
    assert!(
        rows[0].last_used_at_unix_ms > stale_at,
        "a brokered read must stamp the permission it used, or the row idle-expires \
         30 days after APPROVAL instead of after last use: {rows:?}"
    );
}
