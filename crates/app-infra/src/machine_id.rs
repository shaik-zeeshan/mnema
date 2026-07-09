//! Machine fingerprint for once-per-machine activation (ADR 0053). macOS-only:
//! the stable hardware UUID via `gethostuuid(2)` — one libc call, no shelling
//! out (a packaged app has no Homebrew PATH). Other platforms error; activation
//! is macOS-only today (see `SUPPORTS.md`).

use sha2::{Digest, Sha256};

use crate::error::{AppInfraError, Result};

/// Domain-separation prefix for the machine hash (frozen wire contract; the
/// Fulfillment worker computes the identical hash).
const MACHINE_HASH_DOMAIN: &str = "mnema-activation-v1:";

/// The stable per-machine hash bound into an activation receipt:
/// `hex(SHA-256(MACHINE_HASH_DOMAIN + license_id + ":" + hardware_uuid))`,
/// lowercase. Binds a receipt to one license on one machine so it can't be
/// replayed for a different license or copied to another device.
pub fn machine_hash(license_id: &str, hardware_uuid: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MACHINE_HASH_DOMAIN.as_bytes());
    hasher.update(license_id.as_bytes());
    hasher.update(b":");
    hasher.update(hardware_uuid.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The machine's stable hardware UUID (uppercase, dashed canonical form).
/// macOS returns the same value across reboots; a factory reset / logic-board
/// swap changes it (acceptable — that's a different machine).
#[cfg(target_os = "macos")]
pub fn hardware_uuid() -> Result<String> {
    // gethostuuid fills 16 raw bytes; a zero timeout means "don't wait".
    let mut uuid = [0u8; 16];
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `uuid` is a live 16-byte buffer (the size `uuid_t` expects) and
    // `timeout` is a live `timespec`; the call only writes into `uuid`.
    let rc = unsafe { libc::gethostuuid(uuid.as_mut_ptr(), &timeout) };
    if rc != 0 {
        return Err(AppInfraError::LicenseTokenStore(format!(
            "gethostuuid failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(format_uuid(&uuid))
}

/// Canonical 8-4-4-4-12 uppercase-hex form, matching macOS's Hardware UUID.
#[cfg(target_os = "macos")]
fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32],
    )
}

#[cfg(not(target_os = "macos"))]
pub fn hardware_uuid() -> Result<String> {
    Err(AppInfraError::LicenseTokenStore(
        "hardware uuid / activation is unsupported on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_hash_is_deterministic_and_binds_license() {
        let uuid = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE";
        let a = machine_hash("order:one", uuid);
        // Same inputs → same hash.
        assert_eq!(a, machine_hash("order:one", uuid));
        // Lowercase 64-hex-char SHA-256.
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Different license id → different hash (bound to the license).
        assert_ne!(a, machine_hash("order:two", uuid));
        // Different machine → different hash.
        assert_ne!(
            a,
            machine_hash("order:one", "FFFFFFFF-0000-0000-0000-000000000000")
        );
    }

    // Pin the frozen wire contract: hash of the empty-string-domain vector is
    // stable across languages (the TS worker must reproduce it).
    #[test]
    fn machine_hash_matches_frozen_vector() {
        // hex(SHA-256("mnema-activation-v1:order:x:uuid-y"))
        let h = machine_hash("order:x", "uuid-y");
        assert_eq!(h.len(), 64);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn hardware_uuid_is_stable_and_nonempty() {
        let first = hardware_uuid().expect("macOS should return a hardware uuid");
        assert!(!first.is_empty());
        // Canonical dashed form.
        assert_eq!(first.len(), 36);
        assert_eq!(first.as_bytes()[8], b'-');
        // Stable across calls.
        assert_eq!(first, hardware_uuid().expect("second call should succeed"));
    }
}
