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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrBudgetTelemetry {
    pub job_id: i64,
    pub frame_id: Option<i64>,
    pub provider: String,
    pub model_id: Option<String>,
    pub recognition_mode: Option<String>,
    pub status: String,
    pub run_duration_ms: i64,
    pub queue_wait_ms: Option<i64>,
    pub result_text_length: Option<i64>,
    pub observation_count: Option<i64>,
    pub created_at: Option<String>,
}
