//! Offline license verification core (ADR 0045). Pure, dependency-light, and
//! fully unit-testable: no network, no keychain, no DB. Verifies a self-signed
//! Ed25519 license against a hardcoded public key and does the trial-timer math.
//!
//! ## License key wire format (Slice 10 / the Fulfillment minter must match)
//!
//! A license key is the ASCII string:
//!
//! ```text
//!   base64(payload_json) + "." + base64(signature)
//! ```
//!
//! - `base64` is **standard** base64 with padding (RFC 4648, the `STANDARD`
//!   engine), applied to raw bytes.
//! - `payload_json` is the compact (no-whitespace) JSON of [`LicensePayload`]:
//!   `{"email":...,"license_id":...,"tier":...,"issued_at":<i64 unix ms>,
//!   "update_through":<i64 unix ms>}`. Field order/whitespace do not matter for
//!   verification — the signature is checked against the exact transmitted
//!   `payload_json` bytes, then those same bytes are deserialized.
//! - `signature` is the 64-byte Ed25519 signature over the raw UTF-8 bytes of
//!   `payload_json`.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Production Ed25519 public key for offline license verification (ADR 0045).
/// Private key (seed) is held ONLY as the *production* Fulfillment-service
/// secret; rotating it ships a new build. This is the default whenever no
/// build-time override is set — i.e. every release build.
const PRODUCTION_LICENSE_PUBLIC_KEY: [u8; 32] = [
    0xad, 0x46, 0xc9, 0x9c, 0x89, 0xb7, 0x4f, 0xa6, 0x4c, 0x34, 0x2a, 0x04, 0xfb, 0x4c, 0x1d, 0xfd,
    0xe7, 0xf9, 0xa2, 0x59, 0xa8, 0x53, 0xfb, 0x13, 0x93, 0x7f, 0x31, 0x91, 0x73, 0xf6, 0x22, 0x88,
];

/// The public key that licenses **and** the CRL verify against for this build.
///
/// Production by default. A dev/staging build bakes a *different* key by
/// exporting `MNEMA_LICENSE_PUBLIC_KEY` (standard base64 of the 32 raw public
/// key bytes) in the environment at build time — so a key minted by the dev
/// Fulfillment worker (dev seed) verifies only against a dev build, never a
/// shipped one, and vice versa. `app-infra`'s `build.rs` marks the var so a
/// change forces a rebuild. Read via `option_env!`, so the value is fixed into
/// the binary at compile time; an override that isn't valid base64 for exactly
/// 32 bytes is a build-configuration error and panics on first use.
pub fn license_public_key() -> [u8; 32] {
    match option_env!("MNEMA_LICENSE_PUBLIC_KEY") {
        Some(b64) if !b64.trim().is_empty() => {
            let bytes = BASE64
                .decode(b64.trim())
                .expect("MNEMA_LICENSE_PUBLIC_KEY must be valid standard base64");
            bytes
                .try_into()
                .expect("MNEMA_LICENSE_PUBLIC_KEY must decode to exactly 32 bytes")
        }
        _ => PRODUCTION_LICENSE_PUBLIC_KEY,
    }
}

/// Trial length in days (ADR 0044: 30-day Trial).
pub const TRIAL_LEN_DAYS: u32 = 30;

/// Provisional Window length in days (ADR 0053): after a license's first
/// activation attempt that couldn't reach Fulfillment, the app runs
/// provisionally this long before falling to read-only.
pub const PROVISIONAL_WINDOW_DAYS: u32 = 7;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The signed license contents. Ships in every key; verified locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePayload {
    pub email: String,
    pub license_id: String,
    pub tier: String,
    /// Unix milliseconds the license was minted.
    pub issued_at: i64,
    /// Unix milliseconds the paid Update Window runs through.
    pub update_through: i64,
    /// Buyer display name, added after the initial wire shape. `#[serde(default)]`
    /// keeps pre-`name` keys verifying (absent → `None`); `skip_serializing_if`
    /// keeps a `None` name byte-identical to a pre-`name` payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Why a license key failed to parse/verify. Every variant is a rejection —
/// window/expiry logic is the caller's job (Slice 4).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LicenseVerifyError {
    #[error("license key is malformed")]
    Malformed,
    #[error("license key has invalid base64")]
    Base64,
    #[error("license signature is invalid")]
    Signature,
    #[error("license payload is not valid JSON")]
    Json,
}

