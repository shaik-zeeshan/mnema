//! What a brokered read is allowed to cost (ADR 0041, ADR 0059).
//!
//! A brokered read must stay a READ: no exclusive lock, no config-file rewrite per
//! call, and no unbounded file growable by an unauthenticated argument. These are
//! the three claims no unit test in `brokered_access/tests.rs` can make, because
//! they are about what the read path does NOT do — take a lock another process
//! holds, and write.
//!
//! Public API only, no database: a request from an identity with no permission is
//! refused before `initialize_infra`, so this exercises exactly the per-command
//! bookkeeping — the grants read, the stamp, and the audit append.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantScope, BrokerSearchRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest,
};
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-broker-read-path-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn identity(label: &str) -> BrokerClientIdentity {
    BrokerClientIdentity::new(label, BrokerClientIdentitySource::Explicit).unwrap()
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

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// The permission file's last-write time. `mtime` rather than a content compare,
/// because a rewrite that stores the same values is still the flocked write this
/// path exists to avoid, and content equality cannot see it.
fn grants_written_at(dir: &Path) -> SystemTime {
    std::fs::metadata(dir.join("broker-grants.json"))
        .expect("permission file should exist")
        .modified()
        .expect("mtime should be available")
}

fn back_date_last_used(dir: &Path, unix_ms: u64) {
    let path = dir.join("broker-grants.json");
    let raw = std::fs::read_to_string(&path).expect("permission file should exist");
    let mut file: serde_json::Value = serde_json::from_str(&raw).expect("permission file parses");
    for grant in file["grants"].as_array_mut().expect("grants array") {
        grant["lastUsedAtUnixMs"] = serde_json::json!(unix_ms);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap())
        .expect("back-dating writes");
}

fn last_used(dir: &Path, normalized_label: &str) -> u64 {
    BrokeredCaptureAccess::from_config_dir(dir)
        .list_grants()
        .expect("permissions load")
        .grants
        .into_iter()
        .find(|grant| grant.normalized_label == normalized_label)
        .unwrap_or_else(|| panic!("{normalized_label} should still have a row"))
        .last_used_at_unix_ms
}

/// The read path must not queue behind the writers. `active_grant_for_identity`
/// and the common `touch_last_used` both take the SHARED read only, so a held
/// approval/prune lock cannot stall a `mnema search` — and if either one ever
/// starts opening `broker-grants.lock`, every brokered command in flight blocks
/// on whatever the app is doing to the file (ADR 0041).
///
/// A regression here HANGS the read path rather than failing it, so the read runs
/// on a worker thread against a timeout.
#[test]
fn a_read_never_queues_behind_the_permission_lock() {
    let dir = temp_dir("read-lock");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    access
        .upsert_grant_for_identity(identity("Claude Code"), BrokerGrantScope::LAST_DAY)
        .expect("approval stores");

    // Exactly what an approval or a prune holds while it rewrites the file. flock
    // is per open file description, so a second handle in this same process is as
    // blocked as another process would be.
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(dir.join("broker-grants.lock"))
        .expect("lock file opens");
    lock.lock_exclusive().expect("lock should be free");

    let (done, wait) = std::sync::mpsc::channel();
    let worker_dir = dir.clone();
    std::thread::spawn(move || {
        let access = BrokeredCaptureAccess::from_config_dir(&worker_dir);
        let read = access.list_grants().map(|file| file.grants.len());
        let stamp = app_infra::brokered_access::touch_last_used(&worker_dir, "claude code");
        let _ = done.send((read, stamp));
    });

    let outcome = wait.recv_timeout(Duration::from_secs(10));
    lock.unlock().expect("lock releases");

    let (read, stamp) = outcome.expect(
        "a brokered read blocked on the permission lock: the read path must take the \
         shared read only, or every `mnema` command waits on whatever the app is \
         writing (ADR 0041)",
    );
    assert_eq!(read.expect("permissions load"), 1);
    stamp.expect("a fresh stamp is a read and must not need the lock");
}

/// Coarse stamping, in the two halves that matter: nothing is written inside the
/// interval no matter how many calls arrive, and a stale row costs ONE write for
/// the whole burst — not one per call, which is the flocked-rewrite-per-read that
/// ADR 0041 forbids.
///
/// And the stamp is per ROW. If a use stamped every row, no permission would ever
/// idle-expire: one tool running daily would hold the whole list open forever,
/// which is the standing-permission version of never expiring at all.
#[test]
fn a_stale_stamp_costs_one_write_and_touches_only_the_row_that_was_used() {
    let dir = temp_dir("stamp");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    for label in ["Claude Code", "Codex"] {
        access
            .upsert_grant_for_identity(identity(label), BrokerGrantScope::LAST_DAY)
            .expect("approval stores");
    }

    // Inside the interval: no write at all, however many calls arrive.
    let fresh_at = grants_written_at(&dir);
    for _ in 0..50 {
        app_infra::brokered_access::touch_last_used(&dir, "claude code").expect("stamp runs");
    }
    assert_eq!(
        grants_written_at(&dir),
        fresh_at,
        "a stamp inside the interval rewrote the permission file: a brokered read \
         must not pay a flocked write per call (ADR 0041)"
    );

    let stale_at = now_unix_ms().saturating_sub(3 * 60 * 60 * 1000);
    back_date_last_used(&dir, stale_at);
    let before = grants_written_at(&dir);

    app_infra::brokered_access::touch_last_used(&dir, "claude code").expect("stale stamp runs");
    let after_first = grants_written_at(&dir);
    assert_ne!(before, after_first, "a stale row must be stamped");
    assert!(
        last_used(&dir, "claude code") > stale_at,
        "the row that was used is stamped"
    );
    assert_eq!(
        last_used(&dir, "codex"),
        stale_at,
        "one tool's use must not reset another row's idle clock, or nothing ever \
         idle-expires"
    );

    // ...and the burst behind it is free again: the stamp is now fresh.
    for _ in 0..50 {
        app_infra::brokered_access::touch_last_used(&dir, "claude code").expect("stamp runs");
    }
    assert_eq!(
        grants_written_at(&dir),
        after_first,
        "one write for the whole stale burst, not one per call"
    );
}

/// NEGATIVE SPACE: what bounds ONE audit event?
///
/// The event count is capped at 500, but `tool_identity` is the client label —
/// `--client` / `MNEMA_CLI_CLIENT` / `AI_AGENT`, none of them length-capped at the
/// wire — and it is stored twice per event (raw + normalized). On this branch a
/// caller with NO permission writes one of these per refused command, so the whole
/// file would be sized by an unauthenticated argument.
#[test]
fn audit_file_size_is_set_by_an_uncapped_client_label() {
    let dir = temp_dir("bigLabel");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    let rt = runtime();

    let who = identity(&"A".repeat(64 * 1024));
    for _ in 0..520 {
        rt.block_on(access.execute_for_identity(who.clone(), search_request()))
            .unwrap();
    }
    let bytes = std::fs::metadata(dir.join("broker-audit.json"))
        .map(|meta| meta.len())
        .unwrap_or(0);

    // One more refused command against the now-filled file.
    let start = Instant::now();
    rt.block_on(access.execute_for_identity(who.clone(), search_request()))
        .unwrap();
    let one_more = start.elapsed();

    // REGRESSION BOUND. 500 lines x (raw + normalized name) is the whole file, so
    // a capped name caps the file. 512 KB leaves ~500 B of room per line for a
    // 120-char name plus the fixed fields, and still catches any future uncapped
    // string being added to the event.
    assert!(
        bytes < 512 * 1024,
        "one unauthenticated `--client` argument sized broker-audit.json to {bytes} B; \
         every later brokered command rewrites that file and the Settings panel \
         re-reads it over IPC every 30 s"
    );
    // And the per-command cost stays in the same order as a normal-name refusal
    // (1.8 ms release / 13 ms debug measured), not two orders above it.
    assert!(
        one_more < Duration::from_millis(60),
        "one refused command cost {one_more:?}"
    );
}
