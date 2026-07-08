//! Offline CRL (Certificate/Certificate-style Revocation List) verification core
//! (ADR 0052). Pure, dependency-light, fully unit-testable: no network, no
//! keychain, no DB. Verifies a signed, **domain-separated** Ed25519 revocation
//! document against the same public key as [`crate::license_verify`], and answers
//! the pure `is_revoked(license_id, &crl)` membership question.
//!
//! ## CRL wire format (Slice 2 / the Fulfillment worker must match)
//!
//! A CRL document is the ASCII string:
//!
//! ```text
//!   base64(payload_json) + "." + base64(signature)
//! ```
//!
//! - `base64` is **standard** base64 with padding (RFC 4648, the `STANDARD`
//!   engine), applied to raw bytes — same as a license key.
//! - `payload_json` is the compact (no-whitespace) JSON of [`Crl`]:
//!   `{"schema":1,"issued_at":<i64 unix ms>,"revoked_license_ids":[<string>,...]}`.
//!   Field order/whitespace do not matter for verification — the signature is
//!   checked against the exact transmitted `payload_json` bytes, then those same
//!   bytes are deserialized.
//! - `signature` is the 64-byte Ed25519 signature over the raw UTF-8 bytes of
//!   `CRL_DOMAIN ++ payload_json` — the [`CRL_DOMAIN`] prefix domain-separates a
//!   CRL from a license key so neither can ever replay as the other.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::license_verify::license_public_key;

/// Domain-separation prefix for CRL signatures (ADR 0052). Signing/verifying a
/// CRL runs over `CRL_DOMAIN.as_bytes() ++ payload_bytes`, so a license key
/// (signed over the bare payload) can never verify as a CRL, and vice versa.
pub const CRL_DOMAIN: &str = "mnema-crl-v1:";

/// The signed revocation document. Served verbatim by Fulfillment, cached
/// verbatim by the app, re-verified on every read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crl {
    /// Wire-format version. `1` today; the escape hatch for future shapes.
    pub schema: u32,
    /// Unix milliseconds the document was issued. Monotonic on the worker side;
    /// the app accepts only documents fresher than its cache (rollback-proof).
    pub issued_at: i64,
    /// License ids (`order:<uuid>` / `comp:<slug>`) that are revoked.
    pub revoked_license_ids: Vec<String>,
}

/// Why a CRL document failed to parse/verify. Mirrors
/// [`crate::license_verify::LicenseVerifyError`]; every variant is a rejection.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CrlVerifyError {
    #[error("CRL is malformed")]
    Malformed,
    #[error("CRL has invalid base64")]
    Base64,
    #[error("CRL signature is invalid")]
    Signature,
    #[error("CRL payload is not valid JSON")]
    Json,
}

/// Parse and cryptographically verify a CRL document against the hardcoded
/// production public key (domain-separated). Returns the verified document, or
/// a rejection reason.
pub fn parse_and_verify_crl(wire: &str) -> Result<Crl, CrlVerifyError> {
    // The key bytes are a fixed valid Ed25519 point; treat a decode failure as a
    // signature rejection (only reachable if the baked key were ever corrupted).
    let verifying_key = VerifyingKey::from_bytes(&license_public_key())
        .map_err(|_| CrlVerifyError::Signature)?;
    parse_and_verify_crl_with_key(wire, &verifying_key)
}

/// Inner verifier with an injectable key so the accept path is testable against
/// a test-generated keypair (we do not hold the real private key).
fn parse_and_verify_crl_with_key(
    wire: &str,
    verifying_key: &VerifyingKey,
) -> Result<Crl, CrlVerifyError> {
    let (payload_b64, signature_b64) =
        wire.trim().split_once('.').ok_or(CrlVerifyError::Malformed)?;

    let payload_bytes = BASE64
        .decode(payload_b64)
        .map_err(|_| CrlVerifyError::Base64)?;
    let signature_bytes = BASE64
        .decode(signature_b64)
        .map_err(|_| CrlVerifyError::Base64)?;

    let signature_array: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| CrlVerifyError::Signature)?;
    let signature = Signature::from_bytes(&signature_array);

    let signed = [CRL_DOMAIN.as_bytes(), &payload_bytes].concat();
    verifying_key
        .verify(&signed, &signature)
        .map_err(|_| CrlVerifyError::Signature)?;

    serde_json::from_slice(&payload_bytes).map_err(|_| CrlVerifyError::Json)
}

/// Whether `license_id` appears in the revocation list. Pure membership; policy
/// (what a revoked id means) is the caller's job.
pub fn is_revoked(license_id: &str, crl: &Crl) -> bool {
    crl.revoked_license_ids.iter().any(|id| id == license_id)
}