/// Parse and cryptographically verify a license key against this build's public
/// key ([`license_public_key`]). Returns the verified payload, or a rejection reason.
pub fn parse_and_verify_license(
    key: &str,
) -> std::result::Result<LicensePayload, LicenseVerifyError> {
    // The key bytes are a fixed valid Ed25519 point; treat a decode failure as a
    // signature rejection (only reachable if the baked key were ever corrupted).
    let verifying_key = VerifyingKey::from_bytes(&license_public_key())
        .map_err(|_| LicenseVerifyError::Signature)?;
    parse_and_verify_with_key(key, &verifying_key)
}

/// Inner verifier with an injectable key so the accept path is testable against
/// a test-generated keypair (we do not hold the real private key).
fn parse_and_verify_with_key(
    key: &str,
    verifying_key: &VerifyingKey,
) -> std::result::Result<LicensePayload, LicenseVerifyError> {
    let (payload_b64, signature_b64) = key
        .trim()
        .split_once('.')
        .ok_or(LicenseVerifyError::Malformed)?;

    let payload_bytes = BASE64
        .decode(payload_b64)
        .map_err(|_| LicenseVerifyError::Base64)?;
    let signature_bytes = BASE64
        .decode(signature_b64)
        .map_err(|_| LicenseVerifyError::Base64)?;

    let signature_array: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| LicenseVerifyError::Signature)?;
    let signature = Signature::from_bytes(&signature_array);

    verifying_key
        .verify(&payload_bytes, &signature)
        .map_err(|_| LicenseVerifyError::Signature)?;

    serde_json::from_slice(&payload_bytes).map_err(|_| LicenseVerifyError::Json)
}

/// Days left in the trial. Effective "now" is `max(now_ms, max_seen_ms)` to
/// blunt casual clock-rollback; returns 0 once the window has elapsed.
pub fn trial_days_left(
    trial_started_at_ms: i64,
    now_ms: i64,
    max_seen_ms: i64,
    trial_len_days: u32,
) -> u32 {
    let effective_now = now_ms.max(max_seen_ms);
    let end_ms = trial_started_at_ms + (trial_len_days as i64) * DAY_MS;
    let remaining_ms = end_ms - effective_now;
    if remaining_ms <= 0 {
        return 0;
    }
    // Round up so a partial final day still reads as a day left.
    ((remaining_ms + DAY_MS - 1) / DAY_MS) as u32
}

