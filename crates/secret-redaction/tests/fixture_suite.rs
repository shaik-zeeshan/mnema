use secret_redaction::{
    plan_redactions, redact_searchable_text, OcrRedactionInput, OcrRedactionObservation,
    RedactionBoundingBox, RedactionBudget, RedactionContext, RedactionRequest, SecretCategory,
};

fn fixture_lines(name: &str) -> Vec<String> {
    let path = format!("tests/fixtures/{name}");
    std::fs::read_to_string(path)
        .expect("fixture should be readable")
        .lines()
        .map(|line| line.replace("\\n", "\n"))
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .collect()
}

#[test]
fn fixture_true_positives_are_redacted() {
    for line in fixture_lines("true_positives.txt") {
        let (category, input) = line
            .split_once('|')
            .expect("true-positive fixture rows use category|input");
        let result = redact_searchable_text(input, RedactionContext::SearchableText);
        let expected_category = parse_category(category);
        assert!(
            result
                .spans
                .iter()
                .any(|span| span.category == expected_category),
            "expected {category} fixture to redact: {input}"
        );
        assert_ne!(result.redacted_text, input);
    }
}

#[test]
fn fixture_false_positives_are_not_redacted() {
    for input in fixture_lines("false_positives.txt") {
        let result = redact_searchable_text(&input, RedactionContext::SearchableText);
        assert!(
            result.spans.is_empty(),
            "false-positive fixture should not redact: {input}"
        );
        assert_eq!(result.redacted_text, input);
    }
}

#[test]
fn fixture_ocr_visual_lines_are_redacted_as_units() {
    for line in fixture_lines("ocr_visual_lines.txt") {
        let (category, observations) = line
            .split_once('|')
            .expect("OCR fixture rows use category|observation||observation");
        let observations = observations
            .split("||")
            .enumerate()
            .map(|(index, text)| OcrRedactionObservation {
                text: text.to_string(),
                confidence: 0.82,
                bounding_box: RedactionBoundingBox {
                    x: index as f64 * 0.18,
                    y: 0.20,
                    width: 0.16,
                    height: 0.05,
                },
            })
            .collect();

        let plan = plan_redactions(RedactionRequest {
            context: RedactionContext::Ocr,
            result_text: None,
            ocr: Some(OcrRedactionInput { observations }),
            transcript: None,
            additional_surfaces: Vec::new(),
            budget: RedactionBudget::default(),
        })
        .expect("OCR fixture should produce a safe redaction plan");
        let expected_category = parse_category(category);

        assert!(
            plan.redactions
                .iter()
                .any(|redaction| redaction.category == expected_category),
            "expected OCR {category} fixture to redact: {line}"
        );
        assert!(
            !plan.ocr_observation_text.is_empty(),
            "expected OCR fixture to redact source observations: {line}"
        );
    }
}

fn parse_category(category: &str) -> SecretCategory {
    match category {
        "api_key" => SecretCategory::ApiKey,
        "access_token" => SecretCategory::AccessToken,
        "private_key" => SecretCategory::PrivateKey,
        "password" => SecretCategory::Password,
        "auth_code" => SecretCategory::AuthCode,
        "connection_string" => SecretCategory::ConnectionString,
        "seed_like_secret" => SecretCategory::SeedLikeSecret,
        other => panic!("unknown fixture category: {other}"),
    }
}
