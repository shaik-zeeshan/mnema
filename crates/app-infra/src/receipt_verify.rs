//! Offline activation-receipt verification core (ADR 0053). Pure,
//! dependency-light, fully unit-testable: no network, no keychain, no DB.
//! Verifies a signed, **domain-separated** Ed25519 activation receipt against
//! the same public key as [`crate::license_verify`], then checks the receipt is
//! bound to the expected license id and machine hash.
//!
//! ## Receipt wire format (frozen contract; the Fulfillment worker must match)
//!
//! A receipt is the ASCII string:
//!
//! ```text
//!   base64(payload_json) + "." + base64(signature)
//! ```
//!
//! - `base64` is **standard** base64 with padding (RFC 4648, the `STANDARD`
//!   engine), same as a license key / CRL.
//! - `payload_json` is the compact (no-whitespace) JSON of [`Receipt`], field
//!   order `{"schema":1,"license_id":<string>,"machine_hash":<hex string>,
//!   "activated_at":<i64 unix ms>}`. The signature is checked against the exact
//!   transmitted `payload_json` bytes, then those bytes are deserialized.
//! - `signature` is the 64-byte Ed25519 signature over the raw UTF-8 bytes of
//!   `RECEIPT_DOMAIN ++ payload_json` — the [`RECEIPT_DOMAIN`] prefix
//!   domain-separates a receipt from a license key and a CRL so none can replay
//!   as another.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::license_verify::license_public_key;

/// Domain-separation prefix for receipt signatures (ADR 0053). Signing/verifying
/// runs over `RECEIPT_DOMAIN.as_bytes() ++ payload_bytes`, so a license key or a
/// CRL can never verify as a receipt, and vice versa.
pub const RECEIPT_DOMAIN: &str = "mnema-receipt-v1:";

/// The signed activation receipt. Minted once by Fulfillment when a machine
/// activates a license, then cached offline forever (ADR 0053).
///
/// Field declaration order IS the serialized JSON order (serde-derive) — it is
/// pinned by the cross-language wire contract. Do not reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Wire-format version. `1` today; the escape hatch for future shapes.
    pub schema: u32,
    /// License id (`order:<uuid>` / `comp:<slug>`) this receipt activates.
    pub license_id: String,
    /// The bound machine hash (see [`crate::machine_hash`]).
    pub machine_hash: String,
    /// Unix milliseconds the receipt was minted.
    pub activated_at: i64,
}

/// Why a receipt failed to parse/verify. Every variant is a rejection.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReceiptVerifyError {
    #[error("receipt is malformed")]
    Malformed,
    #[error("receipt has invalid base64")]
    Base64,
    #[error("receipt signature is invalid")]
    Signature,
    #[error("receipt payload is not valid JSON")]
    Json,
    #[error("receipt license id does not match")]
    LicenseIdMismatch,
    #[error("receipt machine hash does not match")]
    MachineHashMismatch,
}

/// Parse, cryptographically verify (domain-separated), then bind-check a receipt
/// against the hardcoded production public key. Returns the verified receipt, or
/// a rejection reason. Rejects if the receipt is not for `expected_license_id`
/// or `expected_machine_hash`.
pub fn parse_and_verify_receipt(
    wire: &str,
    expected_license_id: &str,
    expected_machine_hash: &str,
) -> Result<Receipt, ReceiptVerifyError> {
    // The key bytes are a fixed valid Ed25519 point; treat a decode failure as a
    // signature rejection (only reachable if the baked key were ever corrupted).
    let verifying_key = VerifyingKey::from_bytes(&license_public_key())
        .map_err(|_| ReceiptVerifyError::Signature)?;
    parse_and_verify_receipt_with_key(
        wire,
        &verifying_key,
        expected_license_id,
        expected_machine_hash,
    )
}

