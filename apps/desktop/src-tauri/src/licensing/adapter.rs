//! licensegate adapter core: the build-time config (public key / kid /
//! publishable token / base URL, all compile-time overridable), and the pure
//! mapping from the crate's `Evaluation` onto the existing `LicenseStatus` /
//! `Activation` wire types (`capture-types/src/licensing.rs` — unchanged by the
//! migration; the frontend never sees licensegate shapes).
//!
//! Mnema owns the clock: callers feed `evaluate()` the *guarded* now
//! (`max(wall clock, max_timestamp_ever_seen_ms)`), so a clock rollback can
//! never stretch a Provisional Window or a trial. The crate's `clock_tampered`
//! flag is log-only and never gates anything.

use capture_types::{Activation, LicenseStatus};
use serde::{Deserialize, Serialize};

/// Provisional Window length in days (ADR 0053 / licensegate config knob N).
pub(crate) const PROVISIONAL_WINDOW_DAYS: u32 = 7;

/// Trial length in days (ADR 0044). Display-only (`TrialNotStarted` promise);
/// the server's trial plan is the enforcement truth — a running trial's
/// countdown comes from the issued key's `app` expiry, never this value.
/// Compile-time override `MNEMA_TRIAL_LEN_DAYS` keeps the promise copy in
/// step when the server plan changes; 30 is the baked fallback.
pub(crate) fn trial_len_days() -> u32 {
    option_env!("MNEMA_TRIAL_LEN_DAYS")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(30)
}

const DAY_MS: i64 = 86_400_000;

// ---------------------------------------------------------------------------
// Build-time config. Release values (prod pubkey/kid, pk_live token, the
// license.mnema.day domain) come from slice 1's server bring-up; until then
// the placeholders below are baked, overridable at compile time exactly like
// the old MNEMA_LICENSE_PUBLIC_KEY split (option_env!, marked in build.rs).
// ---------------------------------------------------------------------------

/// Placeholder verifying key: 32 zero bytes decode to a valid (small-order)
/// Edwards point that `verify_strict` rejects unconditionally — a build
/// without the release override verifies nothing, and nobody can mint a key
/// that passes it. Never shippable by construction.
const PLACEHOLDER_PUBLIC_KEY_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const PLACEHOLDER_KID: &str = "00000000";
const PLACEHOLDER_PK_TOKEN: &str = "pk_placeholder";
const DEFAULT_BASE_URL: &str = "https://license.mnema.day";

fn override_or(value: Option<&'static str>, fallback: &'static str) -> &'static str {
    match value {
        Some(v) if !v.trim().is_empty() => v,
        _ => fallback,
    }
}

fn public_key_hex() -> &'static str {
    static NORMALIZED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NORMALIZED
        .get_or_init(|| {
            let raw = override_or(
                option_env!("MNEMA_LICENSE_PUBLIC_KEY"),
                PLACEHOLDER_PUBLIC_KEY_HEX,
            );
            normalize_public_key(raw)
        })
        .as_str()
}

/// Accept the verifying key as either 64-char hex or base64 (standard, padded
/// or not) of the 32 raw bytes — the licensegate console hands out base64, the
/// crate wants hex. Anything unrecognized passes through untouched so
/// `Verifier::new` still rejects it (verifier() → None, same as before).
fn normalize_public_key(raw: &str) -> String {
    let raw = raw.trim();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD;
    // No ambiguity: 32 raw bytes are 43-44 base64 chars, while valid hex is
    // exactly 64 chars — 64 base64 chars would decode to 48 bytes, never 32.
    match b64.decode(raw.trim_end_matches('=')) {
        Ok(bytes) if bytes.len() == 32 => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        _ => raw.to_string(),
    }
}

fn kid() -> &'static str {
    override_or(option_env!("MNEMA_LICENSE_KID"), PLACEHOLDER_KID)
}

fn pk_token() -> &'static str {
    override_or(option_env!("MNEMA_LICENSE_PK_TOKEN"), PLACEHOLDER_PK_TOKEN)
}