/// The effective CRL is the freshest of {baked-in floor, fetched cache} by
/// `issued_at` — never rolling back to a shorter list. `None` only when both are
/// absent.
pub fn effective_crl(baked: Option<Crl>, cached: Option<Crl>) -> Option<Crl> {
    match (baked, cached) {
        (Some(b), Some(c)) => Some(if c.issued_at > b.issued_at { c } else { b }),
        (Some(b), None) => Some(b),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    }
}

/// The baked-in revocation floor for fresh installs: the committed
/// `revocations.json` snapshot, signature-verified at runtime exactly like a
/// fetched copy (a tampered build artifact can't inject revocations).
///
/// The committed file is a deliberately **unverifiable placeholder** (empty doc,
/// all-zero signature) so this returns `None` — an empty floor — until release CI
/// (`macos-release.yml`, slice 6) overwrites it with the live signed CRL fetched
/// from the worker. A placeholder that fails verification contributes no
/// revocations and never panics: the safe floor is "revoke nothing".
pub fn baked_crl() -> Option<Crl> {
    parse_and_verify_crl(include_str!("../revocations.json")).ok()
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

    fn sample_crl() -> Crl {
        Crl {
            schema: 1,
            issued_at: 1_700_000_000_000,
            revoked_license_ids: vec!["order:11111111-1111-1111-1111-111111111111".to_string()],
        }
    }

    /// Mint a CRL wire string over `CRL_DOMAIN ++ payload` (the correct context).
    fn mint_crl(signing_key: &SigningKey, crl: &Crl) -> String {
        let payload_json = serde_json::to_vec(crl).expect("crl should serialize");
        let signed = [CRL_DOMAIN.as_bytes(), &payload_json].concat();
        let signature = signing_key.sign(&signed);
        format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        )
    }

    // The canonical fixture pinned by the wire spec (shared byte-for-byte with the
    // Worker test suite). Regenerating it must not change this string.
    const PINNED_WIRE: &str = "eyJzY2hlbWEiOjEsImlzc3VlZF9hdCI6MTcwMDAwMDAwMDAwMCwicmV2b2tlZF9saWNlbnNlX2lkcyI6WyJvcmRlcjoxMTExMTExMS0xMTExLTExMTEtMTExMS0xMTExMTExMTExMTEiXX0=.XjfSyUtXSRRjn6NPWmpGwGMKBwDaXXm1qEj682a4Cdgv4755Df2ZsvRLqJdZVmLVRdAuTBaYUdyEF2xzvXwMBQ==";

    #[test]
    fn pinned_fixture_verifies_and_deserializes() {
        let verifying_key = test_signing_key().verifying_key();
        let crl = parse_and_verify_crl_with_key(PINNED_WIRE, &verifying_key)
            .expect("pinned fixture should verify under the test key");
        assert_eq!(crl, sample_crl());
        // And the mint helper reproduces the pinned string byte-for-byte.
        assert_eq!(mint_crl(&test_signing_key(), &sample_crl()), PINNED_WIRE);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        let (_, signature_b64) = PINNED_WIRE.split_once('.').unwrap();
        // Re-encode a different doc but keep the original signature.
        let mut forged = sample_crl();
        forged.revoked_license_ids = vec!["order:deadbeef".to_string()];
        let forged_json = serde_json::to_vec(&forged).unwrap();
        let forged_wire = format!("{}.{}", BASE64.encode(&forged_json), signature_b64);

        assert_eq!(
            parse_and_verify_crl_with_key(&forged_wire, &verifying_key),
            Err(CrlVerifyError::Signature)
        );
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let verifying_key = test_signing_key().verifying_key();
        let (payload_b64, _) = PINNED_WIRE.split_once('.').unwrap();
        // Well-formed but wrong 64-byte signature.
        let bogus_sig = BASE64.encode([0u8; 64]);
        let forged_wire = format!("{payload_b64}.{bogus_sig}");

        assert_eq!(
            parse_and_verify_crl_with_key(&forged_wire, &verifying_key),
            Err(CrlVerifyError::Signature)
        );
    }

    #[test]
    fn license_context_signature_does_not_verify_as_crl() {
        // Sign the SAME payload without the domain prefix (i.e. the license
        // context). It must NOT verify as a CRL — cross-replay guard.
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload_json = serde_json::to_vec(&sample_crl()).unwrap();
        let signature = signing_key.sign(&payload_json); // no CRL_DOMAIN prefix
        let wire = format!(
            "{}.{}",
            BASE64.encode(&payload_json),
            BASE64.encode(signature.to_bytes())
        );

        assert_eq!(
            parse_and_verify_crl_with_key(&wire, &verifying_key),
            Err(CrlVerifyError::Signature)
        );
    }

    #[test]
    fn valid_signature_over_non_json_payload_is_rejected_as_json() {
        // Correct domain-prefixed signature over non-JSON bytes → `Json`.
        let signing_key = test_signing_key();
        let verifying_key = signing_key.verifying_key();
        let payload = b"not json";
        let signed = [CRL_DOMAIN.as_bytes(), payload].concat();
        let signature = signing_key.sign(&signed);
        let wire = format!(
            "{}.{}",
            BASE64.encode(payload),
            BASE64.encode(signature.to_bytes())
        );
        assert_eq!(
            parse_and_verify_crl_with_key(&wire, &verifying_key),
            Err(CrlVerifyError::Json)
        );
    }

    #[test]
    fn wrong_length_signature_is_rejected_before_verify() {
        let verifying_key = test_signing_key().verifying_key();
        let payload_json = serde_json::to_vec(&sample_crl()).unwrap();
        for bad_len in [32usize, 63, 65] {
            let wire = format!(
                "{}.{}",
                BASE64.encode(&payload_json),
                BASE64.encode(vec![0u8; bad_len])
            );
            assert_eq!(
                parse_and_verify_crl_with_key(&wire, &verifying_key),
                Err(CrlVerifyError::Signature),
                "sig length {bad_len} should be rejected"
            );
        }
    }

    #[test]
    fn effective_crl_keeps_baked_on_equal_issued_at() {
        // Strict `>` means a tie keeps the baked floor. Pin it so a `>=` refactor
        // (which would flip tie behavior) fails.
        let baked = Crl {
            schema: 1,
            issued_at: 100,
            revoked_license_ids: vec!["order:baked".to_string()],
        };
        let cached = Crl {
            schema: 1,
            issued_at: 100,
            revoked_license_ids: vec!["order:cached".to_string()],
        };
        assert_eq!(
            effective_crl(Some(baked.clone()), Some(cached)),
            Some(baked)
        );
    }

    #[test]
    fn garbage_and_bad_base64_are_rejected() {
        // Against the real production const — no private key needed for rejections.
        assert_eq!(parse_and_verify_crl(""), Err(CrlVerifyError::Malformed));
        assert_eq!(
            parse_and_verify_crl("no-dot-here"),
            Err(CrlVerifyError::Malformed)
        );
        assert_eq!(parse_and_verify_crl("!!!.@@@"), Err(CrlVerifyError::Base64));
        // Valid base64 halves but not a real signature over a real payload.
        let junk = format!("{}.{}", BASE64.encode(b"{}"), BASE64.encode([1u8; 64]));
        assert_eq!(parse_and_verify_crl(&junk), Err(CrlVerifyError::Signature));
    }

    #[test]
    fn real_const_rejects_a_test_signed_crl() {
        // A CRL signed by the test keypair must NOT verify against production.
        let wire = mint_crl(&test_signing_key(), &sample_crl());
        assert_eq!(parse_and_verify_crl(&wire), Err(CrlVerifyError::Signature));
    }

    #[test]
    fn is_revoked_hit_and_miss() {
        let crl = sample_crl();
        assert!(is_revoked(
            "order:11111111-1111-1111-1111-111111111111",
            &crl
        ));
        assert!(!is_revoked("order:not-in-the-list", &crl));
        // Empty list revokes nothing.
        let empty = Crl {
            schema: 1,
            issued_at: 0,
            revoked_license_ids: vec![],
        };
        assert!(!is_revoked("order:anything", &empty));
    }

    #[test]
    fn effective_crl_picks_the_fresher_document() {
        let older = Crl {
            schema: 1,
            issued_at: 100,
            revoked_license_ids: vec!["order:old".to_string()],
        };
        let newer = Crl {
            schema: 1,
            issued_at: 200,
            revoked_license_ids: vec!["order:new".to_string()],
        };

        // Freshest wins regardless of argument position (rollback-proof pick).
        assert_eq!(
            effective_crl(Some(older.clone()), Some(newer.clone())),
            Some(newer.clone())
        );
        assert_eq!(
            effective_crl(Some(newer.clone()), Some(older.clone())),
            Some(newer.clone())
        );
        // One-None and both-None.
        assert_eq!(effective_crl(Some(older.clone()), None), Some(older.clone()));
        assert_eq!(effective_crl(None, Some(newer.clone())), Some(newer));
        assert_eq!(effective_crl(None, None), None);
    }

    #[test]
    fn baked_placeholder_returns_none_without_panicking() {
        // The committed placeholder is deliberately unverifiable → empty floor.
        assert_eq!(baked_crl(), None);
    }

    #[test]
    fn test_signed_snapshot_verifies_and_is_revoked() {
        // Proves the mechanism baked_crl() uses: a properly signed snapshot,
        // parsed via the injectable inner fn, verifies and answers membership.
        let verifying_key = test_signing_key().verifying_key();
        let wire = mint_crl(&test_signing_key(), &sample_crl());
        let crl = parse_and_verify_crl_with_key(&wire, &verifying_key)
            .expect("test-signed snapshot should verify");
        assert!(is_revoked(
            "order:11111111-1111-1111-1111-111111111111",
            &crl
        ));
    }
}
