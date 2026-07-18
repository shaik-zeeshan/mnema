//! Machine fingerprint for once-per-machine activation (ADR 0053). macOS-only:
//! the stable hardware UUID via `gethostuuid(2)` — one libc call, no shelling
//! out (a packaged app has no Homebrew PATH). Other platforms error; activation
//! is macOS-only today (see `SUPPORTS.md`). The salted machine-hash derivations
//! live in the `licensegate` client crate (`machine_hash` / `trial_machine_hash`).

use crate::error::{AppInfraError, Result};

/// The machine's stable hardware UUID (uppercase, dashed canonical form).
/// macOS returns the same value across reboots, OS reinstalls, and factory
/// resets (it lives in hardware, so re-activation after an erase is free —
/// ADR 0053's idempotency depends on this); only a logic-board swap changes
/// it (acceptable — that's effectively a different machine).
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

/// The generic hardware model identifier ("Mac15,7") via `sysctl hw.model` —
/// the device label sent on activation so the seller dashboard can tell a
/// license's devices apart (ADR 0055). Never the personal computer name.
#[cfg(target_os = "macos")]
pub fn hardware_model() -> Result<String> {
    let name = c"hw.model";
    let mut len: libc::size_t = 0;
    // SAFETY: a null buffer asks sysctl for the value's length only.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return Err(AppInfraError::LicenseTokenStore(format!(
            "sysctl hw.model length probe failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` is a live buffer of exactly the length sysctl reported.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(AppInfraError::LicenseTokenStore(format!(
            "sysctl hw.model read failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    buf.truncate(len);
    // sysctl returns a NUL-terminated string; drop the terminator.
    if buf.last() == Some(&0) {
        buf.pop();
    }
    String::from_utf8(buf)
        .map_err(|_| AppInfraError::LicenseTokenStore("hw.model is not utf-8".to_string()))
}

#[cfg(not(target_os = "macos"))]
pub fn hardware_model() -> Result<String> {
    Err(AppInfraError::LicenseTokenStore(
        "hardware model / activation is unsupported on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(target_os = "macos")]
    #[test]
    fn hardware_model_is_nonempty_without_nul() {
        let model = hardware_model().expect("macOS should return a hardware model");
        assert!(!model.is_empty());
        assert!(!model.contains('\0'));
    }
}
