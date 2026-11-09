//! The first half of the install gate: the archive's minisign signature, made by
//! the release pipeline's updater key (tauri's signer, `TAURI_SIGNING_PRIVATE_KEY`).

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};

/// `tauri.conf.json` `plugins.updater.pubkey`: base64 of the minisign public key
/// file for key id `F6BBB53B2FAA615D`. The Tauri app verifies with the same key.
pub const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY2QkJCNTNCMkZBQTYxNUQKUldSZFlhb3ZPN1c3OXJSUkNxTG9ZWW1rcnM5Yk1Mclk0Nmh3U08wRDhWeTZxV2JkMTNnbUtjS2QK";

fn decode_text(b64: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Whether `signature_b64` (a base64 minisign signature file, as in the feed) is
/// a valid signature of `data` by `pubkey_b64`. Prehashed and legacy signatures
/// are both accepted, like tauri-plugin-updater.
pub fn signature_is_valid(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> bool {
    let (Some(key), Some(sig)) = (decode_text(pubkey_b64), decode_text(signature_b64)) else {
        return false;
    };
    let (Ok(key), Ok(sig)) = (PublicKey::decode(&key), Signature::decode(&sig)) else {
        return false;
    };
    key.verify(data, &sig, true).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway test key (Ed25519 seed [7; 32], key id EFCDAB8967452301) and its
    // prehashed ("ED", BLAKE2b-512) signature of FIXTURE, made with PyNaCl.
    const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVGQ0RBQjg5Njc0NTIzMDEKUldRQkkwVm5pYXZONytwS2JHUGluRklLdnZWUWV4TXV4Zm1WUjNhdXZyNTdra0llNm1rVVJ0SXMK";
    const FIXTURE_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRlc3Qga2V5ClJVUUJJMFZuaWF2Tjc5K3ZJLy9mVVRDMjNoTHNNLzBDd2Z0YlA4ZXlEYk5qcW04eStVKzNGUUV2QXlONjRXNmtpVExUVlJTdVJkN3N5WXFqaGFpdWdlUSttb3NXWitpWkhRZz0KdHJ1c3RlZCBjb21tZW50OiB0aW1lc3RhbXA6MTcwMDAwMDAwMAlmaWxlOmZpeHR1cmUudGFyLmd6CjlUVnlUWDRIRmhXVmlZNWRjRFhzSXk2d0RyNUhVMVBlWndYNktuc3JBOXNSRlhtdnpsT2RYazRRYlNSR0szcWZXLzdKMjc4ekJLNTcwUlh6UUpnL0FnPT0K";
    const FIXTURE: &[u8] = b"mnema update fixture\n";

    #[test]
    fn a_fixture_signed_with_the_test_key_passes() {
        assert!(signature_is_valid(FIXTURE, FIXTURE_SIG, TEST_PUBKEY));
    }

    #[test]
    fn one_changed_byte_fails() {
        let mut data = FIXTURE.to_vec();
        data[0] ^= 1;
        assert!(!signature_is_valid(&data, FIXTURE_SIG, TEST_PUBKEY));
    }

    #[test]
    fn the_release_key_does_not_accept_a_test_signature() {
        assert!(!signature_is_valid(FIXTURE, FIXTURE_SIG, UPDATER_PUBKEY));
    }

    #[test]
    fn garbage_signatures_fail_closed() {
        assert!(!signature_is_valid(FIXTURE, "", TEST_PUBKEY));
        assert!(!signature_is_valid(FIXTURE, "not base64!", TEST_PUBKEY));
        assert!(!signature_is_valid(FIXTURE, FIXTURE_SIG, "AAAA"));
    }

    #[test]
    fn the_release_key_is_f6bbb53b2faa615d() {
        let text = decode_text(UPDATER_PUBKEY).expect("base64 text");
        assert!(text.contains("minisign public key: F6BBB53B2FAA615D"));
        PublicKey::decode(&text).expect("a minisign public key");
    }
}
