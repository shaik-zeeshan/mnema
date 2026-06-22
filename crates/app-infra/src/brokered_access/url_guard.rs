//! Read-time URL guard for brokered access.
//!
//! Turns a raw captured browser URL into a sanitized, secret-redacted
//! `host[:port]/path` string that is safe to expose to a cloud AI model.
//!
//! The guard is applied at the broker boundary REGARDLESS of the capture mode
//! the URL was recorded under: query and fragment are always stripped, known
//! secret shapes anywhere in the path are redacted, and generic opaque tokens
//! are redacted only when "armed" by a credential-bearing predecessor segment.
//!
//! Accepted residual: a bare, context-free high-entropy path token with no
//! credential-bearing predecessor (e.g. `app.com/AbC9x…`) is allowed to pass.
//! Resource ids (Google Doc ids after `/d/`, commit SHAs after `/commit/`,
//! UUIDs after `/users/`) are preserved precisely because their predecessors
//! are not in the armed list. There is no blunt entropy backstop.

use secret_redaction::{redact_searchable_text, RedactionContext};
use url::Url;

/// Placeholder emitted for an armed-but-not-known-shape opaque token.
///
/// We reuse `secret-redaction`'s `ACCESS_TOKEN` marker so the positional-arming
/// pass produces text indistinguishable from the known-shape redaction pass
/// (both emit `[REDACTED_SECRET: ...]` markers from the same vocabulary).
const ARMED_TOKEN_PLACEHOLDER: &str = "[REDACTED_SECRET: ACCESS_TOKEN]";

/// Path segments that, when they immediately precede a token-shaped segment,
/// "arm" that following segment for redaction (the credential-bearing tail of a
/// reset / verify / magic-link / OTP / auth flow). The keyword itself stays
/// visible — only the armed token is redacted.
const ARMED_PREDECESSORS: &[&str] = &[
    "reset",
    "resetpassword",
    "verify",
    "confirm",
    "activate",
    "invite",
    "magic",
    "magiclink",
    "otp",
    "token",
    "auth",
];

/// Read-time guard: raw captured URL -> `Option<guarded host+path>`.
///
/// Returns `None` when there is no broker-safe text to emit (non-`http(s)`,
/// unparseable).
pub fn guard_url(raw_url: &str) -> Option<String> {
    // 1. Parse + scheme gate.
    let parsed = Url::parse(raw_url).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }

    // 2. Strip query + fragment unconditionally (same semantics as
    //    capture-metadata's `sanitize_url`). The result is `host[:port]/path`.
    let mut sanitized = parsed;
    sanitized.set_query(None);
    sanitized.set_fragment(None);

    let host = sanitized.host_str().unwrap_or_default();
    let authority = match sanitized.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let path = sanitized.path();

    // 3 + 4. Both redaction passes run PER PATH SEGMENT (and the authority is
    //         redacted in isolation). Segmenting first matters: the known-shape
    //         Password detector matches `password` only when an 8+ char value
    //         follows it, so over the joined `reset-password/AbC9x…` string it
    //         would eat the keyword *and* the token together. Per-segment, the
    //         standalone `reset-password` keyword survives (no value attached)
    //         while secrets that legitimately live inside one segment are still
    //         caught — secrets never span a `/`.
    let guarded_authority = redact_known_shape(&authority);

    let mut redacted_count = 0usize;
    let mut passed_count = 0usize;

    // `path` always begins with '/' for http(s) URLs. Splitting on '/' keeps
    // the empty leading/trailing segments so we can rejoin faithfully.
    let segments: Vec<&str> = path.split('/').collect();
    let mut out: Vec<String> = Vec::with_capacity(segments.len());
    // Track the previous NON-EMPTY raw segment as the arming predecessor; arming
    // is decided from the ORIGINAL segment text, not the redacted form.
    let mut prev_keyword: Option<String> = None;
    for segment in &segments {
        if segment.is_empty() {
            out.push(String::new());
            continue;
        }
        let armed = prev_keyword
            .as_deref()
            .map(is_armed_predecessor)
            .unwrap_or(false);
        if armed && is_token_shaped(segment) {
            // 4. Positional-arming pass: an opaque token after a credential
            //    keyword. The keyword segment itself was already emitted intact.
            out.push(ARMED_TOKEN_PLACEHOLDER.to_string());
            redacted_count += 1;
        } else {
            // 3. Known-shape redaction within this single segment.
            out.push(redact_known_shape(segment));
            passed_count += 1;
        }
        // The current (raw) segment becomes the predecessor for the next one.
        prev_keyword = Some(normalize_keyword(segment));
    }
    let guarded_path = out.join("/");

    // 5. Observability: one debug line per call. Never logs URL contents at
    //    info level; the counts keep the "we accepted the bare-token tail"
    //    residual observable.
    log::debug!(
        "brokered url_guard positional-arming: redacted={redacted_count} passed={passed_count}"
    );

    Some(format!("{guarded_authority}{guarded_path}"))
}