/// One licensegate deployment, two products: dev/debug builds talk to the
/// Polar-sandbox product, release builds to prod. The slug is part of the
/// signature domain, so a sandbox key can never verify on a release build.
pub(crate) fn product_slug() -> &'static str {
    if cfg!(debug_assertions) {
        "mnema-dev"
    } else {
        "mnema"
    }
}

/// Cosmetic license-key prefix (stripped before decode when present).
const DISPLAY_PREFIX: &str = "MNEMA-";

/// The base URL for every licensegate API call, most- to least-specific:
/// runtime `MNEMA_LICENSE_BASE_URL` (debug builds only) → build-time
/// `MNEMA_LICENSE_BASE_URL` → the production domain. Replaces the old
/// `MNEMA_DEV_ACTIVATION_URL`/`MNEMA_ACTIVATION_URL` pair with one knob.
pub(crate) fn base_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("MNEMA_LICENSE_BASE_URL") {
        if !url.trim().is_empty() {
            return url;
        }
    }
    override_or(option_env!("MNEMA_LICENSE_BASE_URL"), DEFAULT_BASE_URL).to_string()
}

/// The one place a `licensegate::Verifier` is built. `None` only on a
/// misconfigured compile-time key override (not valid 64-char hex).
pub(crate) fn verifier() -> Option<licensegate::Verifier> {
    licensegate::Verifier::new(
        product_slug(),
        Some(DISPLAY_PREFIX),
        &[(kid(), public_key_hex())],
    )
    .ok()
}

/// The one place a `licensegate::Client` is built — the activate path uses it
/// today; slices 3–6 (trial, claim, reset, validate) build on the same call.
pub(crate) fn client() -> licensegate::Client {
    licensegate::Client::new(&base_url(), pk_token())
}

// ---------------------------------------------------------------------------
// first_seen_at: stamped by the app when a key is first stored (keychain,
// beside the key), write-once per license id, rollback-guarded (the stamp is
// the guarded now). Feeds the crate's provisional-window math.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FirstSeen {
    pub(crate) license_id: String,
    pub(crate) first_seen_at_ms: i64,
}

/// Pure resolution of a stored first-seen record against a license id: same id
/// → its stamp; a different id, garbage, or no record → `None` (a genuinely
/// new license gets a fresh stamp; a re-paste never resets the clock because
/// the caller only writes when this returns `None`).
pub(crate) fn first_seen_in(record_json: Option<&str>, license_id: &str) -> Option<i64> {
    record_json
        .and_then(|json| serde_json::from_str::<FirstSeen>(json).ok())
        .filter(|record| record.license_id == license_id)
        .map(|record| record.first_seen_at_ms)
}

// ---------------------------------------------------------------------------
// Evaluation → wire mapping.
// ---------------------------------------------------------------------------

/// Map the crate's evaluated state onto the existing wire types.
///
/// | crate `State`        | wire                                             |
/// |----------------------|--------------------------------------------------|
/// | `Revoked`            | `Revoked`                                        |
/// | `Expired`            | `ReadOnly` (reachable only by trial keys — paid  |
/// |                      | keys carry a perpetual `app` entitlement)        |
/// | `Activated`          | `Licensed` + `Activation::Activated`; a          |
/// |                      | trial-shaped key reads as `Trial{days_left,…}`   |
/// | `Provisional{..}`    | `Licensed` + `Pending` (or `RefusedOverCap`)     |
/// | `ActivationRequired` | `Licensed` + `Activation::Lapsed` (read-only     |
/// |                      | behavior, "connect once to finish activation")   |
///
/// `machine_supported == false` (non-macOS: no hardware fingerprint) forces
/// `Activated` — activation can never lock out a platform that can't activate.
pub(crate) fn map_status(
    eval: &licensegate::Evaluation,
    key: &licensegate::LicenseKey,
    guarded_now_ms: i64,
    machine_supported: bool,
    over_cap: Option<(String, String)>,
) -> LicenseStatus {
    use licensegate::State;

    let activation = match &eval.state {
        State::Revoked => return LicenseStatus::Revoked,
        State::Expired => return LicenseStatus::ReadOnly,
        _ if !machine_supported => Activation::Activated,
        // An activated trial key surfaces as `Trial` so the frontend shows the
        // countdown and the gate keeps its stricter live-clock expiry check
        // (`capture_allowed_at`) — the ~24h cached-`Licensed` overshoot is
        // deliberately not granted to the zero-cost path.
        State::Activated => match trial_app_expiry_ms(eval) {
            Some(trial_end_ms) => {
                return LicenseStatus::Trial {
                    days_left: days_left(trial_end_ms, guarded_now_ms),
                    trial_end_ms,
                }
            }
            None => Activation::Activated,
        },
        State::Provisional { activation_due } => match over_cap {
            Some((reset_url, buy_url)) => Activation::RefusedOverCap { reset_url, buy_url },
            None => Activation::Pending {
                provisional_days_left: days_left(*activation_due * 1000, guarded_now_ms),
            },
        },
        State::ActivationRequired => Activation::Lapsed,
    };

    let (update_through_ms, in_window) = update_window(eval);
    LicenseStatus::Licensed {
        update_through_ms,
        in_window,
        email: key.customer.email.clone(),
        name: key.customer.name.clone(),
        activation,
    }
}

