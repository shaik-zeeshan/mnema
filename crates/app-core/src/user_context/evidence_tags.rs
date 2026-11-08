//! Deterministic scrub of `f<id>`/`a<id>` evidence tags the engine leaks into
//! an Activity title or summary despite the preamble telling it not to.

use std::collections::HashMap;

/// Tidy-ups applied (to a fixed point) only after a tag was removed, so text the
/// engine wrote cleanly is returned byte-identical. Every rule shortens the
/// string, so the loop terminates.
const TIDY: [(&str, &str); 14] = [
    ("  ", " "),
    (" ,", ","),
    (" .", "."),
    (" ;", ";"),
    (" )", ")"),
    ("( ", "("),
    (" ]", "]"),
    ("[ ", "["),
    (",)", ")"),
    (",]", "]"),
    (";)", ")"),
    (",.", "."),
    ("()", ""),
    ("[]", ""),
];

/// Remove every whole word that is one of THIS window's evidence tags
/// (case-sensitive, so "F1" or "a4 paper" survive unless `a4` really is a tag
/// here), then drop the brackets and separators it leaves behind.
pub(crate) fn strip_evidence_tags(text: &str, known_tags: &HashMap<String, i64>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut removed = false;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c.is_alphanumeric() || c == '_' {
            let end = rest
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            let (word, tail) = rest.split_at(end);
            rest = tail;
            if known_tags.contains_key(word) {
                removed = true;
                // The list separator after a tag goes with it ("f12, a3").
                rest = rest.strip_prefix([',', ';']).unwrap_or(rest);
            } else {
                out.push_str(word);
            }
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !removed {
        return text.to_string();
    }
    loop {
        let before = out.len();
        for (from, to) in TIDY {
            out = out.replace(from, to);
        }
        if out.len() == before {
            break;
        }
    }
    out.trim()
        .trim_end_matches([',', ';'])
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(text: &str) -> String {
        let tags = HashMap::from([
            ("f12".to_string(), 0),
            ("a3".to_string(), 0),
            ("f1".to_string(), 0),
        ]);
        strip_evidence_tags(text, &tags)
    }

    #[test]
    fn strips_only_this_windows_tags_and_their_brackets() {
        assert_eq!(
            strip("Reviewed the config (f12, a3)."),
            "Reviewed the config."
        );
        assert_eq!(
            strip("Reviewed the config [f12] in Zed"),
            "Reviewed the config in Zed"
        );
        assert_eq!(strip("Reviewed the config f12."), "Reviewed the config.");
        assert_eq!(strip("Reviewed the config, f12"), "Reviewed the config");
        assert_eq!(strip("Fixed f12, a3 and tests"), "Fixed and tests");
        assert_eq!(strip("Watched F1 qualifying (f1)"), "Watched F1 qualifying");
        assert_eq!(strip("Printed on a4 paper"), "Printed on a4 paper");
        assert_eq!(strip("f12"), "");
    }

    #[test]
    fn untouched_text_is_returned_verbatim() {
        let text = "Odd  spacing , kept ()";
        assert_eq!(strip(text), text);
    }
}
