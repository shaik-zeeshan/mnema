use serde::{Deserialize, Serialize};

/// Offline license/trial status, computed by the gate and surfaced to the UI.
/// `ReadOnly` is the ONLY capture-blocking state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LicenseStatus {
    /// Trial clock not yet started (no successful Capture yet). Capture allowed.
    TrialNotStarted { trial_days: u32 },
    /// Trial running. Capture allowed.
    Trial { days_left: u32, trial_end_ms: i64 },
    /// Trial expired, unlicensed. Capture disabled; reads untouched.
    ReadOnly,
    /// Owns a license. Capture always allowed; `in_window` gates only new builds.
    Licensed {
        update_through_ms: i64,
        in_window: bool,
        email: String,
    },
}

impl LicenseStatus {
    /// The single gate question: may forward Capture run?
    pub fn capture_allowed(&self) -> bool {
        !matches!(self, LicenseStatus::ReadOnly)
    }
}

/// Result of pasting a license key into Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateLicenseResult {
    pub status: LicenseStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_serializes_as_kind_camel_case() {
        let cases = [
            (LicenseStatus::TrialNotStarted { trial_days: 14 }, "trialNotStarted"),
            (
                LicenseStatus::Trial {
                    days_left: 3,
                    trial_end_ms: 1,
                },
                "trial",
            ),
            (LicenseStatus::ReadOnly, "readOnly"),
            (
                LicenseStatus::Licensed {
                    update_through_ms: 1,
                    in_window: true,
                    email: "a@b.c".to_string(),
                },
                "licensed",
            ),
        ];
        for (status, tag) in cases {
            let json = serde_json::to_value(&status).expect("serialize");
            assert_eq!(json["kind"], tag);
        }
    }

    #[test]
    fn trial_fields_serialize_camel_case() {
        let json = serde_json::to_value(LicenseStatus::Trial {
            days_left: 7,
            trial_end_ms: 42,
        })
        .expect("serialize");
        assert_eq!(json["daysLeft"], 7);
        assert_eq!(json["trialEndMs"], 42);
    }

    #[test]
    fn licensed_fields_serialize_camel_case() {
        let json = serde_json::to_value(LicenseStatus::Licensed {
            update_through_ms: 99,
            in_window: false,
            email: "x@y.z".to_string(),
        })
        .expect("serialize");
        assert_eq!(json["updateThroughMs"], 99);
        assert_eq!(json["inWindow"], false);
        assert_eq!(json["email"], "x@y.z");
    }

    #[test]
    fn every_variant_round_trips() {
        let variants = [
            LicenseStatus::TrialNotStarted { trial_days: 14 },
            LicenseStatus::Trial {
                days_left: 3,
                trial_end_ms: 123,
            },
            LicenseStatus::ReadOnly,
            LicenseStatus::Licensed {
                update_through_ms: 456,
                in_window: true,
                email: "user@example.com".to_string(),
            },
        ];
        for status in variants {
            let json = serde_json::to_value(&status).expect("serialize");
            let back: LicenseStatus = serde_json::from_value(json).expect("deserialize");
            assert_eq!(back, status);
        }

        let result = ActivateLicenseResult {
            status: LicenseStatus::ReadOnly,
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["status"]["kind"], "readOnly");
        let back: ActivateLicenseResult = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, result);
    }

    #[test]
    fn capture_allowed_only_false_for_read_only() {
        assert!(LicenseStatus::TrialNotStarted { trial_days: 14 }.capture_allowed());
        assert!(LicenseStatus::Trial {
            days_left: 1,
            trial_end_ms: 0,
        }
        .capture_allowed());
        assert!(LicenseStatus::Licensed {
            update_through_ms: 0,
            in_window: false,
            email: String::new(),
        }
        .capture_allowed());
        assert!(!LicenseStatus::ReadOnly.capture_allowed());
    }
}
