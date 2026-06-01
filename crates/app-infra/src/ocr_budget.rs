use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrAdmissionOutcome {
    Admitted,
    Skipped,
}

impl OcrAdmissionOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_str(value: &str) -> crate::Result<Self> {
        match value {
            "admitted" => Ok(Self::Admitted),
            "skipped" => Ok(Self::Skipped),
            other => Err(crate::AppInfraError::OcrEngine(format!(
                "invalid OCR admission outcome '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrAdmissionReason {
    AdmittedInitial,
    AdmittedContextChange,
    AdmittedLowPressure,
    AdmittedRepresentative,
    AdmittedVisualNovelty,
    SkippedEquivalentFrame,
    SkippedOcrDisabled,
    SkippedProviderUnavailable,
    SkippedLowOcrValue,
}

impl OcrAdmissionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdmittedInitial => "admitted_initial",
            Self::AdmittedContextChange => "admitted_context_change",
            Self::AdmittedLowPressure => "admitted_low_pressure",
            Self::AdmittedRepresentative => "admitted_representative",
            Self::AdmittedVisualNovelty => "admitted_visual_novelty",
            Self::SkippedEquivalentFrame => "skipped_equivalent_frame",
            Self::SkippedOcrDisabled => "skipped_ocr_disabled",
            Self::SkippedProviderUnavailable => "skipped_provider_unavailable",
            Self::SkippedLowOcrValue => "skipped_low_ocr_value",
        }
    }

    pub fn from_str(value: &str) -> crate::Result<Self> {
        match value {
            "admitted_initial" => Ok(Self::AdmittedInitial),
            "admitted_context_change" => Ok(Self::AdmittedContextChange),
            "admitted_low_pressure" => Ok(Self::AdmittedLowPressure),
            "admitted_representative" => Ok(Self::AdmittedRepresentative),
            "admitted_visual_novelty" => Ok(Self::AdmittedVisualNovelty),
            "skipped_equivalent_frame" => Ok(Self::SkippedEquivalentFrame),
            "skipped_ocr_disabled" => Ok(Self::SkippedOcrDisabled),
            "skipped_provider_unavailable" => Ok(Self::SkippedProviderUnavailable),
            "skipped_low_ocr_value" => Ok(Self::SkippedLowOcrValue),
            other => Err(crate::AppInfraError::OcrEngine(format!(
                "invalid OCR admission reason '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrAdmissionSignals {
    pub first_candidate_in_scope: bool,
    pub context_changed: bool,
    pub low_queue_pressure: bool,
    pub representative_due: bool,
    pub high_queue_pressure: bool,
    /// True when this frame's visual fingerprint (`equivalence_hint`) has not
    /// been seen in the current admission scope this run. Treated as `false`
    /// when equivalence is not ready (e.g. quarantined).
    pub fingerprint_novel_in_scope: bool,
    /// True when a novelty admission is permitted right now: the per-scope rate
    /// floor has elapsed AND the scope is not in a continuous-novelty
    /// (video/animation) burst.
    pub novelty_admission_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrAdmissionDecision {
    pub outcome: OcrAdmissionOutcome,
    pub reason: OcrAdmissionReason,
    pub related_frame_id: Option<i64>,
    pub queue_pressure_count: i64,
    pub recording_active: bool,
    pub signals: OcrAdmissionSignals,
}

impl OcrAdmissionDecision {
    pub fn admit(
        reason: OcrAdmissionReason,
        queue_pressure_count: i64,
        recording_active: bool,
    ) -> Self {
        Self {
            outcome: OcrAdmissionOutcome::Admitted,
            reason,
            related_frame_id: None,
            queue_pressure_count,
            recording_active,
            signals: OcrAdmissionSignals::default(),
        }
    }

    pub fn skip(
        reason: OcrAdmissionReason,
        queue_pressure_count: i64,
        recording_active: bool,
    ) -> Self {
        Self {
            outcome: OcrAdmissionOutcome::Skipped,
            reason,
            related_frame_id: None,
            queue_pressure_count,
            recording_active,
            signals: OcrAdmissionSignals::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_reason_round_trips_through_str() {
        let reasons = [
            OcrAdmissionReason::AdmittedInitial,
            OcrAdmissionReason::AdmittedContextChange,
            OcrAdmissionReason::AdmittedLowPressure,
            OcrAdmissionReason::AdmittedRepresentative,
            OcrAdmissionReason::AdmittedVisualNovelty,
            OcrAdmissionReason::SkippedEquivalentFrame,
            OcrAdmissionReason::SkippedOcrDisabled,
            OcrAdmissionReason::SkippedProviderUnavailable,
            OcrAdmissionReason::SkippedLowOcrValue,
        ];
        for reason in reasons {
            assert_eq!(
                OcrAdmissionReason::from_str(reason.as_str()).unwrap(),
                reason
            );
        }
    }

    #[test]
    fn visual_novelty_reason_uses_snake_case_wire_value() {
        assert_eq!(
            OcrAdmissionReason::AdmittedVisualNovelty.as_str(),
            "admitted_visual_novelty"
        );
    }

    #[test]
    fn novelty_signals_serialize_as_camel_case() {
        let signals = OcrAdmissionSignals {
            fingerprint_novel_in_scope: true,
            novelty_admission_available: true,
            ..Default::default()
        };
        let json = serde_json::to_value(&signals).unwrap();
        assert_eq!(json["fingerprintNovelInScope"], serde_json::json!(true));
        assert_eq!(json["noveltyAdmissionAvailable"], serde_json::json!(true));
    }
}
