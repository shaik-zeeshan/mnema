use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const DETECTOR_VERSION: &str = "secret-redaction-v1";

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

#[derive(Debug, Clone)]
struct Match {
    start: usize,
    end: usize,
    category: SecretCategory,
}

struct Detector {
    regex: Regex,
    category: SecretCategory,
}

static DETECTORS: Lazy<Vec<Detector>> = Lazy::new(|| {
    vec![
        Detector {
            regex: Regex::new(r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----").unwrap(),
            category: SecretCategory::PrivateKey,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|redis)://[A-Za-z0-9._%+\-~]+:[^@\s/]+@[^\s]+").unwrap(),
            category: SecretCategory::ConnectionString,
        },
        Detector {
            regex: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{24,}\b").unwrap(),
            category: SecretCategory::ApiKey,
        },
        Detector {
            regex: Regex::new(r"\bgh[pousr]_[A-Za-z0-9_]{30,}\b").unwrap(),
            category: SecretCategory::AccessToken,
        },
        Detector {
            regex: Regex::new(r"\b(?:xox[baprs]-[A-Za-z0-9\-]{20,}|AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|SG\.[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}|rk_live_[A-Za-z0-9]{24,})\b").unwrap(),
            category: SecretCategory::ApiKey,
        },
        Detector {
            regex: Regex::new(r#"(?i)\b(?:api[_-]?key|access[_-]?token|auth[_-]?token|bearer|secret[_-]?key)\b\s*[:=]\s*['"]?[A-Za-z0-9_\-./+=]{20,}['"]?"#).unwrap(),
            category: SecretCategory::AccessToken,
        },
        Detector {
            regex: Regex::new(r#"(?i)\b(?:password|passwd|pwd|client_secret|db_password)\b\s*[:=]\s*['"]?[^'"\s]{8,}['"]?"#).unwrap(),
            category: SecretCategory::Password,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:otp|mfa|2fa|auth(?:entication)? code|verification code)\b\s*[:=]?\s*\d{6,8}\b").unwrap(),
            category: SecretCategory::AuthCode,
        },
        Detector {
            regex: Regex::new(r"(?i)\b(?:seed phrase|mnemonic|recovery phrase)\b\s*[:=]\s*(?:[a-z]{3,10}\s+){11,23}[a-z]{3,10}\b").unwrap(),
            category: SecretCategory::SeedLikeSecret,
        },
    ]
});

pub fn redact_searchable_text(input: &str, _context: RedactionContext) -> RedactionResult {
    let mut matches = DETECTORS
        .iter()
        .flat_map(|detector| {
            detector.regex.find_iter(input).map(|m| Match {
                start: m.start(),
                end: m.end(),
                category: detector.category,
            })
        })
        .collect::<Vec<_>>();
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
    fn reports_spans_in_redacted_text_positions() {
        let input = "prefix OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz123456 suffix";
        let result = redact(input);
        let span = &result.spans[0];
        assert_eq!(
            &result.redacted_text[span.start..span.end],
            SecretCategory::ApiKey.marker()
        );
    }
}
