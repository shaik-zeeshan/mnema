//! Read-time URL guard for brokered access.
//!
//! Turns a raw captured browser URL into a sanitized, secret-redacted
//! `host[:port]/path` string that is safe to expose to a cloud AI model.
//!
//! The guard is applied at the broker boundary REGARDLESS of the capture mode
//! the URL was recorded under: query and fragment are always stripped. Within
//! the path, three passes redact secrets:
//!
//! 1. **Known-shape pass** — `secret-redaction` catches fixed-prefix tokens
//!    (`sk-`, `ghp_`, JWTs, `AKIA…`, …) anywhere in a segment.
//! 2. **Positional-arming pass** — a credential-bearing predecessor segment
//!    (`reset`, `verify`, `invite`, `login`, `click`, … see
//!    `ARMED_PREDECESSORS`) "arms" the segment that follows it. Because arming
//!    already establishes credential intent, an armed segment is redacted
//!    aggressively: ANY opaque token of `len >= 12` goes, regardless of
//!    character class (so all-lowercase / all-upper / all-digit reset tokens no
//!    longer slip through). `%2F`-encoded slashes inside one path segment are
//!    split into sub-parts so a keyword sub-part can arm the following sub-part
//!    (e.g. `verify%2F<token>`); the original `%2F`/`%2f` delimiter is preserved
//!    on rejoin so the keyword stays readable.
//! 3. **High-entropy backstop** — even with no armed predecessor, an obviously
//!    random, separator-light token (`len >= 12`, mixed character class, a
//!    single run with no hyphen-separated word structure) is redacted UNLESS it
//!    is a recognized resource id: a UUID, an all-hex string (commit SHAs), or
//!    a segment whose immediate predecessor is a resource-id carrier
//!    (`d`, `document`, `commit`, `blob`, `tree`, `users`, … see
//!    `RESOURCE_ID_PREDECESSORS`). This catches share/reset tokens that ride
//!    generic carriers like `/s/`, `/scl/fi/`, or `/ls/click/`.
//!
//! Accepted residual (what still passes):
//!   - A high-entropy token that is ALSO a recognized resource id — i.e. it
//!     sits directly after a resource-id carrier (`/d/<id>`), or it is itself a
//!     UUID or an all-hex string — is preserved by design, even if some such
//!     carriers occasionally front a private token.
//!   - A content slug that happens to look token-ish but is hyphen-separated
//!     into word-like parts (e.g. `getting-started-with-rust`) is preserved;
//!     a credential value crafted to mimic that shape (random parts joined by
//!     hyphens) with no armed predecessor would also pass.
//!   - A short opaque token (`len < 12`) with no known shape and no armed
//!     predecessor still passes.
//! These are the deliberate false-negatives kept to avoid mangling legitimate
//! document ids, SHAs, UUIDs, and human-readable slugs.

use secret_redaction::{redact_searchable_text, RedactionContext};
use url::Url;

/// Placeholder emitted for an armed-but-not-known-shape opaque token.
///
/// We reuse `secret-redaction`'s `ACCESS_TOKEN` marker so the positional-arming
/// pass produces text indistinguishable from the known-shape redaction pass
/// (both emit `[REDACTED_SECRET: ...]` markers from the same vocabulary).
const ARMED_TOKEN_PLACEHOLDER: &str = "[REDACTED_SECRET: ACCESS_TOKEN]";

/// Path segments (sub-parts) that, when they immediately precede a token-shaped
/// segment, "arm" that following segment for redaction (the credential-bearing
/// tail of a reset / verify / magic-link / OTP / auth / share flow). The keyword
/// itself stays visible — only the armed token is redacted.
///
/// Entries are compared after `normalize_keyword` (lowercased, non-alphanumerics
/// dropped), so `reset-password`, `Reset_Password`, and `resetpassword` all
/// match `resetpassword`. Compound reorderings are listed explicitly because
/// normalization cannot reorder (`password-reset` -> `passwordreset`).
///
/// NOTE: generic resource-id carriers (`d`, `document`, `commit`, `users`,
/// `blob`, `tree`, …) are deliberately NOT here — they must stay non-arming so
/// document ids / SHAs / UUIDs survive. They live in
/// `RESOURCE_ID_PREDECESSORS` instead.
const ARMED_PREDECESSORS: &[&str] = &[
    "reset",
    "resetpassword",
    "passwordreset",
    "forgotpassword",
    "forgot",
    "verify",
    "verifyemail",
    "emailverification",
    "verification",
    "confirm",
    "confirmemail",
    "confirmaccount",
    "activate",
    "activation",
    "recover",
    "recovery",
    "invite",
    "invitation",
    "magic",
    "magiclink",
    "otp",
    "token",
    "auth",
    // Generic credential / share / redirect carriers. These do NOT collide with
    // the resource-id carriers in `RESOURCE_ID_PREDECESSORS`.
    "click",
    "login",
    "signin",
    "share",
    "link",
];

