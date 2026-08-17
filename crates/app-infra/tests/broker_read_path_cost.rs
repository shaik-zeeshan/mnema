//! Measurement probe for the brokered read path (perf review, ADR 0059 branch).
//!
//! Public-API only, no database: a request from an identity with NO permission
//! is refused before `initialize_infra`, so this times exactly the per-command
//! bookkeeping the branch added — the grants read(s) and the audit rewrite.

use app_infra::brokered_access::{
    BrokerClientIdentity, BrokerClientIdentitySource, BrokerGrantScope, BrokerSearchRequest,
    BrokeredCaptureAccess, BrokeredCaptureRequest,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-broker-perf-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
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

fn audit_bytes(dir: &Path) -> u64 {
    std::fs::metadata(dir.join("broker-audit.json"))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Drive the refusal path until the audit log is at its 500-event cap, then time
/// one more refusal. That single call is the whole per-command cost the branch
/// pays with no permission at all: 1 grants read + a full 500-event audit
/// read-parse-serialize-write under an exclusive flock.
#[test]
fn cost_of_one_refused_command_at_the_audit_cap() {
    let dir = temp_dir("denial");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    let rt = runtime();
    let who = identity("Claude Code");

    // Warm to the cap.
    for _ in 0..520 {
        rt.block_on(access.execute_for_identity(who.clone(), search_request()))
            .unwrap();
    }
    let events = access.list_history().unwrap().events.len();
    let bytes = audit_bytes(&dir);

    let iterations = 200;
    let start = Instant::now();
    for _ in 0..iterations {
        rt.block_on(access.execute_for_identity(who.clone(), search_request()))
            .unwrap();
    }
    let per_call = start.elapsed() / iterations;

    // Split out the read half, so the append's read-parse vs serialize-write
    // shares are separable.
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(access.list_history().unwrap());
    }
    let read_only = start.elapsed() / iterations;

    println!(
        "REFUSED COMMAND @cap: events={events} audit_file={bytes}B \n  per_call={per_call:?}\n  of which audit read+parse={read_only:?}\n  disk written per call={bytes}B -> {:.1} MB per 60 calls",
        (bytes as f64 * 60.0) / (1024.0 * 1024.0)
    );
}

/// Same command, same identity, but WITH a standing permission — minus the
/// database, which a granted request would also open. Isolates the branch's
/// added grants read on the success path.
#[test]
fn cost_of_the_doubled_grants_read() {
    let dir = temp_dir("double");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    for n in 0..12 {
        access
            .upsert_grant_for_identity(identity(&format!("tool {n}")), BrokerGrantScope::LAST_DAY)
            .unwrap();
    }
    let iterations = 2000;
    // main: one load_grants. HEAD: load_grants + touch_last_used's own load_grants.
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(access.list_grants().unwrap());
    }
    let one = start.elapsed() / iterations;
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(access.list_grants().unwrap());
        app_infra::brokered_access::touch_last_used(&dir, "tool 0").unwrap();
    }
    let two = start.elapsed() / iterations;
    println!("GRANTS: main(1 read)={one:?} head(2 reads)={two:?} delta={:?}", two - one);
}

/// `load_grants` is what `active_grant_for_identity` runs, and what
/// `touch_last_used` runs AGAIN a few lines later. Time one, so the doubled
/// parse has a number.
#[test]
fn cost_of_one_grants_read() {
    let dir = temp_dir("grants");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    for n in 0..12 {
        access
            .upsert_grant_for_identity(identity(&format!("tool {n}")), BrokerGrantScope::LAST_DAY)
            .unwrap();
    }
    let bytes = std::fs::metadata(dir.join("broker-grants.json"))
        .unwrap()
        .len();

    let iterations = 2000;
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(access.list_grants().unwrap());
    }
    let per_call = start.elapsed() / iterations;
    println!(
        "GRANTS READ: rows=12 grants_file={bytes}B per_read={:?}",
        per_call
    );
}

