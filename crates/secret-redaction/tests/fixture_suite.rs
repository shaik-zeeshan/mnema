use secret_redaction::{redact_searchable_text, RedactionContext};

fn fixture_lines(name: &str) -> Vec<String> {
    let path = format!("tests/fixtures/{name}");
    std::fs::read_to_string(path)
        .expect("fixture should be readable")
        .lines()
        .map(|line| line.replace("\\n", "\n"))
        .filter(|line| !line.trim().is_empty())
        .collect()
}

#[test]
fn fixture_true_positives_are_redacted() {
    for line in fixture_lines("true_positives.txt") {
        let (category, input) = line
            .split_once('|')
            .expect("true-positive fixture rows use category|input");
        let result = redact_searchable_text(input, RedactionContext::SearchableText);
        assert!(
            !result.spans.is_empty(),
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