/// Inner verifier with an injectable key so the accept path is testable against
/// a test-generated keypair (we do not hold the real private key).
fn parse_and_verify_receipt_with_key(
    wire: &str,
    verifying_key: &VerifyingKey,
    expected_license_id: &str,
    expected_machine_hash: &str,
) -> Result<Receipt, ReceiptVerifyError> {
    let (payload_b64, signature_b64) = wire
        .trim()
        .split_once('.')
        .ok_or(ReceiptVerifyError::Malformed)?;

    let payload_bytes = BASE64
        .decode(payload_b64)
        .map_err(|_| ReceiptVerifyError::Base64)?;
    let signature_bytes = BASE64
        .decode(signature_b64)
        .map_err(|_| ReceiptVerifyError::Base64)?;

    let signature_array: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| ReceiptVerifyError::Signature)?;
    let signature = Signature::from_bytes(&signature_array);

    let signed = [RECEIPT_DOMAIN.as_bytes(), &payload_bytes].concat();
    verifying_key
        .verify(&signed, &signature)
        .map_err(|_| ReceiptVerifyError::Signature)?;

    let receipt: Receipt =
        serde_json::from_slice(&payload_bytes).map_err(|_| ReceiptVerifyError::Json)?;

    // Bind checks come AFTER signature verification: an authentic receipt for the
    // wrong license/machine is still a rejection (replay guard).
    if receipt.license_id != expected_license_id {
        return Err(ReceiptVerifyError::LicenseIdMismatch);
    }
    if receipt.machine_hash != expected_machine_hash {
        return Err(ReceiptVerifyError::MachineHashMismatch);
    }
    Ok(receipt)
}

