//! Over-cap self-service (plan slice 6): "Free up my devices" — key-authed
//! `POST /v1/reset` that empties the license's slot set, then auto-retries the
//! ordinary activation chain — and the Settings device count from
//! `POST /v1/validate` (`devices {used, cap}`). Both surface a COUNT only,
//! never a device list: the published privacy commitment ("no device names
//! sent or stored") stays true word-for-word.

use capture_types::{LicenseDevices, ResetDevicesOutcome};

use super::{activation, adapter};

/// What to do with the reset call's outcome — pure and IO-free
/// (`reset_license_devices` is the shell that executes it).
#[derive(Debug, PartialEq, Eq)]
enum ResetDisposition {
    /// Slots emptied: chain the ordinary background activation (which stores
    /// the receipt, clears the over-cap hint, and recomputes on success).
    RetryActivation,
    /// Reset cooldown (once per 30 days); `retry_at_ms` is when it reopens.
    RateLimited { retry_at_ms: Option<i64> },
    /// Anything else (dead license, transport, unknown refusals): surface a
    /// human-readable error and change nothing.
    Fail(String),
}

/// Classify a reset result. Pure so the outcome table is pinnable without HTTP
/// (same shape as `activation::classify_activation`).
fn classify_reset(
    result: Result<licensegate::ResetOutcome, licensegate::ApiError>,
) -> ResetDisposition {
    use licensegate::{ApiError, RefusalCode};
    match result {
        Ok(_) => ResetDisposition::RetryActivation,
        Err(ApiError::Refused {
            code: RefusalCode::ResetRateLimited { retry_at },
            ..
        }) => ResetDisposition::RateLimited {
            retry_at_ms: retry_at.as_deref().and_then(adapter::parse_ts_ms),
        },
        Err(ApiError::Refused { message, .. }) => ResetDisposition::Fail(if message.is_empty() {
            "The license server refused the reset. Try again later.".to_string()
        } else {
            message
        }),
        Err(ApiError::Transport(_)) => ResetDisposition::Fail(
            "Couldn't reach the license server — check your connection and try again.".to_string(),
        ),
    }
}

/// The devices count in a validate result: present only when the key resolved
/// to a license. Any failure → `None` (the UI shows nothing, never stale
/// numbers).
fn devices_in(
    result: Result<licensegate::Validation, licensegate::ApiError>,
) -> Option<LicenseDevices> {
    result.ok()?.license.map(|license| LicenseDevices {
        used: license.devices.used,
        cap: license.devices.cap,
    })
}

/// "Free up my devices" on the over-cap screen: reset (authorized by key
/// possession), then immediately re-run the same activation chain the paste /
/// launch / daily-tick paths use, so the freed slot is reclaimed by this
/// machine in the same breath and the `license_status` event flips the UI.
#[tauri::command]
pub async fn reset_license_devices(
    app_handle: tauri::AppHandle,
) -> Result<ResetDevicesOutcome, String> {
    let Some(key_wire) = app_infra::load_license_key().ok().flatten() else {
        return Err("No license key is stored on this device.".to_string());
    };
    match classify_reset(adapter::client().reset(&key_wire).await) {
        ResetDisposition::RetryActivation => {
            // Awaited (not spawned) so a same-breath success returns with the
            // receipt already stored and the status already republished.
            activation::run_activation(app_handle.clone()).await;
            Ok(ResetDevicesOutcome::Reset)
        }
        ResetDisposition::RateLimited { retry_at_ms } => {
            Ok(ResetDevicesOutcome::RateLimited { retry_at_ms })
        }
        ResetDisposition::Fail(message) => Err(message),
    }
}

/// Device count for the Settings licensing panel, fetched lazily when the
/// panel shows. `Ok(None)` — render nothing — when there is no stored key,
/// the server is unreachable, or the key didn't resolve to a license.
#[tauri::command]
pub async fn get_license_devices() -> Result<Option<LicenseDevices>, String> {
    let Some(key_wire) = app_infra::load_license_key().ok().flatten() else {
        return Ok(None);
    };
    Ok(devices_in(adapter::client().validate(&key_wire, None).await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use licensegate::{ApiError, RefusalCode, ResetOutcome, Validation, ValidationCode};

    fn refused(code: RefusalCode) -> Result<ResetOutcome, ApiError> {
        Err(ApiError::Refused {
            code,
            message: "server says no".to_string(),
        })
    }

    #[test]
    fn success_retries_activation() {
        assert_eq!(
            classify_reset(Ok(ResetOutcome {
                reset_at: "2026-07-16T00:00:00Z".into(),
                next_reset_available_at: "2026-08-15T00:00:00Z".into(),
            })),
            ResetDisposition::RetryActivation,
        );
    }

    #[test]
    fn rate_limited_surfaces_the_servers_retry_at() {
        assert_eq!(
            classify_reset(refused(RefusalCode::ResetRateLimited {
                retry_at: Some("2026-08-10T00:00:00Z".into()),
            })),
            ResetDisposition::RateLimited {
                retry_at_ms: adapter::parse_ts_ms("2026-08-10T00:00:00Z"),
            },
        );
        // Missing or malformed retry_at degrades to None — the UI still shows
        // the cooldown copy, just without a date.
        for retry_at in [None, Some("not-a-date".to_string())] {
            assert_eq!(
                classify_reset(refused(RefusalCode::ResetRateLimited { retry_at })),
                ResetDisposition::RateLimited { retry_at_ms: None },
            );
        }
    }

    #[test]
    fn other_refusals_and_transport_fail_with_readable_copy() {
        // Refusals carry the server's message through.
        for code in [
            RefusalCode::LicenseRevoked,
            RefusalCode::LicenseNotFound,
            RefusalCode::Unauthorized,
            RefusalCode::Other("brand_new".into()),
        ] {
            assert_eq!(
                classify_reset(refused(code.clone())),
                ResetDisposition::Fail("server says no".to_string()),
                "{code:?}"
            );
        }
        // An empty server message never surfaces as a blank error.
        match classify_reset(Err(ApiError::Refused {
            code: RefusalCode::InvalidRequest,
            message: String::new(),
        })) {
            ResetDisposition::Fail(message) => assert!(!message.is_empty()),
            other => panic!("expected Fail, got {other:?}"),
        }
        // Transport reads as a connectivity problem, not a server refusal.
        match classify_reset(Err(ApiError::Transport("timeout".into()))) {
            ResetDisposition::Fail(message) => assert!(message.contains("connection")),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    fn validation(license: Option<licensegate::LicenseInfo>) -> Validation {
        Validation {
            valid: license.is_some(),
            code: ValidationCode::Valid,
            message: None,
            warnings: vec![],
            license,
        }
    }

    #[test]
    fn devices_in_reads_the_count_and_never_invents_one() {
        let license = licensegate::LicenseInfo {
            license_id: "01J".into(),
            plan: "pro".into(),
            status: "active".into(),
            expires_at: None,
            entitlements: vec![],
            devices: licensegate::Devices { used: 2, cap: 3 },
        };
        assert_eq!(
            devices_in(Ok(validation(Some(license)))),
            Some(LicenseDevices { used: 2, cap: 3 }),
        );
        // Key didn't resolve to a license (e.g. invalid_signature) → nothing.
        assert_eq!(devices_in(Ok(validation(None))), None);
        // Offline/unreachable → nothing, never stale numbers.
        assert_eq!(devices_in(Err(ApiError::Transport("offline".into()))), None);
    }
}