/// Segments whose immediate successor is a RESOURCE ID, not a credential. When
/// one of these precedes a high-entropy token, the token is PRESERVED — these
/// carriers front document ids, commit SHAs, object ids, and user ids that the
/// model needs to see.
///
/// This list is consulted only by the high-entropy backstop; the positional
/// arming pass never treats these as armed.
const RESOURCE_ID_PREDECESSORS: &[&str] = &[
    "d",
    "document",
    "commit",
    "commits",
    "blob",
    "tree",
    "users",
    "user",
    "raw",
    "objects",
    "id",
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

    // 3 + 4 + 5. Both redaction passes run PER PATH SUB-PART (and the authority
    //         is redacted in isolation). Segmenting first matters: the
    //         known-shape Password detector matches `password` only when an 8+
    //         char value follows it, so over the joined `reset-password/AbC9x…`
    //         string it would eat the keyword *and* the token together.
    //         Per-segment, the standalone `reset-password` keyword survives (no
    //         value attached) while secrets that legitimately live inside one
    //         segment are still caught — secrets never span a `/`.
    let guarded_authority = redact_known_shape(&authority);

    let mut redacted_count = 0usize;
    let mut passed_count = 0usize;

    // `path` always begins with '/' for http(s) URLs. Splitting on '/' keeps
    // the empty leading/trailing segments so we can rejoin faithfully. The
    // `url` crate leaves `%2F` verbatim in `path()`, so we additionally split
    // each segment on `%2F`/`%2f` into sub-parts (see `process_segment`) and
    // run the same arming logic across the sub-parts.
    let segments: Vec<&str> = path.split('/').collect();
    let mut out: Vec<String> = Vec::with_capacity(segments.len());
    // Track the previous NON-EMPTY raw sub-part as the arming predecessor;
    // arming is decided from the ORIGINAL sub-part text, not the redacted form.
    let mut prev_keyword: Option<String> = None;
    for segment in &segments {
        if segment.is_empty() {
            out.push(String::new());
            continue;
        }
        let processed = process_segment(
            segment,
            &mut prev_keyword,
            &mut redacted_count,
            &mut passed_count,
        );
        out.push(processed);
    }
    let guarded_path = out.join("/");

    // 6. Observability: one debug line per call. Never logs URL contents at
    //    info level; the counts keep the residual observable.
    log::debug!(
        "brokered url_guard positional-arming: redacted={redacted_count} passed={passed_count}"
    );

    Some(format!("{guarded_authority}{guarded_path}"))
}