/// Synthesize `update_through_ms`/`in_window` from the `updates` entitlement.
/// The entitlement's day-granular expiry lapses at exactly 00:00 UTC — the
/// same instant the crate's own `entitled()` uses — so `in_window` is answered
/// by the crate itself and the ms timestamp is that midnight. No `updates`
/// entitlement (trial keys) or no expiry → never gate updates.
fn update_window(eval: &licensegate::Evaluation) -> (i64, bool) {
    let expires_at = eval
        .entitlements
        .iter()
        .find(|e| e.key == "updates")
        .map(|e| e.expires_at.as_deref());
    match expires_at {
        Some(Some(date)) => match parse_ts_ms(date) {
            Some(ms) => (ms, eval.entitled("updates")),
            // Unparseable expiry in a signature-verified artifact: mirror the
            // crate's permissive treat-as-never-expiring.
            None => (i64::MAX, true),
        },
        Some(None) | None => (i64::MAX, true),
    }
}

/// A trial key is `app` with an expiry — paid keys mint `app` perpetual by
/// construction (slice 1 product config; guarded by the sacred pin below).
/// Read from the governing snapshot, not the raw key, so a re-activated
/// receipt's entitlements decide.
fn trial_app_expiry_ms(eval: &licensegate::Evaluation) -> Option<i64> {
    eval.entitlements
        .iter()
        .find(|e| e.key == "app")
        .and_then(|e| e.expires_at.as_deref())
        .and_then(parse_ts_ms)
}

/// Whole days left until `end_ms`, rounding a partial final day up (never
/// cheat the user of half a day). Shared by the Provisional Window countdown
/// and the trial countdown.
fn days_left(end_ms: i64, guarded_now_ms: i64) -> u32 {
    let remaining_ms = end_ms - guarded_now_ms;
    if remaining_ms <= 0 {
        return 0;
    }
    ((remaining_ms + DAY_MS - 1) / DAY_MS) as u32
}

/// `"YYYY-MM-DD"` or `"YYYY-MM-DDTHH:MM:SSZ"` → unix ms. Mirrors the crate's
/// private `parse_ts` (state.rs) so the synthesized ms instant is exactly the
/// crate's lapse instant. `None` if malformed. Also used by `reset` to turn
/// the server's `retry_at` into a UI-formattable instant.
pub(crate) fn parse_ts_ms(s: &str) -> Option<i64> {
    let field = |range: core::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    let days = days_from_civil(field(0..4)?, field(5..7)?, field(8..10)?);
    let secs = if s.len() >= 19 {
        field(11..13)? * 3600 + field(14..16)? * 60 + field(17..19)?
    } else {
        0
    };
    Some((days * 86_400 + secs) * 1000)
}