/// The hourly stamp: pre-check read, then flock + read + write (+ a prune write).
#[test]
fn cost_of_the_hourly_stamp() {
    let dir = temp_dir("stamp");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    for n in 0..12 {
        access
            .upsert_grant_for_identity(identity(&format!("tool {n}")), BrokerGrantScope::LAST_DAY)
            .unwrap();
    }

    // Fresh stamp: pre-check only, returns without the lock.
    let iterations = 2000;
    let start = Instant::now();
    for _ in 0..iterations {
        app_infra::brokered_access::touch_last_used(&dir, "tool 0").unwrap();
    }
    let fresh = start.elapsed() / iterations;

    // Force staleness by rewriting the file with an old last_used.
    let raw = std::fs::read_to_string(dir.join("broker-grants.json")).unwrap();
    let mut file: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for grant in file["grants"].as_array_mut().unwrap() {
        grant["lastUsedAtUnixMs"] = serde_json::json!(1_000_000_000_000u64);
    }
    std::fs::write(
        dir.join("broker-grants.json"),
        serde_json::to_string_pretty(&file).unwrap(),
    )
    .unwrap();

    let start = Instant::now();
    app_infra::brokered_access::touch_last_used(&dir, "tool 0").unwrap();
    let stale = start.elapsed();

    println!("STAMP: fresh(pre-check only)={fresh:?} stale(flock+write)={stale:?}");
}

/// NEGATIVE SPACE: what bounds ONE audit event?
///
/// The event count is capped at 500, but `tool_identity` is the client label —
/// `--client` / `MNEMA_CLI_CLIENT` / `AI_AGENT`, none of them length-capped —
/// and it is stored twice per event (raw + normalized). On this branch a caller
/// with NO permission writes one of these per refused command, so the whole file
/// is sized by an unauthenticated argument.
#[test]
fn audit_file_size_is_set_by_an_uncapped_client_label() {
    let dir = temp_dir("bigLabel");
    let access = BrokeredCaptureAccess::from_config_dir(&dir);
    let rt = runtime();

    let label_bytes = 64 * 1024;
    let who = identity(&"A".repeat(label_bytes));

    let start = Instant::now();
    for _ in 0..520 {
        rt.block_on(access.execute_for_identity(who.clone(), search_request()))
            .unwrap();
    }
    let fill = start.elapsed();
    let bytes = audit_bytes(&dir);

    // One more refused command against the now-bloated file.
    let start = Instant::now();
    rt.block_on(access.execute_for_identity(who.clone(), search_request()))
        .unwrap();
    let one_more = start.elapsed();

    // And what the Settings panel pays to render 20 rows out of it.
    let start = Instant::now();
    let events = access.list_history().unwrap().events.len();
    let panel_read = start.elapsed();

    println!(
        "OVERSIZED LABEL: --client of {label_bytes}B -> audit_file={bytes}B ({:.1} MB)\n  \
         520 refused commands took {fill:?}\n  \
         one more refused command: {one_more:?}\n  \
         one Settings poll (list_cli_access_history) reads/parses {events} events in {panel_read:?}",
        bytes as f64 / (1024.0 * 1024.0)
    );

    // REGRESSION BOUND. 500 lines x (raw + normalized name) is the whole file,
    // so a capped name caps the file. 512 KB leaves ~500 B of room per line for
    // a 120-char name plus the fixed fields, and still catches any future
    // uncapped string being added to the event.
    assert!(
        bytes < 512 * 1024,
        "one unauthenticated `--client` argument sized broker-audit.json to {bytes} B; \
         every later brokered command rewrites that file and the Settings panel \
         re-reads it over IPC every 30 s"
    );
    // And the per-command cost stays in the same order as a normal-name refusal
    // (1.8 ms release / 13 ms debug measured), not two orders above it.
    assert!(
        one_more < std::time::Duration::from_millis(60),
        "one refused command cost {one_more:?}"
    );
}