/// Days left in the Provisional Window — identical shape to [`trial_days_left`]
/// with the same anti-rollback (`max(now, max_seen)`); returns 0 once elapsed.
///
// ponytail: "actual unreachability" is approximated as "time since the first
// activation attempt that never succeeded" — the window is simply 7 days from
// that first attempt for the license id. If we ever need to distinguish "server
// was down" from "user is offline", track attempt outcomes; not worth it now.
pub fn provisional_days_left(
    provisional_started_at_ms: i64,
    now_ms: i64,
    max_seen_ms: i64,
    window_days: u32,
) -> u32 {
    let effective_now = now_ms.max(max_seen_ms);
    let end_ms = provisional_started_at_ms + (window_days as i64) * DAY_MS;
    let remaining_ms = end_ms - effective_now;
    if remaining_ms <= 0 {
        return 0;
    }
    ((remaining_ms + DAY_MS - 1) / DAY_MS) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    // Deterministic test keypair — no RNG needed. This is NOT the production
    // key; the real private key exists only in the Fulfillment service.
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn sample_payload() -> LicensePayload {
        LicensePayload {
            email: "owner@example.com".to_string(),
            license_id: "lic-abc-123".to_string(),
            tier: "standard".to_string(),
            issued_at: 1_700_000_000_000,
            update_through: 1_731_536_000_000,
            name: None,
        }
    }

    fn mint_key(signing_key: &SigningKey, payload: &LicensePayload) -> String {
        let payload_json = serde_json::to_vec(payload).expect("payload should serialize");
        let signature = signing_key.sign(&payload_json);
        format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        )
    }

    /// Cross-language pinned license wire — byte-identical twin of
    /// `PINNED_LICENSE_WIRE` in `services/fulfillment/test/fulfillment.test.ts`
    /// (same `[7u8;32]` seed). Mint must reproduce it and parse+verify must
    /// recover the payload, so `mint.ts` ↔ this verifier can't silently drift
    /// while each side's own round-trip tests stay green. Carries a non-empty
    /// `name` on purpose (the newest wire field).
    const PINNED_LICENSE_WIRE: &str = "eyJlbWFpbCI6ImJ1eWVyQGV4YW1wbGUuY29tIiwibGljZW5zZV9pZCI6Im9yZGVyOjExMTExMTExLTExMTEtMTExMS0xMTExLTExMTExMTExMTExMSIsInRpZXIiOiJsaWNlbnNlIiwiaXNzdWVkX2F0IjoxNzAwMDAwMDAwMDAwLCJ1cGRhdGVfdGhyb3VnaCI6MTczMTUzNjAwMDAwMCwibmFtZSI6IkFkYSBMb3ZlbGFjZSJ9.gf1pCUjfe1cO100kxcHjkOZrQvIa3D3w4WLpl4VsNlYQf0Px3Xx17IGgP6cXLEgRw23KtFjWSAgepW9VZGIlDg==";

    fn pinned_payload() -> LicensePayload {
        LicensePayload {
            email: "buyer@example.com".to_string(),
            license_id: "order:11111111-1111-1111-1111-111111111111".to_string(),
            tier: "license".to_string(),
            issued_at: 1_700_000_000_000,
            update_through: 1_731_536_000_000,
            name: Some("Ada Lovelace".to_string()),
        }
    }

    #[test]
    fn pinned_license_wire_mints_and_verifies_byte_for_byte() {
        let signing_key = test_signing_key();
        // Mint reproduces the TS fixture exactly (field order, compact JSON,
        // standard base64, deterministic Ed25519).
        assert_eq!(mint_key(&signing_key, &pinned_payload()), PINNED_LICENSE_WIRE);
        // And the pinned wire verifies back to the payload.
        assert_eq!(
            parse_and_verify_with_key(PINNED_LICENSE_WIRE, &signing_key.verifying_key()),
            Ok(pinned_payload())
        );
    }

    #[test]
    fn empty_string_name_parses_as_some_empty() {
        // Fulfillment always emits `name` (`?? ""`), so `"name":""` is a real
        // wire shape — it must parse as `Some("")`, distinct from an absent
        // (pre-`name`) key's `None`.
        let json = r#"{"email":"a@b.com","license_id":"lic-1","tier":"standard","issued_at":1,"update_through":2,"name":""}"#;
        let payload: LicensePayload = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(payload.name, Some(String::new()));

        let named = r#"{"email":"a@b.com","license_id":"lic-1","tier":"standard","issued_at":1,"update_through":2,"name":"Zee Shaik"}"#;
        let payload: LicensePayload = serde_json::from_str(named).expect("should deserialize");
        assert_eq!(payload.name.as_deref(), Some("Zee Shaik"));
    }

    #[test]
    fn production_public_key_pin_matches_the_ts_side() {
        // The same base64 literal is asserted against bake-crl.ts's
        // PRODUCTION_PUBLIC_KEY (kept in sync by hand — this is the drift guard).
        assert_eq!(
            BASE64.encode(PRODUCTION_LICENSE_PUBLIC_KEY),
            "rUbJnIm3T6ZMNCoE+0wd/ef5olmoU/sTk38xkXP2Iog="
        );
    }

    #[test]
    fn valid_signature_is_accepted() {
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload = sample_payload();
        let key = mint_key(&signing_key, &payload);

        let verified =
            parse_and_verify_with_key(&key, &verifying_key).expect("valid key should verify");
        assert_eq!(verified, payload);
    }

    #[test]
    fn expired_update_through_still_parses() {
        // Window logic is the caller's job; verification only proves authenticity
        // and that the (past) date is readable.
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let mut payload = sample_payload();
        payload.update_through = 1; // long expired
        let key = mint_key(&signing_key, &payload);

        let verified =
            parse_and_verify_with_key(&key, &verifying_key).expect("should still verify");
        assert_eq!(verified.update_through, 1);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let key = mint_key(&signing_key, &sample_payload());

        // Re-encode a different payload but keep the original signature.
        let (_, signature_b64) = key.split_once('.').unwrap();
        let mut forged = sample_payload();
        forged.tier = "enterprise".to_string();
        let forged_json = serde_json::to_vec(&forged).unwrap();
        let forged_key = format!("{}.{}", BASE64.encode(&forged_json), signature_b64);

        assert_eq!(
            parse_and_verify_with_key(&forged_key, &verifying_key),
            Err(LicenseVerifyError::Signature)
        );
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let key = mint_key(&signing_key, &sample_payload());
        let (payload_b64, _) = key.split_once('.').unwrap();
        // A well-formed but wrong 64-byte signature.
        let bogus_sig = BASE64.encode([0u8; 64]);
        let forged_key = format!("{payload_b64}.{bogus_sig}");

        assert_eq!(
            parse_and_verify_with_key(&forged_key, &verifying_key),
            Err(LicenseVerifyError::Signature)
        );
    }

    #[test]
    fn garbage_and_empty_keys_are_rejected() {
        // Against the real production const — no private key needed for rejections.
        assert_eq!(
            parse_and_verify_license(""),
            Err(LicenseVerifyError::Malformed)
        );
        assert_eq!(
            parse_and_verify_license("no-dot-here"),
            Err(LicenseVerifyError::Malformed)
        );
        assert_eq!(
            parse_and_verify_license("!!!.@@@"),
            Err(LicenseVerifyError::Base64)
        );
        // Valid base64 halves but not a real signature over a real payload.
        let junk = format!("{}.{}", BASE64.encode(b"{}"), BASE64.encode([1u8; 64]));
        assert_eq!(
            parse_and_verify_license(&junk),
            Err(LicenseVerifyError::Signature)
        );
    }

    #[test]
    fn valid_signature_over_non_json_payload_is_rejected_as_json() {
        // A cryptographically valid signature over payload bytes that aren't JSON
        // (a minter signing the wrong bytes) is the ONLY path to the `Json` variant.
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload = b"not json at all";
        let signature = signing_key.sign(payload);
        let key = format!(
            "{}.{}",
            BASE64.encode(payload),
            BASE64.encode(signature.to_bytes())
        );
        assert_eq!(
            parse_and_verify_with_key(&key, &verifying_key),
            Err(LicenseVerifyError::Json)
        );
    }

    #[test]
    fn wrong_length_signature_is_rejected_before_verify() {
        // A signature half that decodes to a valid base64 blob of the wrong length
        // is rejected at `try_into::<[u8; 64]>` — a distinct branch from a verify
        // failure. Use a real payload so only the signature length is wrong.
        let verifying_key = test_signing_key().verifying_key();
        let payload_json = serde_json::to_vec(&sample_payload()).unwrap();
        for bad_len in [32usize, 63, 65] {
            let key = format!(
                "{}.{}",
                BASE64.encode(&payload_json),
                BASE64.encode(vec![0u8; bad_len])
            );
            assert_eq!(
                parse_and_verify_with_key(&key, &verifying_key),
                Err(LicenseVerifyError::Signature),
                "sig length {bad_len} should be rejected"
            );
        }
    }

    #[test]
    fn real_const_rejects_a_test_signed_key() {
        // A key signed by the test keypair must NOT verify against production.
        let key = mint_key(&test_signing_key(), &sample_payload());
        assert_eq!(
            parse_and_verify_license(&key),
            Err(LicenseVerifyError::Signature)
        );
    }

    #[test]
    fn public_key_defaults_to_production_and_is_a_valid_point() {
        // Built without the MNEMA_LICENSE_PUBLIC_KEY override (the normal test /
        // release build), the selector returns the production key and it decodes.
        assert_eq!(license_public_key(), PRODUCTION_LICENSE_PUBLIC_KEY);
        assert!(VerifyingKey::from_bytes(&license_public_key()).is_ok());
    }

    #[test]
    fn trial_days_left_full_at_start() {
        let start = 1_000_000;
        assert_eq!(trial_days_left(start, start, start, TRIAL_LEN_DAYS), 30);
    }

    #[test]
    fn trial_days_left_zero_past_end() {
        let start = 1_000_000;
        let past_end = start + 31 * DAY_MS;
        assert_eq!(
            trial_days_left(start, past_end, past_end, TRIAL_LEN_DAYS),
            0
        );
    }

    #[test]
    fn trial_days_left_uses_max_seen_on_rollback() {
        let start = 1_000_000;
        // Real elapsed time = 25 days, but the clock is rolled back to day 1.
        let max_seen = start + 25 * DAY_MS;
        let rolled_back_now = start + 1 * DAY_MS;
        // Honest math would say ~29 left; anti-rollback pins it to ~5.
        assert_eq!(
            trial_days_left(start, rolled_back_now, max_seen, TRIAL_LEN_DAYS),
            5
        );
    }

    #[test]
    fn trial_days_left_rounds_a_partial_final_day_up() {
        // 25.5 days elapsed of 30 → 4.5 remaining → reads as 5 days left.
        // A floor regression would show 4 and cheat the user of half a day.
        let elapsed = 25 * DAY_MS + DAY_MS / 2;
        assert_eq!(trial_days_left(0, elapsed, elapsed, TRIAL_LEN_DAYS), 5);
    }

    #[test]
    fn provisional_days_left_rounds_a_partial_final_day_up() {
        // 5.5 days elapsed of 7 → 1.5 remaining → reads as 2 days left.
        let elapsed = 5 * DAY_MS + DAY_MS / 2;
        assert_eq!(
            provisional_days_left(0, elapsed, elapsed, PROVISIONAL_WINDOW_DAYS),
            2
        );
    }

    #[test]
    fn provisional_days_left_full_at_start_and_zero_past_end() {
        let start = 1_000_000;
        assert_eq!(
            provisional_days_left(start, start, start, PROVISIONAL_WINDOW_DAYS),
            7
        );
        let past_end = start + 8 * DAY_MS;
        assert_eq!(
            provisional_days_left(start, past_end, past_end, PROVISIONAL_WINDOW_DAYS),
            0
        );
    }

    #[test]
    fn provisional_days_left_uses_max_seen_on_rollback() {
        let start = 1_000_000;
        // Real elapsed = 5 days, but the clock is rolled back to day 1.
        let max_seen = start + 5 * DAY_MS;
        let rolled_back_now = start + 1 * DAY_MS;
        // Honest math would say ~6 left; anti-rollback pins it to ~2.
        assert_eq!(
            provisional_days_left(start, rolled_back_now, max_seen, PROVISIONAL_WINDOW_DAYS),
            2
        );
    }

    #[test]
    fn payload_round_trips_without_name() {
        // A pre-`name` key: JSON with no `name` field deserializes to `None`, and
        // a `None` name serializes back to bytes without a `name` key.
        let json = r#"{"email":"a@b.com","license_id":"lic-1","tier":"standard","issued_at":1,"update_through":2}"#;
        let payload: LicensePayload = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(payload.name, None);
        let reserialized = serde_json::to_string(&payload).unwrap();
        assert!(
            !reserialized.contains("name"),
            "None name must not serialize"
        );
    }

    #[test]
    fn payload_round_trips_with_name() {
        let mut payload = sample_payload();
        payload.name = Some("Ada Lovelace".to_string());
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""name":"Ada Lovelace""#));
        assert_eq!(
            serde_json::from_str::<LicensePayload>(&json).unwrap(),
            payload
        );
    }
}