// Howard Hinnant's civil-from-days inverse: days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use licensegate::{evaluate, Crl, Customer, Entitlement, LicenseKey, Receipt, State};

    #[test]
    fn public_key_env_accepts_hex_and_base64() {
        let hex = "ad46c99c89b74fa64c342a04fb4c1dfde7f9a259a853fb13937f319173f62288";
        // Hex passes through untouched.
        assert_eq!(normalize_public_key(hex), hex);
        // Base64 of the same 32 bytes (padded or not, with stray whitespace)
        // normalizes to that hex.
        assert_eq!(
            normalize_public_key("rUbJnIm3T6ZMNCoE+0wd/ef5olmoU/sTk38xkXP2Iog="),
            hex
        );
        assert_eq!(
            normalize_public_key(" rUbJnIm3T6ZMNCoE+0wd/ef5olmoU/sTk38xkXP2Iog\n"),
            hex
        );
        // Garbage passes through so Verifier::new still rejects it.
        assert_eq!(normalize_public_key("not-a-key"), "not-a-key");
        assert_eq!(
            normalize_public_key(PLACEHOLDER_PUBLIC_KEY_HEX),
            PLACEHOLDER_PUBLIC_KEY_HEX
        );
    }

    const DAY_S: i64 = 86_400;
    // 2026-07-11T00:00:00Z
    const NOW_S: i64 = 1_783_728_000;
    const NOW_MS: i64 = NOW_S * 1000;

    fn ent(key: &str, expires_at: Option<&str>) -> Entitlement {
        Entitlement {
            key: key.into(),
            expires_at: expires_at.map(str::to_string),
        }
    }

    /// Paid key: perpetual `app`, one-year `updates` — the shape slice 1's
    /// product config mints by construction.
    fn paid_key() -> LicenseKey {
        key_with(vec![ent("app", None), ent("updates", Some("2027-07-11"))])
    }

    /// Trial key: `app` with a 30-day expiry, no `updates` window.
    fn trial_key() -> LicenseKey {
        key_with(vec![ent("app", Some("2026-07-31"))])
    }

    fn key_with(entitlements: Vec<Entitlement>) -> LicenseKey {
        LicenseKey {
            kid: "24f6ed6a".into(),
            license_id: "01JZWXJ3F1V7Q2M8B4N6S9T0CC".into(),
            plan: "pro".into(),
            customer: Customer {
                name: "Renée O'Brien".into(),
                email: "renee@example.com".into(),
            },
            entitlements,
            issued_at: "2026-07-01T00:00:00Z".into(),
        }
    }

    fn receipt_for(key: &LicenseKey) -> Receipt {
        Receipt {
            kid: key.kid.clone(),
            license_id: key.license_id.clone(),
            machine_hash: "aa".repeat(32),
            activated_at: "2026-07-05T00:00:00Z".into(),
            entitlements: key.entitlements.clone(),
        }
    }

    fn crl_naming(id: &str) -> Crl {
        Crl {
            kid: "56475aa7".into(),
            issued_at: "2026-07-10T00:00:00Z".into(),
            revoked_license_ids: vec![id.to_string()],
        }
    }

    fn assert_licensed_with(status: &LicenseStatus, expected: &Activation) {
        match status {
            LicenseStatus::Licensed {
                activation,
                email,
                name,
                ..
            } => {
                assert_eq!(activation, expected);
                assert_eq!(email, "renee@example.com");
                assert_eq!(name, "Renée O'Brien");
            }
            other => panic!("expected Licensed, got {other:?}"),
        }
    }

    // ── the mapping table: every crate State × key-kind ────────────────────

    #[test]
    fn mapping_table_pins_every_state_for_paid_and_trial_keys() {
        for key in [paid_key(), trial_key()] {
            let receipt = receipt_for(&key);

            // Revoked → Revoked (outranks everything).
            let eval = evaluate(
                &key,
                Some(&receipt),
                Some(&crl_naming(&key.license_id)),
                NOW_S - 30 * DAY_S,
                7,
                NOW_S,
            );
            assert_eq!(eval.state, State::Revoked);
            assert_eq!(
                map_status(&eval, &key, NOW_MS, true, None),
                LicenseStatus::Revoked
            );

            // Activated → Licensed + Activated for a paid key; a trial-shaped
            // key (expiring `app`) surfaces as the Trial countdown instead.
            let eval = evaluate(&key, Some(&receipt), None, NOW_S - 30 * DAY_S, 7, NOW_S);
            assert_eq!(eval.state, State::Activated);
            let status = map_status(&eval, &key, NOW_MS, true, None);
            if key.entitlements.iter().any(|e| e.key == "app" && e.expires_at.is_some()) {
                assert_eq!(
                    status,
                    LicenseStatus::Trial {
                        days_left: 20,
                        trial_end_ms: parse_ts_ms("2026-07-31").unwrap(),
                    }
                );
            } else {
                assert_licensed_with(&status, &Activation::Activated);
            }

            // Provisional → Licensed + Pending (days left from activation_due).
            let first_seen = NOW_S - 3 * DAY_S;
            let eval = evaluate(&key, None, None, first_seen, 7, NOW_S);
            assert_eq!(
                eval.state,
                State::Provisional {
                    activation_due: first_seen + 7 * DAY_S
                }
            );
            assert_licensed_with(
                &map_status(&eval, &key, NOW_MS, true, None),
                &Activation::Pending {
                    provisional_days_left: 4,
                },
            );

            // Provisional + over-cap hint → RefusedOverCap surfaces the links.
            assert_licensed_with(
                &map_status(
                    &eval,
                    &key,
                    NOW_MS,
                    true,
                    Some(("https://reset".into(), "https://buy".into())),
                ),
                &Activation::RefusedOverCap {
                    reset_url: "https://reset".into(),
                    buy_url: "https://buy".into(),
                },
            );

            // ActivationRequired → Licensed + Lapsed (read-only behavior with
            // the "connect once to finish activation" copy — capture blocked).
            let eval = evaluate(&key, None, None, NOW_S - 8 * DAY_S, 7, NOW_S);
            assert_eq!(eval.state, State::ActivationRequired);
            let status = map_status(&eval, &key, NOW_MS, true, None);
            assert_licensed_with(&status, &Activation::Lapsed);
            assert!(!status.capture_allowed());
        }

        // Expired → ReadOnly: the trial-expiry path (only trial keys reach it).
        let key = trial_key();
        let past_expiry = NOW_S + 30 * DAY_S;
        let eval = evaluate(&key, None, None, NOW_S, 7, past_expiry);
        assert_eq!(eval.state, State::Expired);
        assert_eq!(
            map_status(&eval, &key, past_expiry * 1000, true, None),
            LicenseStatus::ReadOnly
        );
    }

    #[test]
    fn sacred_pin_a_perpetual_app_key_can_never_map_to_read_only_via_expired() {
        // Guards server-side config fat-fingering: as long as the paid plan
        // mints `app` with no expiry, Expired (→ ReadOnly) is unreachable no
        // matter how far the clock runs or whether a receipt exists.
        let key = paid_key();
        let receipt = receipt_for(&key);
        for years in [0i64, 1, 10, 100] {
            let now = NOW_S + years * 365 * DAY_S;
            for receipt in [None, Some(&receipt)] {
                let eval = evaluate(&key, receipt, None, NOW_S - 30 * DAY_S, 7, now);
                assert_ne!(eval.state, State::Expired, "at +{years}y");
                assert_ne!(
                    map_status(&eval, &key, now * 1000, true, None),
                    LicenseStatus::ReadOnly,
                    "at +{years}y"
                );
            }
        }
    }

    #[test]
    fn non_macos_maps_every_non_terminal_state_to_activated() {
        // No hardware fingerprint → activation can never lock out; Revoked and
        // Expired still apply (they are key states, not machine states).
        let key = paid_key();
        let eval = evaluate(&key, None, None, NOW_S - 8 * DAY_S, 7, NOW_S);
        assert_eq!(eval.state, State::ActivationRequired);
        assert_licensed_with(
            &map_status(&eval, &key, NOW_MS, false, None),
            &Activation::Activated,
        );

        let eval = evaluate(
            &key,
            None,
            Some(&crl_naming(&key.license_id)),
            NOW_S,
            7,
            NOW_S,
        );
        assert_eq!(
            map_status(&eval, &key, NOW_MS, false, None),
            LicenseStatus::Revoked
        );
    }

    // ── clock ownership: the guarded now, not the wall clock, decides ──────

    #[test]
    fn winding_the_clock_back_never_reopens_a_lapsed_provisional_window() {
        let key = paid_key();
        let first_seen = NOW_S - 10 * DAY_S; // window lapsed 3 days ago
        let max_seen_ms = NOW_MS; // high-water mark saw the lapse
        let rolled_back_wall_ms = NOW_MS - 9 * DAY_S * 1000; // wound back inside the window

        // Mnema owns the clock: the guarded now is what reaches evaluate().
        let guarded_now_ms = rolled_back_wall_ms.max(max_seen_ms);
        let eval = evaluate(&key, None, None, first_seen, 7, guarded_now_ms / 1000);
        assert_eq!(eval.state, State::ActivationRequired);
        assert_licensed_with(
            &map_status(&eval, &key, guarded_now_ms, true, None),
            &Activation::Lapsed,
        );
    }

    #[test]
    fn winding_the_clock_back_never_extends_an_expired_trial() {
        let key = trial_key(); // app expires 2026-07-31
        let expiry_ms = parse_ts_ms("2026-07-31").unwrap();
        let max_seen_ms = expiry_ms + DAY_MS; // the lapse was observed
        let rolled_back_wall_ms = NOW_MS; // wound back before expiry

        let guarded_now_ms = rolled_back_wall_ms.max(max_seen_ms);
        let eval = evaluate(&key, None, None, NOW_S, 7, guarded_now_ms / 1000);
        assert_eq!(eval.state, State::Expired);
        assert_eq!(
            map_status(&eval, &key, guarded_now_ms, true, None),
            LicenseStatus::ReadOnly
        );
    }

    #[test]
    fn clock_tampered_flag_never_changes_the_mapping() {
        // A now below the newest held signed timestamp trips the flag; the
        // state — and therefore the wire status — is computed exactly the same.
        let key = paid_key();
        let receipt = receipt_for(&key);
        let crl = crl_naming("someone-else"); // issued 2026-07-10, newest artifact
        let backdated = NOW_S - 5 * DAY_S;

        let eval = evaluate(
            &key,
            Some(&receipt),
            Some(&crl),
            backdated - DAY_S,
            7,
            backdated,
        );
        assert!(eval.clock_tampered);
        assert_licensed_with(
            &map_status(&eval, &key, backdated * 1000, true, None),
            &Activation::Activated,
        );
    }

    // ── update-window synthesis ─────────────────────────────────────────────

    #[test]
    fn update_window_lapses_at_exactly_midnight_utc_with_the_crate() {
        let key = paid_key(); // updates expire 2027-07-11 (day-granular)
        let lapse_s = parse_ts_ms("2027-07-11").unwrap() / 1000;

        // One second before midnight UTC: still in window.
        let eval = evaluate(&key, None, None, lapse_s - DAY_S, 7, lapse_s - 1);
        assert!(eval.entitled("updates"));
        match map_status(&eval, &key, (lapse_s - 1) * 1000, true, None) {
            LicenseStatus::Licensed {
                update_through_ms,
                in_window,
                ..
            } => {
                assert_eq!(update_through_ms, lapse_s * 1000);
                assert!(in_window);
            }
            other => panic!("expected Licensed, got {other:?}"),
        }

        // At exactly 00:00 UTC the crate lapses the entitlement — in_window
        // flips at the same instant, and update_through_ms IS that instant.
        let eval = evaluate(&key, None, None, lapse_s - DAY_S, 7, lapse_s);
        assert!(!eval.entitled("updates"));
        match map_status(&eval, &key, lapse_s * 1000, true, None) {
            LicenseStatus::Licensed {
                update_through_ms,
                in_window,
                ..
            } => {
                assert_eq!(update_through_ms, lapse_s * 1000);
                assert!(!in_window);
            }
            other => panic!("expected Licensed, got {other:?}"),
        }
    }

    #[test]
    fn missing_or_perpetual_updates_entitlement_never_gates() {
        // Trial keys (no `updates` entitlement) and a hypothetical perpetual
        // `updates` both read as forever-in-window — never gate on absent data.
        for key in [
            trial_key(),
            key_with(vec![ent("app", None), ent("updates", None)]),
        ] {
            let eval = evaluate(&key, None, None, NOW_S, 7, NOW_S);
            match map_status(&eval, &key, NOW_MS, true, None) {
                LicenseStatus::Licensed {
                    update_through_ms,
                    in_window,
                    ..
                } => {
                    assert_eq!(update_through_ms, i64::MAX);
                    assert!(in_window);
                }
                other => panic!("expected Licensed, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_ts_ms_matches_known_instants() {
        assert_eq!(parse_ts_ms("1970-01-01"), Some(0));
        assert_eq!(parse_ts_ms("2026-07-11T00:00:00Z"), Some(NOW_MS));
        assert_eq!(parse_ts_ms("2026-07-11"), Some(NOW_MS));
        assert_eq!(
            parse_ts_ms("2026-07-11T01:02:03Z"),
            Some(NOW_MS + 3_723_000)
        );
        assert_eq!(parse_ts_ms("garbage"), None);
    }

    #[test]
    fn days_left_rounds_a_partial_final_day_up() {
        let end_ms = NOW_MS + 2 * DAY_MS;
        // 1.5 days remaining reads as 2 (never cheat the user of half a day).
        assert_eq!(days_left(end_ms, NOW_MS + DAY_MS / 2), 2);
        assert_eq!(days_left(end_ms, NOW_MS), 2);
        assert_eq!(days_left(end_ms, end_ms), 0);
    }

    // ── the running trial: server-issued key → Trial countdown ─────────────

    #[test]
    fn activated_trial_key_anchors_the_countdown_on_the_key_expiry() {
        // The trial clock is the issued key's `app` expiry — nothing local
        // (stamps, constants) moves it; the gate blocks at exactly that instant
        // via the wire type's live-clock check.
        let key = trial_key(); // app expires 2026-07-31
        let end_ms = parse_ts_ms("2026-07-31").unwrap();
        let eval = evaluate(&key, Some(&receipt_for(&key)), None, NOW_S, 7, NOW_S);
        assert_eq!(eval.state, State::Activated);

        let status = map_status(&eval, &key, NOW_MS, true, None);
        assert_eq!(
            status,
            LicenseStatus::Trial {
                days_left: 20,
                trial_end_ms: end_ms,
            }
        );
        assert!(status.capture_allowed_at(end_ms - 1));
        assert!(!status.capture_allowed_at(end_ms));

        // And once the guarded now passes the expiry, the crate itself flips
        // to Expired → ReadOnly (winding the clock back can't resurrect it:
        // the guarded now never goes backwards).
        let past = end_ms / 1000 + 1;
        let eval = evaluate(&key, Some(&receipt_for(&key)), None, NOW_S, 7, past);
        assert_eq!(eval.state, State::Expired);
        assert_eq!(
            map_status(&eval, &key, past * 1000, true, None),
            LicenseStatus::ReadOnly
        );
    }

    // ── first_seen resolution ───────────────────────────────────────────────

    #[test]
    fn first_seen_same_id_reads_different_id_or_garbage_is_absent() {
        let json = serde_json::to_string(&FirstSeen {
            license_id: "01CURRENT".to_string(),
            first_seen_at_ms: 777,
        })
        .unwrap();
        // Same id → the stored stamp (a re-paste never resets the clock).
        assert_eq!(first_seen_in(Some(&json), "01CURRENT"), Some(777));
        // Different id → absent, so a new license gets a fresh stamp.
        assert_eq!(first_seen_in(Some(&json), "01OTHER"), None);
        // No record / garbage → absent.
        assert_eq!(first_seen_in(None, "01CURRENT"), None);
        assert_eq!(first_seen_in(Some("{corrupt"), "01CURRENT"), None);
    }

    // ── config plumbing ─────────────────────────────────────────────────────

    #[test]
    fn placeholder_config_builds_a_verifier_that_accepts_nothing() {
        // The all-zero placeholder is a valid point (Verifier::new succeeds)
        // that verify_strict rejects — a build without release config can
        // never accept a key, rather than failing open or panicking.
        let verifier = verifier().expect("placeholder key must construct");
        assert!(verifier.verify_license("eyJraWQiOiIwMDAwMDAwMCJ9.AA").is_err());
    }

    #[test]
    fn base_url_defaults_to_the_production_domain() {
        // Built without overrides (the normal test build).
        assert_eq!(base_url(), DEFAULT_BASE_URL);
    }
}
