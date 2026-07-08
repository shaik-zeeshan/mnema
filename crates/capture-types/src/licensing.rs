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
    /// Authentic key that appears on the signed revocation list (refund/leak).
    /// Capture disabled; recorded history stays readable. Distinct from
    /// `ReadOnly` so the UI can say "revoked" honestly (never "refunded").
    Revoked,
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
        !matches!(self, LicenseStatus::ReadOnly | LicenseStatus::Revoked)
    }

    /// Like [`Self::capture_allowed`], but a `Trial` whose window has lapsed
    /// also blocks. The cached status only recomputes on gate events (launch,
    /// capture start), so a trial can expire while the cache still says
    /// `Trial` — without this, the first start after expiry slips through.
    pub fn capture_allowed_at(&self, now_ms: i64) -> bool {
        match self {
            LicenseStatus::ReadOnly | LicenseStatus::Revoked => false,
            LicenseStatus::Trial { trial_end_ms, .. } => now_ms < *trial_end_ms,
            _ => true,
        }
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
            (LicenseStatus::Revoked, "revoked"),
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
            LicenseStatus::Revoked,
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
        assert!(!LicenseStatus::Revoked.capture_allowed());
    }

    #[test]
    fn capture_allowed_at_blocks_lapsed_trial() {
        let trial = LicenseStatus::Trial {
            days_left: 1,
            trial_end_ms: 1_000,
        };
        assert!(trial.capture_allowed_at(999));
        assert!(!trial.capture_allowed_at(1_000));
        assert!(!trial.capture_allowed_at(2_000));
        // Non-trial states are unaffected by the clock.
        assert!(LicenseStatus::TrialNotStarted { trial_days: 30 }.capture_allowed_at(i64::MAX));
        assert!(!LicenseStatus::ReadOnly.capture_allowed_at(0));
        assert!(!LicenseStatus::Revoked.capture_allowed_at(i64::MAX));
        assert!(LicenseStatus::Licensed {
            update_through_ms: 0,
            in_window: false,
            email: String::new(),
        }
        .capture_allowed_at(i64::MAX));
    }
}