/// Process one '/'-delimited path segment, which may itself contain one or more
/// `%2F`/`%2f`-encoded slashes. Splits the segment into sub-parts on those
/// encoded slashes, applies the arming + redaction passes across the sub-parts
/// (a keyword sub-part arms the following sub-part), and rejoins faithfully,
/// preserving the original `%2F`/`%2f` delimiter casing so the output still
/// reads correctly (keyword visible, token -> placeholder).
///
/// `prev_keyword` is threaded through so arming carries across the whole
/// segment (and into the next segment via the last sub-part).
fn process_segment(
    segment: &str,
    prev_keyword: &mut Option<String>,
    redacted_count: &mut usize,
    passed_count: &mut usize,
) -> String {
    // Split on `%2F`/`%2f` while remembering each delimiter's exact casing so we
    // can reproduce it on rejoin.
    let (sub_parts, delimiters) = split_encoded_slash(segment);

    let mut rebuilt = String::new();
    for (i, part) in sub_parts.iter().enumerate() {
        if i > 0 {
            // Re-emit the original delimiter that separated part i-1 from part i.
            rebuilt.push_str(delimiters[i - 1]);
        }
        if part.is_empty() {
            // An empty sub-part (e.g. trailing `%2F`) cannot be a token and
            // does not arm anything; leave the predecessor untouched.
            continue;
        }
        let armed = prev_keyword
            .as_deref()
            .map(is_armed_predecessor)
            .unwrap_or(false);
        let prev_is_resource_carrier = prev_keyword
            .as_deref()
            .map(is_resource_id_predecessor)
            .unwrap_or(false);

        if armed && is_armed_opaque(part) {
            // Positional-arming pass: an opaque token after a credential
            // keyword. Arming establishes credential intent, so we redact ANY
            // opaque `len >= 12` token regardless of character class.
            rebuilt.push_str(ARMED_TOKEN_PLACEHOLDER);
            *redacted_count += 1;
        } else if !prev_is_resource_carrier && is_backstop_token(part) {
            // High-entropy backstop: an obviously-random token with no armed
            // predecessor and no resource-id carrier in front of it. UUIDs,
            // all-hex SHAs, and resource ids are excluded inside
            // `is_backstop_token` / via `prev_is_resource_carrier`.
            rebuilt.push_str(ARMED_TOKEN_PLACEHOLDER);
            *redacted_count += 1;
        } else {
            // Known-shape redaction within this single sub-part.
            rebuilt.push_str(&redact_known_shape(part));
            *passed_count += 1;
        }
        // The current (raw) sub-part becomes the predecessor for the next one
        // (within this segment and, for the last sub-part, the next segment).
        *prev_keyword = Some(normalize_keyword(part));
    }
    rebuilt
}

