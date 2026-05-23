use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DETECTOR_VERSION: &str = "secret-redaction-v2";
const DEFAULT_CANDIDATE_WINDOW_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SecretCategory {
    ApiKey,
    AccessToken,
    PrivateKey,
    Password,
    AuthCode,
    ConnectionString,
    SeedLikeSecret,
}

impl SecretCategory {
    pub fn marker(self) -> &'static str {
        match self {
            Self::ApiKey => "[REDACTED_SECRET: API_KEY]",
            Self::AccessToken => "[REDACTED_SECRET: ACCESS_TOKEN]",
            Self::PrivateKey => "[REDACTED_SECRET: PRIVATE_KEY]",
            Self::Password => "[REDACTED_SECRET: PASSWORD]",
            Self::AuthCode => "[REDACTED_SECRET: AUTH_CODE]",
            Self::ConnectionString => "[REDACTED_SECRET: CONNECTION_STRING]",
            Self::SeedLikeSecret => "[REDACTED_SECRET: SEED_SECRET]",
        }
    }

    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AccessToken => "access_token",
            Self::PrivateKey => "private_key",
            Self::Password => "password",
            Self::AuthCode => "auth_code",
            Self::ConnectionString => "connection_string",
            Self::SeedLikeSecret => "seed_like_secret",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionContext {
    Ocr,
    MicrophoneTranscript,
    SystemAudioTranscript,
    SearchableText,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSpan {
    pub start: usize,
    pub end: usize,
    pub category: SecretCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionResult {
    pub redacted_text: String,
    pub spans: Vec<RedactionSpan>,
    pub detector_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionRequest {
    pub context: RedactionContext,
    pub result_text: Option<String>,
    pub ocr: Option<OcrRedactionInput>,
    pub transcript: Option<TranscriptRedactionInput>,
    #[serde(default)]
    pub additional_surfaces: Vec<DerivedTextSurface>,
    pub budget: RedactionBudget,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrRedactionInput {
    #[serde(default)]
    pub observations: Vec<OcrRedactionObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrRedactionObservation {
    pub text: String,
    pub confidence: f32,
    pub bounding_box: RedactionBoundingBox,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionBoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptRedactionInput {
    #[serde(default)]
    pub segments: Vec<TranscriptRedactionSegment>,
    #[serde(default)]
    pub words: Vec<TranscriptRedactionWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptRedactionSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptRedactionWord {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DerivedTextSurface {
    pub kind: RedactionSurfaceKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionSurfaceKind {
    ResultText,
    OcrVisualLine,
    OcrObservation,
    TranscriptSegment,
    TranscriptWord,
    ContextText,
}

impl RedactionSurfaceKind {
    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::ResultText => "result_text",
            Self::OcrVisualLine => "ocr_visual_line",
            Self::OcrObservation => "ocr_observation",
            Self::TranscriptSegment => "transcript_segment",
            Self::TranscriptWord => "transcript_word",
            Self::ContextText => "context_text",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionScope {
    ExactSpan,
    RedactionUnit,
}

impl RedactionScope {
    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::ExactSpan => "exact_span",
            Self::RedactionUnit => "redaction_unit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedRedaction {
    pub category: SecretCategory,
    pub surface_kind: RedactionSurfaceKind,
    pub redaction_scope: RedactionScope,
    pub redacted_start: usize,
    pub redacted_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedRedactionPlan {
    pub result_text: Option<String>,
    #[serde(default)]
    pub ocr_observation_text: HashMap<usize, String>,
    #[serde(default)]
    pub transcript_segment_text: HashMap<usize, String>,
    #[serde(default)]
    pub transcript_word_text: HashMap<usize, String>,
    #[serde(default)]
    pub redactions: Vec<PlannedRedaction>,
    pub detector_version: String,
    pub telemetry: RedactionTelemetry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionTelemetry {
    pub scanned_surfaces: usize,
    pub redaction_count: usize,
    pub truncated_by_budget: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionBudget {
    pub max_surfaces: usize,
    pub max_surface_chars: usize,
}

impl Default for RedactionBudget {
    fn default() -> Self {
        Self {
            max_surfaces: 512,
            max_surface_chars: DEFAULT_CANDIDATE_WINDOW_CHARS * 512,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedactionError {
    SafetyFailure,
}

impl std::fmt::Display for RedactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SafetyFailure => f.write_str("secret redaction safety failure"),
        }
    }
}

impl std::error::Error for RedactionError {}

#[derive(Debug, Clone)]
struct Match {
    start: usize,
    end: usize,
    category: SecretCategory,
}

struct Detector {
    regex: Regex,
    category: SecretCategory,
    requires_evidence: bool,
}

static DETECTORS: Lazy<Vec<Detector>> = Lazy::new(|| {
    vec![
        Detector {
            regex: Regex::new(r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----").unwrap(),
            category: SecretCategory::PrivateKey,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|redis)://[A-Za-z0-9._%+\-~]+:[^@\s/]+@[^\s]+").unwrap(),
            category: SecretCategory::ConnectionString,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{24,}\b").unwrap(),
            category: SecretCategory::ApiKey,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\bgh[pousr]_[A-Za-z0-9_]{30,}\b").unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b").unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\bxox[baprs]-[A-Za-z0-9\-]{20,}\b").unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9_\-./+=]{20,}\b").unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\b(?:AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|SG\.[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}|rk_live_[A-Za-z0-9]{24,})\b").unwrap(),
            category: SecretCategory::ApiKey,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"\b(?:AIza[0-9A-Za-z_\-]{35}|ya29\.[0-9A-Za-z_\-]{40,}|glpat-[0-9A-Za-z_\-]{20,}|whsec_[0-9A-Za-z]{24,})\b").unwrap(),
            category: SecretCategory::ApiKey,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r"https://hooks\.slack\.com/services/[A-Za-z0-9_/+-]{24,}").unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: false,
        },
        Detector {
            regex: Regex::new(r#"(?i)\b(?:api\s*[_-]?\s*key|secret\s*[_-]?\s*key)\b\s*[:=\-]?\s*['"]?[A-Za-z0-9_\-./+=]{20,}['"]?"#).unwrap(),
            category: SecretCategory::ApiKey,
            requires_evidence: true,
        },
        Detector {
            regex: Regex::new(r#"(?i)\b(?:access\s*[_-]?\s*token|auth\s*[_-]?\s*token|bearer|webhook\s*[_-]?\s*secret)\b\s*[:=\-]?\s*['"]?[A-Za-z0-9_\-./+=]{20,}['"]?"#).unwrap(),
            category: SecretCategory::AccessToken,
            requires_evidence: true,
        },
        Detector {
            regex: Regex::new(r#"(?i)\b(?:password|passwd|pwd|client\s*[_-]?\s*secret|db\s*[_-]?\s*password)\b\s*[:=]?\s*['"]?[^'"\s]{8,}['"]?"#).unwrap(),
            category: SecretCategory::Password,
            requires_evidence: true,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:otp|mfa(?: code)?|2fa(?: code)?|auth(?:entication)? code|verification code)\b\s*[:=]?\s*\d{6,8}\b").unwrap(),
            category: SecretCategory::AuthCode,
            requires_evidence: true,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:seed phrase|mnemonic|recovery phrase)\b\s*[:=]\s*(?:[a-z]{3,10}\s+){11,23}[a-z]{3,10}\b").unwrap(),
            category: SecretCategory::SeedLikeSecret,
            requires_evidence: true,
        },
    ]
});

static EVIDENCE_PREFILTER: Lazy<AhoCorasick> = Lazy::new(|| {
    AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build([
            "api",
            "key",
            "token",
            "bearer",
            "secret",
            "password",
            "passwd",
            "pwd",
            "client",
            "webhook",
            "otp",
            "mfa",
            "2fa",
            "verification",
            "auth",
            "seed",
            "mnemonic",
            "recovery",
        ])
        .expect("redaction evidence prefilter should compile")
});

static NON_SECRET_DIAGNOSTIC_TEXT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)secret-redaction-v\d+\b").unwrap());

static PLACEHOLDER_SECRET_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(?:<[^>]*(?:key|token|secret|password)[^>]*>|your[-_ ]?(?:api[-_ ]?)?(?:key|token|secret|password)|changeme|change[-_ ]?me|example)(?:[/?#&@\s'"]|$)"#,
    )
    .unwrap()
});

pub fn redact_searchable_text(input: &str, _context: RedactionContext) -> RedactionResult {
    redact_text(input)
}

pub fn plan_redactions(request: RedactionRequest) -> Result<UnifiedRedactionPlan, RedactionError> {
    let mut plan = UnifiedRedactionPlan {
        result_text: request.result_text.clone(),
        ocr_observation_text: HashMap::new(),
        transcript_segment_text: HashMap::new(),
        transcript_word_text: HashMap::new(),
        redactions: Vec::new(),
        detector_version: DETECTOR_VERSION.to_string(),
        telemetry: RedactionTelemetry::default(),
    };

    if let Some(text) = request.result_text.as_deref() {
        apply_surface_redaction(
            &mut plan,
            RedactionSurfaceKind::ResultText,
            text,
            |plan, text| {
                plan.result_text = Some(text.to_string());
            },
        );
    }

    if let Some(ocr) = request.ocr {
        for line in build_ocr_visual_lines(&ocr.observations) {
            let result = redact_text(&line.text);
            plan.telemetry.scanned_surfaces += 1;
            if !result.spans.is_empty() {
                for span in &result.spans {
                    plan.redactions.push(PlannedRedaction {
                        category: span.category,
                        surface_kind: RedactionSurfaceKind::OcrVisualLine,
                        redaction_scope: RedactionScope::RedactionUnit,
                        redacted_start: span.start,
                        redacted_end: span.end,
                    });
                }
                for index in line.observation_indices {
                    let marker = dominant_marker(&result);
                    plan.ocr_observation_text.insert(index, marker.to_string());
                    plan.redactions.push(PlannedRedaction {
                        category: result.spans[0].category,
                        surface_kind: RedactionSurfaceKind::OcrObservation,
                        redaction_scope: RedactionScope::RedactionUnit,
                        redacted_start: 0,
                        redacted_end: marker.len(),
                    });
                }
            }
        }

        for (index, observation) in ocr.observations.iter().enumerate() {
            if plan.ocr_observation_text.contains_key(&index) {
                continue;
            }
            let result = redact_text(&observation.text);
            plan.telemetry.scanned_surfaces += 1;
            if !result.spans.is_empty() {
                plan.ocr_observation_text
                    .insert(index, result.redacted_text.clone());
                for span in result.spans {
                    plan.redactions.push(PlannedRedaction {
                        category: span.category,
                        surface_kind: RedactionSurfaceKind::OcrObservation,
                        redaction_scope: RedactionScope::ExactSpan,
                        redacted_start: span.start,
                        redacted_end: span.end,
                    });
                }
            }
        }
    }

    if let Some(transcript) = request.transcript {
        for (index, segment) in transcript.segments.iter().enumerate() {
            let result = redact_text(&segment.text);
            plan.telemetry.scanned_surfaces += 1;
            if !result.spans.is_empty() {
                plan.transcript_segment_text
                    .insert(index, result.redacted_text.clone());
                for span in result.spans {
                    plan.redactions.push(PlannedRedaction {
                        category: span.category,
                        surface_kind: RedactionSurfaceKind::TranscriptSegment,
                        redaction_scope: RedactionScope::ExactSpan,
                        redacted_start: span.start,
                        redacted_end: span.end,
                    });
                }
            }
        }
        for (index, word) in transcript.words.iter().enumerate() {
            let result = redact_text(&word.text);
            plan.telemetry.scanned_surfaces += 1;
            if !result.spans.is_empty() {
                plan.transcript_word_text
                    .insert(index, result.redacted_text.clone());
                for span in result.spans {
                    plan.redactions.push(PlannedRedaction {
                        category: span.category,
                        surface_kind: RedactionSurfaceKind::TranscriptWord,
                        redaction_scope: RedactionScope::ExactSpan,
                        redacted_start: span.start,
                        redacted_end: span.end,
                    });
                }
            }
        }
    }

    for surface in request.additional_surfaces {
        apply_surface_redaction(
            &mut plan,
            surface.kind,
            &bounded_surface_text(&surface.text, request.budget.max_surface_chars),
            |_plan, _text| {},
        );
    }

    plan.telemetry.redaction_count = plan.redactions.len();
    if plan.telemetry.scanned_surfaces > request.budget.max_surfaces {
        plan.telemetry.truncated_by_budget = true;
        return Err(RedactionError::SafetyFailure);
    }

    Ok(plan)
}

fn redact_text(input: &str) -> RedactionResult {
    let evidence_windows = evidence_windows(input);
    let mut matches = Vec::new();
    for detector in DETECTORS.iter() {
        if detector.requires_evidence {
            for (window_start, window_end) in &evidence_windows {
                matches.extend(
                    detector
                        .regex
                        .find_iter(&input[*window_start..*window_end])
                        .map(|m| Match {
                            start: window_start + m.start(),
                            end: window_start + m.end(),
                            category: detector.category,
                        }),
                );
            }
        } else {
            matches.extend(detector.regex.find_iter(input).map(|m| Match {
                start: m.start(),
                end: m.end(),
                category: detector.category,
            }));
        }
    }
    matches
        .retain(|m| !is_non_secret_diagnostic_match(input, m) && !is_placeholder_match(input, m));
    matches.sort_by_key(|m| (m.start, usize::MAX - m.end));

    let mut selected: Vec<Match> = Vec::new();
    for candidate in matches {
        if candidate.start == candidate.end {
            continue;
        }
        if let Some(last) = selected.last_mut() {
            if candidate.start < last.end {
                let candidate_len = candidate.end - candidate.start;
                let last_len = last.end - last.start;
                if candidate.start >= last.start && candidate.end <= last.end {
                    continue;
                }
                if candidate_len > last_len {
                    *last = candidate;
                }
                continue;
            }
        }
        selected.push(candidate);
    }

    let mut redacted_text = String::with_capacity(input.len());
    let mut spans = Vec::new();
    let mut cursor = 0;
    for m in selected {
        redacted_text.push_str(&input[cursor..m.start]);
        let start = redacted_text.len();
        redacted_text.push_str(m.category.marker());
        let end = redacted_text.len();
        spans.push(RedactionSpan {
            start,
            end,
            category: m.category,
        });
        cursor = m.end;
    }
    redacted_text.push_str(&input[cursor..]);

    RedactionResult {
        redacted_text,
        spans,
        detector_version: DETECTOR_VERSION.to_string(),
    }
}

fn is_non_secret_diagnostic_match(input: &str, m: &Match) -> bool {
    NON_SECRET_DIAGNOSTIC_TEXT.is_match(&input[m.start..m.end])
}

fn is_placeholder_match(input: &str, m: &Match) -> bool {
    let matched = &input[m.start..m.end];
    PLACEHOLDER_SECRET_VALUE.is_match(matched)
        || matched.contains("://user:password@")
        || matched.contains("://default:password@")
        || matched.contains(":password@")
        || matched.contains("password@")
        || is_weak_bare_password_phrase(matched, m.category)
}

fn is_weak_bare_password_phrase(matched: &str, category: SecretCategory) -> bool {
    if category != SecretCategory::Password || matched.contains(':') || matched.contains('=') {
        return false;
    }
    !matched.chars().any(|c| c.is_ascii_digit())
}

fn evidence_windows(input: &str) -> Vec<(usize, usize)> {
    let mut windows = EVIDENCE_PREFILTER
        .find_iter(input)
        .map(|m| {
            let start = input[..m.start()]
                .char_indices()
                .rev()
                .nth(DEFAULT_CANDIDATE_WINDOW_CHARS / 2)
                .map(|(index, _)| index)
                .unwrap_or(0);
            let end = input[m.end()..]
                .char_indices()
                .nth(DEFAULT_CANDIDATE_WINDOW_CHARS / 2)
                .map(|(index, _)| m.end() + index)
                .unwrap_or(input.len());
            (start, end)
        })
        .collect::<Vec<_>>();
    windows.sort_unstable();

    let mut merged = Vec::<(usize, usize)>::new();
    for (start, end) in windows {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn apply_surface_redaction(
    plan: &mut UnifiedRedactionPlan,
    kind: RedactionSurfaceKind,
    text: &str,
    apply_text: impl FnOnce(&mut UnifiedRedactionPlan, &str),
) {
    let result = redact_text(text);
    plan.telemetry.scanned_surfaces += 1;
    apply_text(plan, &result.redacted_text);
    for span in result.spans {
        plan.redactions.push(PlannedRedaction {
            category: span.category,
            surface_kind: kind,
            redaction_scope: RedactionScope::ExactSpan,
            redacted_start: span.start,
            redacted_end: span.end,
        });
    }
}

fn bounded_surface_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

#[derive(Debug)]
struct OcrVisualLine {
    text: String,
    observation_indices: Vec<usize>,
}

fn build_ocr_visual_lines(observations: &[OcrRedactionObservation]) -> Vec<OcrVisualLine> {
    let mut indexed = observations.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by(|(_, a), (_, b)| {
        a.bounding_box
            .y
            .total_cmp(&b.bounding_box.y)
            .then_with(|| a.bounding_box.x.total_cmp(&b.bounding_box.x))
    });

    let mut lines: Vec<Vec<(usize, &OcrRedactionObservation)>> = Vec::new();
    for (index, observation) in indexed {
        let center_y = observation.bounding_box.y + observation.bounding_box.height / 2.0;
        if let Some(line) = lines.last_mut() {
            let (_, first) = line[0];
            let first_center = first.bounding_box.y + first.bounding_box.height / 2.0;
            let tolerance = first
                .bounding_box
                .height
                .max(observation.bounding_box.height)
                .max(0.02);
            if (center_y - first_center).abs() <= tolerance {
                line.push((index, observation));
                continue;
            }
        }
        lines.push(vec![(index, observation)]);
    }

    lines
        .into_iter()
        .map(|mut line| {
            line.sort_by(|(_, a), (_, b)| a.bounding_box.x.total_cmp(&b.bounding_box.x));
            OcrVisualLine {
                text: line
                    .iter()
                    .map(|(_, observation)| observation.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                observation_indices: line.into_iter().map(|(index, _)| index).collect(),
            }
        })
        .collect()
}

fn dominant_marker(result: &RedactionResult) -> &'static str {
    result
        .spans
        .first()
        .map(|span| span.category.marker())
        .unwrap_or(SecretCategory::AccessToken.marker())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact(input: &str) -> RedactionResult {
        redact_searchable_text(input, RedactionContext::SearchableText)
    }

    #[test]
    fn redacts_common_api_tokens() {
        let input = "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz123456 and ghp_abcdefghijklmnopqrstuvwxyz1234567890";
        let result = redact(input);
        assert!(result
            .redacted_text
            .contains("[REDACTED_SECRET: ACCESS_TOKEN]"));
        assert!(!result
            .redacted_text
            .contains("sk-proj-abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!result
            .redacted_text
            .contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert_eq!(result.detector_version, DETECTOR_VERSION);
    }

    #[test]
    fn redacts_private_key_blocks() {
        let input = "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----";
        let result = redact(input);
        assert_eq!(result.redacted_text, "[REDACTED_SECRET: PRIVATE_KEY]");
        assert_eq!(result.spans[0].category, SecretCategory::PrivateKey);
    }

    #[test]
    fn redacts_env_passwords_db_urls_and_auth_codes() {
        let input =
            "DB_PASSWORD=hunter222 postgres://user:pass1234@localhost/app verification code 123456";
        let result = redact(input);
        assert!(result.redacted_text.contains("[REDACTED_SECRET: PASSWORD]"));
        assert!(result
            .redacted_text
            .contains("[REDACTED_SECRET: CONNECTION_STRING]"));
        assert!(result
            .redacted_text
            .contains("[REDACTED_SECRET: AUTH_CODE]"));
        assert!(!result.redacted_text.contains("hunter222"));
        assert!(!result.redacted_text.contains("pass1234"));
    }

    #[test]
    fn avoids_broad_false_positives() {
        let input = "uuid 550e8400-e29b-41d4-a716-446655440000 color #ff00aa stack at main.rs:123";
        let result = redact(input);
        assert_eq!(result.redacted_text, input);
        assert!(result.spans.is_empty());
    }

    #[test]
    fn ignores_diagnostic_redaction_version_with_numeric_prefix() {
        let plan = plan_redactions(RedactionRequest {
            context: RedactionContext::Ocr,
            result_text: None,
            ocr: Some(OcrRedactionInput {
                observations: vec![
                    OcrRedactionObservation {
                        text: "access token".to_string(),
                        confidence: 0.9,
                        bounding_box: RedactionBoundingBox {
                            x: 0.0,
                            y: 0.0,
                            width: 0.2,
                            height: 0.1,
                        },
                    },
                    OcrRedactionObservation {
                        text: "31secret-redaction-v2 2026-05-2315:42:30".to_string(),
                        confidence: 0.9,
                        bounding_box: RedactionBoundingBox {
                            x: 0.3,
                            y: 0.0,
                            width: 0.6,
                            height: 0.1,
                        },
                    },
                ],
            }),
            transcript: None,
            additional_surfaces: Vec::new(),
            budget: RedactionBudget::default(),
        })
        .expect("diagnostic text should produce a safe redaction plan");

        assert!(plan.redactions.is_empty());
        assert!(plan.ocr_observation_text.is_empty());
    }

    #[test]
    fn reports_spans_in_redacted_text_positions() {
        let input = "prefix OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz123456 suffix";
        let result = redact(input);
        let span = &result.spans[0];
        assert_eq!(
            &result.redacted_text[span.start..span.end],
            SecretCategory::ApiKey.marker()
        );
    }

    #[test]
    fn planner_redacts_split_ocr_visual_line_as_units() {
        let secret = "sk-abcdefghijklmnopqrstuvwxyz123456";
        let plan = plan_redactions(RedactionRequest {
            context: RedactionContext::Ocr,
            result_text: None,
            ocr: Some(OcrRedactionInput {
                observations: vec![
                    OcrRedactionObservation {
                        text: "OPENAI_API_KEY".to_string(),
                        confidence: 0.9,
                        bounding_box: RedactionBoundingBox {
                            x: 0.0,
                            y: 0.0,
                            width: 0.3,
                            height: 0.1,
                        },
                    },
                    OcrRedactionObservation {
                        text: secret.to_string(),
                        confidence: 0.9,
                        bounding_box: RedactionBoundingBox {
                            x: 0.4,
                            y: 0.0,
                            width: 0.5,
                            height: 0.1,
                        },
                    },
                ],
            }),
            transcript: None,
            additional_surfaces: Vec::new(),
            budget: RedactionBudget::default(),
        })
        .expect("planner should redact split OCR line");

        assert_eq!(
            plan.ocr_observation_text.get(&0).map(String::as_str),
            Some("[REDACTED_SECRET: API_KEY]")
        );
        assert_eq!(
            plan.ocr_observation_text.get(&1).map(String::as_str),
            Some("[REDACTED_SECRET: API_KEY]")
        );
        assert!(plan.redactions.iter().any(|redaction| {
            redaction.surface_kind == RedactionSurfaceKind::OcrVisualLine
                && redaction.redaction_scope == RedactionScope::RedactionUnit
        }));
    }

    #[test]
    fn planner_fails_when_surface_budget_is_exceeded() {
        let result = plan_redactions(RedactionRequest {
            context: RedactionContext::SearchableText,
            result_text: Some("safe".to_string()),
            ocr: None,
            transcript: None,
            additional_surfaces: Vec::new(),
            budget: RedactionBudget {
                max_surfaces: 0,
                max_surface_chars: 64,
            },
        });
        assert_eq!(result, Err(RedactionError::SafetyFailure));
    }

    #[test]
    fn planner_counts_non_matching_ocr_surfaces_against_budget() {
        let result = plan_redactions(RedactionRequest {
            context: RedactionContext::Ocr,
            result_text: None,
            ocr: Some(OcrRedactionInput {
                observations: vec![OcrRedactionObservation {
                    text: "ordinary visible text".to_string(),
                    confidence: 0.9,
                    bounding_box: RedactionBoundingBox {
                        x: 0.0,
                        y: 0.0,
                        width: 0.5,
                        height: 0.1,
                    },
                }],
            }),
            transcript: None,
            additional_surfaces: Vec::new(),
            budget: RedactionBudget {
                max_surfaces: 0,
                max_surface_chars: 64,
            },
        });

        assert_eq!(result, Err(RedactionError::SafetyFailure));
    }
}