/// The provisional-window record stored under the `activation_state` keychain
/// account: when we first tried and failed to activate this license online, so
/// the app can run provisionally for [`crate::PROVISIONAL_WINDOW_DAYS`] before
/// falling to read-only. Serialized as compact JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationState {
    pub license_id: String,
    pub provisional_started_at_ms: i64,
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

    const SAMPLE_LICENSE_ID: &str = "order:11111111-1111-1111-1111-111111111111";
    const SAMPLE_MACHINE_HASH: &str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sample_receipt() -> Receipt {
        Receipt {
            schema: 1,
            license_id: SAMPLE_LICENSE_ID.to_string(),
            machine_hash: SAMPLE_MACHINE_HASH.to_string(),
            activated_at: 1_700_000_000_000,
        }
    }

    /// Mint a receipt wire string over `RECEIPT_DOMAIN ++ payload` (correct ctx).
    fn mint_receipt(signing_key: &SigningKey, receipt: &Receipt) -> String {
        let payload_json = serde_json::to_vec(receipt).expect("receipt should serialize");
        let signed = [RECEIPT_DOMAIN.as_bytes(), &payload_json].concat();
        let signature = signing_key.sign(&signed);
        format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        )
    }

    // The canonical fixture pinned by the wire spec (shared byte-for-byte with
    // the Worker test suite). Regenerating it must not change this string.
    const PINNED_RECEIPT_WIRE: &str = "eyJzY2hlbWEiOjEsImxpY2Vuc2VfaWQiOiJvcmRlcjoxMTExMTExMS0xMTExLTExMTEtMTExMS0xMTExMTExMTExMTEiLCJtYWNoaW5lX2hhc2giOiJlM2IwYzQ0Mjk4ZmMxYzE0OWFmYmY0Yzg5OTZmYjkyNDI3YWU0MWU0NjQ5YjkzNGNhNDk1OTkxYjc4NTJiODU1IiwiYWN0aXZhdGVkX2F0IjoxNzAwMDAwMDAwMDAwfQ==.09bH7xJoQusxo5FP8VF4VF1VGEZWReZJd5LXDw1aXraBgyo0WrXXzXzJZnCw5hUR2C7W1FeaZPLkOdWx5fQ2CQ==";

    #[test]
    fn pinned_fixture_verifies_and_deserializes() {
        let verifying_key = test_signing_key().verifying_key();
        let receipt = parse_and_verify_receipt_with_key(
            PINNED_RECEIPT_WIRE,
            &verifying_key,
            SAMPLE_LICENSE_ID,
            SAMPLE_MACHINE_HASH,
        )
        .expect("pinned fixture should verify under the test key");
        assert_eq!(receipt, sample_receipt());
        // And the mint helper reproduces the pinned string byte-for-byte.
        assert_eq!(
            mint_receipt(&test_signing_key(), &sample_receipt()),
            PINNED_RECEIPT_WIRE
        );
    }

    #[test]
    fn wrong_license_id_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        assert_eq!(
            parse_and_verify_receipt_with_key(
                PINNED_RECEIPT_WIRE,
                &verifying_key,
                "order:someone-else",
                SAMPLE_MACHINE_HASH,
            ),
            Err(ReceiptVerifyError::LicenseIdMismatch)
        );
    }

    #[test]
    fn wrong_machine_hash_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        assert_eq!(
            parse_and_verify_receipt_with_key(
                PINNED_RECEIPT_WIRE,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                "0000000000000000000000000000000000000000000000000000000000000000",
            ),
            Err(ReceiptVerifyError::MachineHashMismatch)
        );
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        let (_, signature_b64) = PINNED_RECEIPT_WIRE.split_once('.').unwrap();
        let mut forged = sample_receipt();
        forged.activated_at = 42;
        let forged_json = serde_json::to_vec(&forged).unwrap();
        let forged_wire = format!("{}.{}", BASE64.encode(&forged_json), signature_b64);
        assert_eq!(
            parse_and_verify_receipt_with_key(
                &forged_wire,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                SAMPLE_MACHINE_HASH
            ),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        let (payload_b64, _) = PINNED_RECEIPT_WIRE.split_once('.').unwrap();
        let bogus_sig = BASE64.encode([0u8; 64]);
        let forged_wire = format!("{payload_b64}.{bogus_sig}");
        assert_eq!(
            parse_and_verify_receipt_with_key(
                &forged_wire,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                SAMPLE_MACHINE_HASH
            ),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn license_context_signature_does_not_verify_as_receipt() {
        // Sign the SAME payload without the domain prefix (the license context).
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload_json = serde_json::to_vec(&sample_receipt()).unwrap();
        let signature = signing_key.sign(&payload_json); // no RECEIPT_DOMAIN
        let wire = format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        );
        assert_eq!(
            parse_and_verify_receipt_with_key(
                &wire,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                SAMPLE_MACHINE_HASH
            ),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn crl_context_signature_does_not_verify_as_receipt() {
        // A signature over the CRL domain prefix must not verify as a receipt.
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload_json = serde_json::to_vec(&sample_receipt()).unwrap();
        let signed = [crate::CRL_DOMAIN.as_bytes(), &payload_json].concat();
        let signature = signing_key.sign(&signed);
        let wire = format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        );
        assert_eq!(
            parse_and_verify_receipt_with_key(
                &wire,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                SAMPLE_MACHINE_HASH
            ),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn garbage_and_bad_base64_are_rejected() {
        // Against the real production const — no private key needed for rejections.
        assert_eq!(
            parse_and_verify_receipt("", SAMPLE_LICENSE_ID, SAMPLE_MACHINE_HASH),
            Err(ReceiptVerifyError::Malformed)
        );
        assert_eq!(
            parse_and_verify_receipt("no-dot-here", SAMPLE_LICENSE_ID, SAMPLE_MACHINE_HASH),
            Err(ReceiptVerifyError::Malformed)
        );
        assert_eq!(
            parse_and_verify_receipt("!!!.@@@", SAMPLE_LICENSE_ID, SAMPLE_MACHINE_HASH),
            Err(ReceiptVerifyError::Base64)
        );
        let junk = format!("{}.{}", BASE64.encode(b"{}"), BASE64.encode([1u8; 64]));
        assert_eq!(
            parse_and_verify_receipt(&junk, SAMPLE_LICENSE_ID, SAMPLE_MACHINE_HASH),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn valid_signature_over_non_json_payload_is_rejected_as_json() {
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload = b"not json";
        let signed = [RECEIPT_DOMAIN.as_bytes(), payload].concat();
        let signature = signing_key.sign(&signed);
        let wire = format!(
            "{}.{}",
            BASE64.encode(payload),
            BASE64.encode(signature.to_bytes())
        );
        assert_eq!(
            parse_and_verify_receipt_with_key(
                &wire,
                &verifying_key,
                SAMPLE_LICENSE_ID,
                SAMPLE_MACHINE_HASH
            ),
            Err(ReceiptVerifyError::Json)
        );
    }

    #[test]
    fn real_const_rejects_a_test_signed_receipt() {
        // A receipt signed by the test keypair must NOT verify against production.
        let wire = mint_receipt(&test_signing_key(), &sample_receipt());
        assert_eq!(
            parse_and_verify_receipt(&wire, SAMPLE_LICENSE_ID, SAMPLE_MACHINE_HASH),
            Err(ReceiptVerifyError::Signature)
        );
    }

    #[test]
    fn activation_state_round_trips_json() {
        let state = ActivationState {
            license_id: SAMPLE_LICENSE_ID.to_string(),
            provisional_started_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            serde_json::from_str::<ActivationState>(&json).unwrap(),
            state
        );
    }
}