/// Split a path segment on `%2F`/`%2f` (case-insensitive), returning the
/// sub-parts AND the exact delimiter strings that separated them (so casing is
/// preserved on rejoin). For a segment with no encoded slash this returns a
/// single sub-part and no delimiters.
fn split_encoded_slash(segment: &str) -> (Vec<&str>, Vec<&str>) {
    let bytes = segment.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut delimiters: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 3 <= segment.len() {
        if bytes[i] == b'%'
            && (bytes[i + 1] == b'2')
            && (bytes[i + 2] == b'F' || bytes[i + 2] == b'f')
        {
            parts.push(&segment[start..i]);
            delimiters.push(&segment[i..i + 3]);
            i += 3;
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&segment[start..]);
    (parts, delimiters)
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

/// Case-insensitive, non-alphanumeric-ignoring membership test against the
/// resource-id carrier list. A successor of one of these is a resource id, not
/// a credential, so the high-entropy backstop must leave it alone.
fn is_resource_id_predecessor(segment: &str) -> bool {
    RESOURCE_ID_PREDECESSORS.contains(&normalize_keyword(segment).as_str())
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

/// Aggressive opacity test used ONLY on the ARMED path. Because the predecessor
/// already established credential intent, any opaque token of `len >= 12` is
/// redacted regardless of character class — this catches all-lowercase,
/// all-upper, all-digit, and `prefix_<opaque>` reset / invite / OTP / share
/// tokens that the conservative `is_token_shaped` test would miss.
///
/// We still require `len >= 12` so a short dictionary word after a keyword
/// (e.g. `verify/email`) survives. The ONLY thing we refuse to redact here is a
/// hyphen-separated word slug, so an armed-but-human-readable tail (rare) is
/// not mangled. Underscore-joined tokens like `test_abc123XYZ…` are NOT word
/// slugs and ARE redacted.
fn is_armed_opaque(segment: &str) -> bool {
    if segment.chars().count() < 12 {
        return false;
    }
    !is_hyphen_word_slug(segment)
}

/// True when the segment looks like a hyphen-separated sequence of
/// human-readable words (e.g. `getting-started-with-rust`,
/// `my-awesome-post-2024`): two or more `-`-delimited parts where every part is
/// purely alphanumeric and at least one part is purely alphabetic. Random
/// tokens are single runs (no hyphens) and so are never word slugs.
fn is_hyphen_word_slug(segment: &str) -> bool {
    let parts: Vec<&str> = segment.split('-').collect();
    if parts.len() < 2 {
        return false;
    }
    let mut has_alpha_word = false;
    for part in &parts {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        if part.bytes().all(|b| b.is_ascii_alphabetic()) {
            has_alpha_word = true;
        }
    }
    has_alpha_word
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

/// High-entropy backstop candidate test. Redact a segment when it is an
/// obviously-random, separator-light token — but NEVER a recognized resource
/// id. Resource ids are excluded here so that document ids, commit SHAs, and
/// UUIDs survive even though they are token-shaped.
///
/// A segment is a backstop token when:
///   - it is token-shaped (`len >= 12`, mixed character class), AND
///   - it is a single run (no hyphen-separated dictionary-word structure), so
///     human-readable slugs like `getting-started-with-rust` are preserved, AND
///   - it is NOT a UUID, AND
///   - it is NOT all-hex (covers commit SHAs).
///
/// (Resource-id carriers in front of the token are handled by the caller via
/// `is_resource_id_predecessor`, so a token after `/d/` survives.)
fn is_backstop_token(segment: &str) -> bool {
    if !is_token_shaped(segment) {
        return false;
    }
    if !is_single_run(segment) {
        return false;
    }
    if is_uuid(segment) {
        return false;
    }
    if is_all_hex(segment) {
        return false;
    }
    true
}

/// True when the segment is a single contiguous alphanumeric run — no `-`, `_`,
/// `.`, or other separators that would indicate a hyphen-joined word slug.
/// Random opaque tokens are single runs; content slugs are not.
fn is_single_run(segment: &str) -> bool {
    segment.chars().all(|c| c.is_ascii_alphanumeric())
}

/// True for the canonical 8-4-4-4-12 hex UUID shape (case-insensitive).
fn is_uuid(segment: &str) -> bool {
    let groups: Vec<&str> = segment.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    let expected = [8usize, 4, 4, 4, 12];
    for (group, &len) in groups.iter().zip(expected.iter()) {
        if group.len() != len || !group.bytes().all(|b| b.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

/// True when every character is a hex digit (covers commit SHAs). Used to
/// preserve all-hex resource ids in the backstop. Requires at least one
/// character; an empty string is not hex.
fn is_all_hex(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(raw: &str) -> Option<String> {
        guard_url(raw)
    }

    // --- MUST be redacted (armed predecessor) ---

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

    // --- Fix 1: %2F-encoded slash bypass ---

    #[test]
    fn encoded_slash_verify_token_is_redacted() {
        let out = guard("https://app.com/verify%2FAbC9xK2mP4qR7sT0").unwrap();
        assert!(out.contains("verify"), "keyword must stay visible: {out}");
        assert!(!out.contains("AbC9xK2mP4qR7sT0"), "armed token must go: {out}");
        assert_eq!(
            out,
            format!("app.com/verify%2F{ARMED_TOKEN_PLACEHOLDER}")
        );
    }

    #[test]
    fn encoded_slash_lowercase_delimiter_preserved() {
        let out = guard("https://app.com/invite%2fAbC9xK2mP4qR7sT0").unwrap();
        assert!(!out.contains("AbC9xK2mP4qR7sT0"), "{out}");
        assert_eq!(
            out,
            format!("app.com/invite%2f{ARMED_TOKEN_PLACEHOLDER}")
        );
    }

    #[test]
    fn encoded_slash_magiclink_token_is_redacted() {
        let out = guard("https://app.com/magiclink%2FzzzzzzzzzzzzzzzzT0").unwrap();
        assert!(!out.contains("zzzzzzzzzzzzzzzzT0"), "{out}");
        assert!(out.contains("magiclink"), "{out}");
    }

    // --- Fix 2: single-class armed tokens ---

    #[test]
    fn armed_all_lowercase_token_is_redacted() {
        let out = guard("https://site.com/reset-password/abcdefghijklmnopqrstuvwxyz").unwrap();
        assert!(
            !out.contains("abcdefghijklmnopqrstuvwxyz"),
            "all-lowercase armed token must be redacted: {out}"
        );
        assert!(out.contains("reset-password"), "{out}");
    }

    #[test]
    fn armed_all_upper_token_is_redacted() {
        let out = guard("https://site.com/reset/ABCDEFGHIJKLMNOPQRST").unwrap();
        assert!(
            !out.contains("ABCDEFGHIJKLMNOPQRST"),
            "all-upper armed token must be redacted: {out}"
        );
        assert!(out.contains("reset"), "{out}");
    }

    #[test]
    fn armed_all_digit_token_is_redacted() {
        let out = guard("https://site.com/invite/019283746501928374").unwrap();
        assert!(
            !out.contains("019283746501928374"),
            "all-digit armed token must be redacted: {out}"
        );
        assert!(out.contains("invite"), "{out}");
    }

    // --- Fix 3: compound reorderings + share/redirect carriers ---

    #[test]
    fn password_reset_compound_is_armed() {
        let out = guard("https://site.com/password-reset/AbC9xK2mP4qR7sT0").unwrap();
        assert!(!out.contains("AbC9xK2mP4qR7sT0"), "{out}");
        assert!(out.contains("password-reset"), "{out}");
    }

    #[test]
    fn confirm_email_compound_is_armed() {
        let out = guard("https://app.com/confirm-email/AbC9xK2mP4qR7sT0").unwrap();
        assert!(!out.contains("AbC9xK2mP4qR7sT0"), "{out}");
        assert!(out.contains("confirm-email"), "{out}");
    }

    #[test]
    fn sendgrid_click_carrier_token_is_redacted() {
        let out = guard("https://u.ct.sendgrid.net/ls/click/abc123XYZ456def789ghij").unwrap();
        assert!(
            !out.contains("abc123XYZ456def789ghij"),
            "click-carried token must be redacted: {out}"
        );
        assert!(out.contains("click"), "{out}");
    }

    #[test]
    fn stripe_login_carrier_token_is_redacted() {
        let out = guard("https://billing.stripe.com/p/login/test_abc123XYZdef456ghi").unwrap();
        assert!(
            !out.contains("test_abc123XYZdef456ghi"),
            "login-carried token must be redacted: {out}"
        );
        assert!(out.contains("login"), "{out}");
    }

    // --- Fix 4: high-entropy backstop on generic carriers ---

    #[test]
    fn dropbox_scl_fi_token_is_redacted() {
        let out = guard("https://www.dropbox.com/scl/fi/abc123def456ghi789jkl/x").unwrap();
        assert!(
            !out.contains("abc123def456ghi789jkl"),
            "share token after generic carrier must be redacted: {out}"
        );
    }

    #[test]
    fn bare_share_token_is_redacted_by_backstop() {
        let out = guard("https://app.com/s/AbC9xK2mP4qR7sT0").unwrap();
        assert!(
            !out.contains("AbC9xK2mP4qR7sT0"),
            "bare high-entropy share token must be redacted: {out}"
        );
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
            "doc id must survive (predecessor `d` is a resource-id carrier): {out}"
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
            "SHA must survive (all-hex): {out}"
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
            "UUID must survive: {out}"
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

    #[test]
    fn content_slug_survives() {
        let out = guard("https://blog.example.com/posts/my-awesome-post-2024").unwrap();
        assert!(
            out.contains("my-awesome-post-2024"),
            "hyphenated content slug must survive: {out}"
        );
        assert_eq!(out, "blog.example.com/posts/my-awesome-post-2024");
    }

    #[test]
    fn guide_slug_survives() {
        let out = guard("https://docs.example.com/guide/getting-started-with-rust").unwrap();
        assert!(
            out.contains("getting-started-with-rust"),
            "hyphenated slug must survive: {out}"
        );
        assert_eq!(out, "docs.example.com/guide/getting-started-with-rust");
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

    // --- Backstop unit-level sanity ---

    #[test]
    fn uuid_detector_is_strict() {
        assert!(is_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_uuid("550e8400-e29b-41d4-a716-44665544000")); // last group short
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid("550e8400e29b41d4a716446655440000")); // no dashes -> single run
    }

    #[test]
    fn all_hex_detector_covers_sha() {
        assert!(is_all_hex("9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"));
        assert!(!is_all_hex("9fceb02zzz"));
        assert!(!is_all_hex(""));
    }

    #[test]
    fn single_run_distinguishes_token_from_slug() {
        assert!(is_single_run("AbC9xK2mP4qR7sT0"));
        assert!(!is_single_run("my-awesome-post-2024"));
        assert!(!is_single_run("getting-started-with-rust"));
    }
}
