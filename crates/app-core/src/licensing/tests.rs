use super::*;

fn crl_naming(id: &str) -> licensegate::Crl {
    licensegate::Crl {
        kid: "56475aa7".to_string(),
        issued_at: "2026-07-10T00:00:00Z".to_string(),
        revoked_license_ids: vec![id.to_string()],
    }
}

fn key_with_id(license_id: &str) -> licensegate::LicenseKey {
    licensegate::LicenseKey {
        kid: "24f6ed6a".into(),
        license_id: license_id.into(),
        plan: "pro".into(),
        customer: licensegate::Customer {
            name: "Owner".into(),
            email: "owner@example.com".into(),
        },
        entitlements: vec![licensegate::Entitlement {
            key: "app".into(),
            expires_at: None,
        }],
        issued_at: "2026-07-01T00:00:00Z".into(),
    }
}

#[test]
fn stale_compute_never_publishes_over_a_newer_one() {
    let last = AtomicU64::new(0);
    // In-order publishes all land.
    assert!(try_claim_publish(&last, 1));
    assert!(try_claim_publish(&last, 2));
    // A compute that started BEFORE the last published one is dropped —
    // this is the CRL-refresh race: stale `Licensed` can't overwrite `Revoked`.
    assert!(!try_claim_publish(&last, 1));
    // The newest generation republishing (same compute re-checked) still lands.
    assert!(try_claim_publish(&last, 2));
    assert!(try_claim_publish(&last, 3));
}

#[test]
fn is_key_revoked_hit_miss_and_no_crl() {
    let crl = crl_naming("01PRESS");
    assert!(is_key_revoked("01PRESS", Some(&crl)));
    assert!(!is_key_revoked("01FRIEND", Some(&crl)));
    assert!(!is_key_revoked("01PRESS", None));
}

#[test]
fn deep_link_confirm_only_guards_healthy_license_swaps() {
    let licensed = licensed_with(Activation::Activated);
    // The one guarded shape: healthy Licensed + a different incoming id.
    assert!(replacing_healthy_license(
        Some(&licensed),
        Some("01OLD"),
        "01NEW"
    ));
    // Same license id (replayed link) → frictionless.
    assert!(!replacing_healthy_license(
        Some(&licensed),
        Some("01OLD"),
        "01OLD"
    ));
    // No stored key (fresh claim) → frictionless.
    assert!(!replacing_healthy_license(Some(&licensed), None, "01NEW"));
    // Unhealthy states: replacement can only help → frictionless.
    for status in [LicenseStatus::ReadOnly, LicenseStatus::Revoked] {
        assert!(!replacing_healthy_license(
            Some(&status),
            Some("01OLD"),
            "01NEW"
        ));
    }
    assert!(!replacing_healthy_license(None, Some("01OLD"), "01NEW"));
}

// ── license_block: the shared capture-gate decision ────────────────────

// A real key's status always carries the customer email — an empty email
// on `Lapsed` means the trial-issuance ceiling (see `capture_refusal_copy`).
fn licensed_with(activation: Activation) -> LicenseStatus {
    LicenseStatus::Licensed {
        update_through_ms: 0,
        in_window: true,
        email: "owner@example.com".into(),
        name: String::new(),
        activation,
    }
}

#[test]
fn license_block_blocks_with_the_right_code_and_revoked_flag() {
    let block = license_block(Some(&LicenseStatus::ReadOnly), 0).expect("blocked");
    assert_eq!(block.code, "capture_refused_read_only");
    assert!(!block.revoked);

    let block = license_block(Some(&LicenseStatus::Revoked), 0).expect("blocked");
    assert_eq!(block.code, "capture_refused_revoked");
    assert!(block.revoked);

    let block = license_block(Some(&licensed_with(Activation::Lapsed)), 0).expect("blocked");
    assert_eq!(block.code, "capture_refused_unactivated");
    assert!(!block.revoked);

    // A cached Trial whose window has since lapsed blocks at `now` — the
    // first start after expiry must not slip through on the stale cache.
    let stale_trial = LicenseStatus::Trial {
        days_left: 1,
        trial_end_ms: 1_000,
    };
    let block = license_block(Some(&stale_trial), 1_000).expect("blocked");
    assert_eq!(block.code, "capture_refused_read_only");
}