/// Run the shared known-shape secret redactor over a single string and return
/// its redacted form. The default `RedactionBudget` (max_surfaces=512) is fine
/// for one short URL segment. This catches `gh_…`, `sk-…`, JWTs, etc.
fn redact_known_shape(input: &str) -> String {
    redact_searchable_text(input, RedactionContext::SearchableText).redacted_text
}

/// Case-insensitive, non-alphanumeric-ignoring membership test against the
/// armed predecessor list.
fn is_armed_predecessor(segment: &str) -> bool {
    ARMED_PREDECESSORS.contains(&normalize_keyword(segment).as_str())
}

/// Lowercase a segment and drop every non-alphanumeric character, so that
/// `reset-password`, `Reset_Password`, and `resetpassword` all compare equal.
fn normalize_keyword(segment: &str) -> String {
    segment
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Conservative "token-shaped" test — favors PRESERVING resource ids.
///
/// A segment qualifies only if `len >= 12` AND
/// (`has_digit && has_ascii_alpha`) OR (`has_uppercase && has_lowercase`).
///
/// This makes dictionary words like `email` survive (so `/verify/email` is
/// preserved) while high-entropy tokens like `AbC9xK2mP4qR7s` or a long hex
/// `8f3a9c...` (len >= 12) get redacted.
fn is_token_shaped(segment: &str) -> bool {
    if segment.chars().count() < 12 {
        return false;
    }
    let mut has_digit = false;
    let mut has_ascii_alpha = false;
    let mut has_upper = false;
    let mut has_lower = false;
    for c in segment.chars() {
        if c.is_ascii_digit() {
            has_digit = true;
        }
        if c.is_ascii_alphabetic() {
            has_ascii_alpha = true;
        }
        if c.is_ascii_uppercase() {
            has_upper = true;
        }
        if c.is_ascii_lowercase() {
            has_lower = true;
        }
    }
    (has_digit && has_ascii_alpha) || (has_upper && has_lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(raw: &str) -> Option<String> {
        guard_url(raw)
    }

    // --- MUST be redacted ---

    #[test]
    fn reset_password_token_is_redacted_keyword_visible() {
        let out = guard("https://site.com/reset-password/AbC9xK2mP4qR7sT0").unwrap();
        assert!(
            out.contains("reset-password"),
            "predecessor keyword must stay visible: {out}"
        );
        assert!(
            !out.contains("AbC9xK2mP4qR7sT0"),
            "armed token must be redacted: {out}"
        );
        assert_eq!(out, format!("site.com/reset-password/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn invite_hex_token_is_redacted() {
        let out = guard("https://site.com/invite/8f3a9c2b7e1d4a6f").unwrap();
        assert!(out.contains("invite"), "{out}");
        assert!(!out.contains("8f3a9c2b7e1d4a6f"), "{out}");
        assert_eq!(out, format!("site.com/invite/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn token_after_verify_is_redacted() {
        let out = guard("https://app.com/auth/verify/Xy91Kd83Lm04Qp").unwrap();
        assert!(out.contains("auth"), "{out}");
        assert!(out.contains("verify"), "{out}");
        assert!(!out.contains("Xy91Kd83Lm04Qp"), "{out}");
        assert_eq!(out, format!("app.com/auth/verify/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn query_and_fragment_are_stripped_wholesale() {
        let out = guard("https://site.com/page?token=SECRET123#access_token=abc").unwrap();
        assert!(!out.contains("SECRET123"), "query must be stripped: {out}");
        assert!(!out.contains("access_token"), "fragment must be stripped: {out}");
        assert!(!out.contains("abc"), "fragment value must be stripped: {out}");
        assert_eq!(out, "site.com/page");
    }

    // --- MUST be preserved ---

    #[test]
    fn google_doc_id_survives() {
        let out =
            guard("https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz/edit").unwrap();
        assert!(
            out.contains("1AbCdEfGhIjKlMnOpQrStUvWxYz"),
            "doc id must survive (predecessor `d` not armed): {out}"
        );
        assert_eq!(
            out,
            "docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz/edit"
        );
    }

    #[test]
    fn commit_sha_survives() {
        let out = guard(
            "https://github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f",
        )
        .unwrap();
        assert!(
            out.contains("9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"),
            "SHA must survive (predecessor `commit` not armed): {out}"
        );
        assert_eq!(
            out,
            "github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"
        );
    }

    #[test]
    fn uuid_survives() {
        let out = guard("https://site.com/users/550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(
            out.contains("550e8400-e29b-41d4-a716-446655440000"),
            "UUID must survive (predecessor `users` not armed): {out}"
        );
        assert_eq!(
            out,
            "site.com/users/550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn verify_email_survives_not_token_shaped() {
        let out = guard("https://app.com/verify/email").unwrap();
        assert!(
            out.contains("email"),
            "dictionary word `email` is not token-shaped, must survive: {out}"
        );
        assert_eq!(out, "app.com/verify/email");
    }

    // --- Boundary ---

    #[test]
    fn file_scheme_returns_none() {
        assert_eq!(guard("file:///Users/me/secret.txt"), None);
    }

    #[test]
    fn unparseable_returns_none() {
        assert_eq!(guard("not a url"), None);
    }

    #[test]
    fn other_schemes_return_none() {
        assert_eq!(guard("ftp://example.com/file"), None);
        assert_eq!(guard("mailto:someone@example.com"), None);
    }

    #[test]
    fn full_mode_style_input_still_strips_query_and_fragment() {
        // A URL captured under a "Full" mode would carry its query+fragment; the
        // boundary guard strips them regardless of how it was recorded.
        let out =
            guard("https://example.com/dashboard?session=AbC9xK2mP4qR7sT0#tab=secrets").unwrap();
        assert!(!out.contains("AbC9xK2mP4qR7sT0"), "{out}");
        assert!(!out.contains("secrets"), "{out}");
        assert_eq!(out, "example.com/dashboard");
    }

    #[test]
    fn known_shape_secret_in_path_is_redacted_anywhere() {
        // A GitHub token in the path (no armed predecessor) is still caught by
        // the known-shape redaction pass.
        let out =
            guard("https://example.com/page/ghp_1234567890abcdefABCDEF1234567890abcd").unwrap();
        assert!(
            !out.contains("ghp_1234567890abcdefABCDEF1234567890abcd"),
            "known-shape GitHub token must be redacted: {out}"
        );
    }

    #[test]
    fn host_only_url_passes_through() {
        let out = guard("https://example.com").unwrap();
        assert_eq!(out, "example.com/");
    }

    #[test]
    fn port_is_preserved_in_authority() {
        let out = guard("https://example.com:8443/dashboard").unwrap();
        assert_eq!(out, "example.com:8443/dashboard");
    }
}
