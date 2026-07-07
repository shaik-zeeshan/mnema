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

/// Ed25519 public key for offline license verification (ADR 0045).
/// Private key is held ONLY as a Fulfillment-service secret; rotating it ships a new build.
pub const LICENSE_PUBLIC_KEY: [u8; 32] = [
    0x60, 0x7b, 0x11, 0x3a, 0x84, 0x46, 0x64, 0xe5, 0xfc, 0xae, 0x01, 0x84, 0xf0, 0xc9, 0x7e, 0xe4,
    0x6c, 0xde, 0x69, 0xb9, 0x40, 0xdd, 0xc6, 0xde, 0xa8, 0x46, 0xb8, 0x2f, 0x88, 0x4c, 0xe2, 0x12,
];

/// Trial length in days (ADR 0044: 30-day Trial).
pub const TRIAL_LEN_DAYS: u32 = 30;

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

/// Parse and cryptographically verify a license key against the hardcoded
/// production public key. Returns the verified payload, or a rejection reason.
pub fn parse_and_verify_license(key: &str) -> std::result::Result<LicensePayload, LicenseVerifyError> {
    // The const is a fixed valid Ed25519 point; treat a decode failure as a
    // signature rejection (only reachable if the const were ever corrupted).
    let verifying_key =
        VerifyingKey::from_bytes(&LICENSE_PUBLIC_KEY).map_err(|_| LicenseVerifyError::Signature)?;
    parse_and_verify_with_key(key, &verifying_key)
}

/// Inner verifier with an injectable key so the accept path is testable against
/// a test-generated keypair (we do not hold the real private key).
fn parse_and_verify_with_key(
    key: &str,
    verifying_key: &VerifyingKey,
) -> std::result::Result<LicensePayload, LicenseVerifyError> {
    let (payload_b64, signature_b64) =
        key.trim().split_once('.').ok_or(LicenseVerifyError::Malformed)?;

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

        let verified = parse_and_verify_with_key(&key, &verifying_key).expect("should still verify");
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
    fn real_const_rejects_a_test_signed_key() {
        // A key signed by the test keypair must NOT verify against production.
        let key = mint_key(&test_signing_key(), &sample_payload());
        assert_eq!(
            parse_and_verify_license(&key),
            Err(LicenseVerifyError::Signature)
        );
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
        assert_eq!(trial_days_left(start, past_end, past_end, TRIAL_LEN_DAYS), 0);
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
}