#[test]
fn license_block_allows_running_states_and_never_locks_on_unknown() {
    // Unknown (deferred gate hasn't run) → allow.
    assert_eq!(license_block(None, i64::MAX), None);
    for status in [
        LicenseStatus::TrialNotStarted { trial_days: 30 },
        LicenseStatus::Trial {
            days_left: 5,
            trial_end_ms: i64::MAX,
        },
        licensed_with(Activation::Activated),
        licensed_with(Activation::Pending {
            provisional_days_left: 3,
        }),
        licensed_with(Activation::RefusedOverCap {
            reset_url: "https://reset".into(),
            buy_url: "https://buy".into(),
        }),
    ] {
        assert_eq!(license_block(Some(&status), 0), None, "{status:?}");
    }
}

// ── vet_license_key: the store-or-reject decision ──────────────────────

#[test]
fn vet_rejects_garbage_before_any_store() {
    let err = vet_license_key("garbage", None, |_| Err(licensegate::Error::Envelope))
        .expect_err("garbage must be rejected");
    assert!(err.contains("invalid or corrupted"));
}

#[test]
fn vet_rejects_authentic_but_revoked_with_a_distinct_message() {
    let crl = crl_naming("01REFUNDED");
    let err = vet_license_key("payload.sig", Some(&crl), |_| Ok(key_with_id("01REFUNDED")))
        .expect_err("revoked must be rejected");
    assert!(err.contains("revoked"));
    assert!(!err.contains("invalid"));
}

#[test]
fn vet_accepts_a_valid_unrevoked_key() {
    let crl = crl_naming("01SOMEONE_ELSE");
    let payload = vet_license_key("payload.sig", Some(&crl), |_| Ok(key_with_id("01MINE")))
        .expect("valid key must pass");
    assert_eq!(payload.license_id, "01MINE");
}

#[test]
fn capture_refusal_copy_is_honest_per_state() {
    // Revoked reads as "revoked" with its own code — never trial-ended copy.
    let (code, message) = capture_refusal_copy(&LicenseStatus::Revoked);
    assert_eq!(code, "capture_refused_revoked");
    assert!(message.contains("revoked"));
    assert!(!message.contains("trial"));

    // A lapsed activation reads as the unactivated copy — not trial, not revoked.
    let (code, message) = capture_refusal_copy(&licensed_with(Activation::Lapsed));
    assert_eq!(code, "capture_refused_unactivated");
    assert!(message.contains("activation"));
    assert!(!message.contains("trial"));
    assert!(!message.contains("revoked"));

    // The keyless issuance ceiling (Lapsed with no customer email) reads as
    // "connect once to start your trial" — never the finish-activation copy.
    let (code, message) = capture_refusal_copy(&trial::trial_ceiling_status());
    assert_eq!(code, "capture_refused_trial_unissued");
    assert!(message.contains("trial"));
    assert!(message.contains("connect"));
    assert!(!message.contains("activation"));

    // A lapsed trial / read-only reads as the trial-ended copy.
    for status in [
        LicenseStatus::ReadOnly,
        LicenseStatus::Trial {
            days_left: 0,
            trial_end_ms: 0,
        },
    ] {
        let (code, message) = capture_refusal_copy(&status);
        assert_eq!(code, "capture_refused_read_only");
        assert!(message.contains("trial"));
    }
}

// ── publish: the status reaches the host as `LicenseStatus` ────────────────

#[test]
fn a_computed_status_is_cached_and_emitted_as_license_status() {
    use crate::test_support::{core_in, FakeHost};

    // Debug builds skip the real gate (no `MNEMA_LICENSE_ENFORCE` in tests), so
    // the compute publishes the dev status without touching the pool.
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost::default());
    let core = core_in(host.clone(), dir.path());
    core.manage(LicenseGate(Mutex::new(None)));
    let pool = core.block_on(async { SqlitePool::connect_lazy("sqlite::memory:").unwrap() });

    let status = core.block_on(compute_license_status(&pool, &core, 0));

    assert_eq!(cached_status(&core), Some(status.clone()));
    assert_eq!(
        *host.events.lock().unwrap(),
        vec![CoreEvent::LicenseStatus(
            serde_json::to_value(&status).unwrap()
        )]
    );
}
