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
//! 3. **High-entropy backstop** — even with no armed predecessor, an opaque
//!    token (`len >= 12`, mixed character class) is redacted by default. The
//!    opaque run counts `-`, `_`, `.`, `~`, and `+` as part of one token, so
//!    base64url (`A-Za-z0-9-_`), underscore-joined, and dotted (`v1.<rand>`)
//!    secrets no longer slip through on the strength of a single separator.
//!    A backstop token is PRESERVED only when it is a human-readable hyphen
//!    word slug (`getting-started-with-rust`, ≥1 purely-alphabetic part) or a
//!    recognized PUBLIC-ID shape (a UUID or an all-hex string / commit SHA).
//!    Generic resource-id carriers (`d`, `document`, `commit`, `users`, `id`,
//!    `raw`, … see `RESOURCE_ID_PREDECESSORS`) no longer blanket-rescue their
//!    successor: a token after such a carrier survives ONLY if it is itself a
//!    public-id shape (UUID / all-hex / all-numeric); a mixed-class opaque token
//!    after `/id/`, `/raw/`, `/user/` is redacted like any other. A sub-part
//!    that carries `%XX` escapes is percent-DECODED before the opacity test
//!    (decode-then-rescan), so a token hiding behind encoded `=`/`+` padding
//!    (`dGhpc2lzYXNlY3JldA%3D%3D`) is judged exactly as its decoded form and
//!    redacted, while readable encoded content (`Hello%20World` -> a space)
//!    stays non-opaque and survives. `%` itself is never an opaque char.
//!
//! Accepted residual (what still passes — kept deliberately, stated honestly):
//!   - A UUID or an all-hex string (commit SHA / object id) is preserved as a
//!     public-id shape even though, without surrounding context, a 32-hex string
//!     COULD in principle be a secret. This residual is unavoidable here: bare
//!     SHAs and UUIDs legitimately appear in paths and the model needs them.
//!   - A carrier-LESS opaque PUBLIC id with no hyphen-word structure (e.g. a
//!     Spotify `track/4cOdK2wGLETKBW3PvgPWqT` base62 id) is OVER-redacted — it
//!     is indistinguishable from a share token at read time, and we favor
//!     redaction. This is an accepted false-positive, not a leak.
//!   - A content slug that is hyphen-separated into word-like parts with ≥1
//!     purely-alphabetic part (`getting-started-with-rust`) is preserved; a
//!     credential crafted to mimic that exact shape (random parts joined by
//!     hyphens, at least one all-alpha part) with no armed predecessor would
//!     also pass. Note the armed pass drops this exemption entirely.
//!   - A short opaque token (`len < 12`) with no known shape and no armed
//!     predecessor still passes.
//!   - A `len >= 12` opaque token that is SINGLE character class with no digit
//!     (all-lowercase OR all-uppercase letters) and has no armed predecessor is
//!     PRESERVED (e.g. `/s/abcdefghijklmnopqr`). The backstop's mixed-class gate
//!     is exactly what keeps human-readable dictionary path words
//!     (`documentation`, `introduction`, `accessibility`, `notifications`) from
//!     being redacted, so we cannot tighten it without gutting readable URL
//!     content; telling a random all-lowercase token from a dictionary word
//!     would need a dictionary/entropy heuristic that is out of scope and
//!     fragile. The armed pass (`is_armed_opaque`, len-only) still redacts these
//!     in credential flows, and query/fragment — the dominant token vector — is
//!     already stripped wholesale, so this residual is narrow.
//! Everything else opaque — base64url / underscore / dotted high-entropy tokens,
//! mixed-class tokens after generic carriers, and ANY `len >= 12` token after an
//! armed credential keyword (hyphens and all) — is now redacted.

use secret_redaction::{redact_searchable_text, RedactionContext};
use std::borrow::Cow;
use url::Url;

/// Placeholder emitted for an armed-but-not-known-shape opaque token.
///
/// We reuse `secret-redaction`'s `ACCESS_TOKEN` marker so the positional-arming
/// pass produces text indistinguishable from the known-shape redaction pass
/// (both emit `[REDACTED_SECRET: ...]` markers from the same vocabulary).
const ARMED_TOKEN_PLACEHOLDER: &str = "[REDACTED_SECRET: ACCESS_TOKEN]";

/// Upper bound on the path bytes the guard will redact per URL.
///
/// The guard runs the per-sub-part redaction passes (incl. the `secret-redaction`
/// regex driver, which does NOT apply its own `max_surface_chars` cap on the
/// `redact_searchable_text` path) once per sub-part, so total CPU is linear in
/// path length. Path length is REMOTELY influenceable: a visited page controls
/// its own URL, and a malicious site can navigate to a multi-megabyte path that
/// is then captured and re-guarded at the broker boundary on EVERY search hit and
/// EVERY timeline interval (up to `MAX_SEARCH_LIMIT` per page). Capping the
/// guarded path keeps each `guard_url` call constant-bounded regardless of the
/// captured URL's size. 8 KiB comfortably exceeds any legitimate human-navigated
/// path (browsers themselves cap around 2 KB of address-bar URL); anything past it
/// is adversarial padding or an oversized token, and the overflow is redacted
/// wholesale so a token straddling the cut cannot leak.
const MAX_GUARDED_PATH_BYTES: usize = 8 * 1024;

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
    // Session-id path parameters (`;jsessionid=<token>`, `;sid=<token>`, …). The
    // value after `=` is a live session credential; arming it here redacts even a
    // single-character-class session id the high-entropy backstop would preserve.
    "jsessionid",
    "sessionid",
    "phpsessid",
    "sid",
    "session",
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
    let full_path = sanitized.path();

    // Bound the redacted path to a constant so the linear per-sub-part work
    // (incl. the un-`max_surface_chars`-capped `redact_searchable_text` regex
    // driver) can never be driven super-large by a remotely-influenceable URL
    // length. Cut on a char boundary; when the path overflows, the truncated
    // remainder is redacted wholesale (a token straddling the cut cannot leak).
    let path_overflowed = full_path.len() > MAX_GUARDED_PATH_BYTES;
    let path = if path_overflowed {
        let mut cut = MAX_GUARDED_PATH_BYTES;
        while cut > 0 && !full_path.is_char_boundary(cut) {
            cut -= 1;
        }
        &full_path[..cut]
    } else {
        full_path
    };

    // 3 + 4 + 5. Both redaction passes run PER PATH SUB-PART (and the authority
    //         is redacted in isolation). Segmenting first matters: the
    //         known-shape Password detector matches `password` only when an 8+
    //         char value follows it, so over the joined `reset-password/AbC9x…`
    //         string it would eat the keyword *and* the token together.
    //         Per-segment, the standalone `reset-password` keyword survives (no
    //         value attached) while secrets that legitimately live inside one
    //         segment are still caught — secrets never span a `/`.
    let guarded_authority = redact_known_shape(&authority);

    let mut guarded_path = redact_path(path);
    if path_overflowed {
        // The path exceeded the guard's bound and was cut. The dropped remainder
        // could be (part of) a secret token, so emit a redaction marker in its
        // place rather than silently truncating.
        guarded_path.push_str(ARMED_TOKEN_PLACEHOLDER);
    }

    Some(format!("{guarded_authority}{guarded_path}"))
}

/// Apply the per-path-sub-part arming + redaction passes to one already-cut
/// path and return the guarded path string. Shared by [`guard_url`] (the
/// model-text boundary) and [`secret_scrubbed_fetch_target`] (the fetch-target
/// boundary) so BOTH redact magic-link / reset / share tokens in the path
/// identically — a redacted path 404s at the origin, which is the fetch
/// target's fail-closed behavior.
///
/// `path` always begins with '/' for http(s) URLs. Splitting on '/' keeps the
/// empty leading/trailing segments so we can rejoin faithfully. The `url` crate
/// leaves `%2F` verbatim in `path()`, so we additionally split each segment on
/// `%2F`/`%2f` into sub-parts (see `process_segment`) and run the same arming
/// logic across the sub-parts.
fn redact_path(path: &str) -> String {
    let mut redacted_count = 0usize;
    let mut passed_count = 0usize;

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

    // Observability: one debug line per call. Never logs URL contents at info
    // level; the counts keep the residual observable.
    log::debug!(
        "brokered url_guard positional-arming: redacted={redacted_count} passed={passed_count}"
    );

    out.join("/")
}

/// Query-parameter NAMES that mark a credential-bearing param (compared after
/// [`normalize_keyword`], which lowercases and drops non-alphanumerics, so
/// `access_token`, `Access-Token`, and `accesstoken` all collapse to
/// `accesstoken`). Used only by [`secret_scrubbed_fetch_target`]: a param with
/// one of these names is DROPPED from the fetch target regardless of its value.
const CREDENTIAL_PARAM_NAMES: &[&str] = &[
    "token",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "code",
    "auth",
    "authorization",
    "bearer",
    "jwt",
    "key",
    "apikey",
    "apisecret",
    "secret",
    "clientsecret",
    "password",
    "passwd",
    "pwd",
    "session",
    "sessionid",
    "sid",
    "sig",
    "signature",
    "otp",
    "credential",
    "credentials",
];

/// True when a query-param NAME is credential-shaped (member of
/// [`CREDENTIAL_PARAM_NAMES`] after normalization).
fn is_credential_param_name(name: &str) -> bool {
    CREDENTIAL_PARAM_NAMES.contains(&normalize_keyword(name).as_str())
}

/// True when a query-param VALUE is secret-shaped per the module's deterministic
/// detector: a known fixed-prefix token (`ghp_`, `sk-`, JWT, …) OR a
/// high-entropy backstop token. Reuses the exact machinery `guard_url` applies
/// to path segments, so the two boundaries judge secrets identically.
fn is_secret_value(value: &str) -> bool {
    redact_known_shape(value) != value || is_backstop_token(value)
}

/// Fetch-target boundary: raw captured URL -> `Option<absolute URL string>` that
/// is safe to send to the ORIGIN over the network.
///
/// Contrast with [`guard_url`], the MODEL-TEXT boundary (strips the whole query
/// and emits only `host[:port]/path`). The origin already knows its own URL, so
/// here we keep everything EXCEPT secrets (the "two-boundary" design, grill G6):
///
/// - Scheme forced to **https**.
/// - **Query params**: a param is DROPPED when its NAME is credential-shaped
///   ([`is_credential_param_name`]) OR its VALUE is secret-shaped
///   ([`is_secret_value`]); every other param is kept verbatim (`?v=`, `?id=`,
///   `?tab=`, `?page=` survive).
/// - **Path**: the SAME per-segment secret redaction [`guard_url`] applies via
///   [`redact_path`], so a magic-link / reset path token becomes a
///   `[REDACTED_SECRET: …]` placeholder and the target 404s at the origin —
///   fail closed by design.
/// - **Fragment**: dropped (never sent to the origin anyway).
///
/// Returns `None` for a non-`http(s)` or unparseable input.
pub fn secret_scrubbed_fetch_target(raw_url: &str) -> Option<String> {
    let parsed = Url::parse(raw_url).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }

    // SSRF fail-closed: never produce a fetch target for a loopback / private /
    // link-local / metadata host. The captured URL comes from the user's own
    // browsing history, which can legitimately include `http://localhost:3000`,
    // a LAN device (`https://192.168.1.1`), or a link-local address — fetching it
    // and streaming the body into a (possibly cloud) model exfiltrates
    // internal-network / localhost service content. Screened here (initial
    // target) AND on every redirect hop (see `is_disallowed_fetch_url`).
    if is_disallowed_fetch_url(&parsed) {
        return None;
    }

    let mut target = parsed.clone();
    // Force https. http and https are both "special" schemes, so this switch is
    // always permitted; a failure here means an unexpected input, so bail.
    if target.set_scheme("https").is_err() {
        return None;
    }

    // Strip URL-embedded Basic-auth credentials (`user:pass@`). They are a secret,
    // and the cookie-less fetch client can never ride the user's authenticated
    // session — keeping userinfo would send the credential over the network as an
    // `Authorization: Basic` header and do exactly that. Fail closed (a 401 is the
    // safe outcome), matching the model-text boundary (`guard_url`), which already
    // emits only host+port and never userinfo.
    let _ = target.set_username("");
    let _ = target.set_password(None);

    // Path: redact secrets exactly as the model-text boundary does. A redacted
    // magic-link / reset token becomes a nonsense path -> 404 at the origin.
    let redacted_path = redact_path(parsed.path());
    target.set_path(&redacted_path);

    // Query: keep every param whose name is not credential-shaped and whose
    // value is not secret-shaped. `query_pairs()` percent-decodes; re-appending
    // through `query_pairs_mut` re-encodes, so kept params round-trip faithfully.
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(name, value)| !is_credential_param_name(name) && !is_secret_value(value))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        target.set_query(None);
    } else {
        target.query_pairs_mut().clear();
        for (name, value) in &kept {
            target.query_pairs_mut().append_pair(name, value);
        }
    }

    target.set_fragment(None);
    Some(target.to_string())
}

/// SSRF host screen for the `fetch_url` egress. Returns `true` when `url`'s host
/// must NEVER be fetched: a loopback, private (RFC 1918), link-local (incl. the
/// cloud metadata IP `169.254.169.254`), unique-local, carrier-grade-NAT, or
/// unspecified address, or the `localhost` name. Applied to BOTH the initial
/// fetch target ([`secret_scrubbed_fetch_target`]) and every redirect hop (the
/// origin controls `Location`, so a benign captured URL can be redirected into
/// internal infrastructure). The `url` crate parses decimal / hex / octal IPv4
/// literals (`http://2130706433`, `http://0x7f000001`) into `Host::Ipv4`, so
/// those encodings are screened too.
///
/// Residual (documented, not fixed here): a DNS *name* that resolves to a private
/// IP (a rebinding vector) is NOT caught — that needs a resolving connector. IP
/// literals and `localhost` — the concrete, testable vectors — are closed.
pub fn is_disallowed_fetch_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ipv4_is_disallowed(ip),
        Some(url::Host::Ipv6(ip)) => ipv6_is_disallowed(ip),
        Some(url::Host::Domain(host)) => host_is_local_name(host),
        // No host (opaque / non-network URL) — never fetch.
        None => true,
    }
}

/// SSRF screen on a RESOLVED connection address. This is the real boundary the
/// fetch client's custom DNS resolver applies: it catches a public host NAME that
/// resolves to a private IP (split-horizon DNS / DNS-rebinding) — the case
/// [`is_disallowed_fetch_url`] (URL host only) cannot see — as well as IP-literal
/// hosts and every redirect hop's connection. Returns `true` when the client must
/// refuse to connect to `ip`.
pub fn ip_is_disallowed_fetch_target(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => ipv4_is_disallowed(v4),
        std::net::IpAddr::V6(v6) => ipv6_is_disallowed(v6),
    }
}

/// True for an IPv4 address the fetch client must never reach.
fn ipv4_is_disallowed(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private()          // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()  // 127/8
        || ip.is_link_local()// 169.254/16 (incl. metadata 169.254.169.254)
        || ip.is_unspecified()// 0.0.0.0
        || ip.is_broadcast() // 255.255.255.255
        || a == 0            // 0.0.0.0/8 "this network"
        || (a == 100 && (b & 0xc0) == 64) // 100.64/10 carrier-grade NAT
}

/// True for an IPv6 address the fetch client must never reach.
fn ipv6_is_disallowed(ip: std::net::Ipv6Addr) -> bool {
    // An address embedding an IPv4 reaches the same host — screen the embedded v4
    // with the v4 rules. `to_ipv4()` covers both IPv4-mapped (`::ffff:a.b.c.d`)
    // and the deprecated IPv4-compatible form (`::a.b.c.d`, e.g. `::7f00:1`), so
    // `[::ffff:169.254.169.254]` and `[::7f00:1]` are caught. (`::1`/`::` land
    // here too, as 0.0.0.1/0.0.0.0 — both disallowed v4s, same verdict.)
    if let Some(v4) = ip.to_ipv4() {
        return ipv4_is_disallowed(v4);
    }
    let seg = ip.segments();
    // NAT64 well-known prefix 64:ff9b::/96 (RFC 6052) also embeds an IPv4 in the
    // low 32 bits — screen it the same way.
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let v4 = std::net::Ipv4Addr::from((u32::from(seg[6]) << 16) | u32::from(seg[7]));
        return ipv4_is_disallowed(v4);
    }
    ip.is_loopback()              // ::1
        || ip.is_unspecified()    // ::
        || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
}

/// True when a host NAME is a loopback name (`localhost` or a `*.localhost`
/// subdomain, which RFC 6761 reserves to loopback).
fn host_is_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
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
    // Split on `%2F`/`%2f` (encoded slash), `;` (matrix / path-parameter
    // delimiter, e.g. `;jsessionid=<token>`), and `=` (so the VALUE of a
    // `name=value` path parameter becomes its own sub-part, with `name` as its
    // arming predecessor), remembering each delimiter's exact text so we can
    // reproduce it faithfully on rejoin.
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

        // A resource-id carrier (`d`, `commit`, `id`, `raw`, `user`, …) only
        // rescues its successor when that successor is itself a recognized
        // public-id shape (UUID / all-hex / all-numeric). A mixed-class opaque
        // token after a generic carrier (e.g. `/id/SuperSecretSessionToken…`)
        // is NOT a public id and must still fall through to the backstop.
        let carrier_rescues = prev_is_resource_carrier && is_public_id_shape(part);

        if armed && is_armed_opaque(part) {
            // Positional-arming pass: an opaque token after a credential
            // keyword. Arming establishes credential intent, so we redact ANY
            // opaque `len >= 12` token regardless of character class (hyphens
            // included — the hyphen-word-slug exemption is dropped here).
            rebuilt.push_str(ARMED_TOKEN_PLACEHOLDER);
            *redacted_count += 1;
        } else if !carrier_rescues && is_backstop_token(part) {
            // High-entropy backstop: an opaque token with no armed predecessor
            // that the carrier did not rescue. UUIDs, all-hex SHAs, and
            // hyphen-word slugs are excluded inside `is_backstop_token`.
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

/// Split a path segment on `%2F`/`%2f` (encoded slash, case-insensitive), `;`
/// (matrix / path-parameter delimiter), and `=` (path-parameter `name=value`
/// boundary), returning the sub-parts AND the exact delimiter strings that
/// separated them (so casing / delimiter is preserved on rejoin). For a segment
/// with none of these this returns a single sub-part and no delimiters.
///
/// Splitting on `;` and `=` matters for credential containment: a Java-EE
/// session id rides the path as `;jsessionid=<token>`, and generic matrix
/// parameters carry `;name=<token>`. Without isolating the value after `=`, the
/// `;`/`=` chars (not opaque-charset members) would take the whole segment out of
/// the backstop's reach and the session/token value would leak whole. Splitting
/// here lets the `name` (`jsessionid`, `sid`, …) arm the following value sub-part
/// and lets the backstop see the value as a standalone opaque run.
fn split_encoded_slash(segment: &str) -> (Vec<&str>, Vec<&str>) {
    let bytes = segment.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let mut delimiters: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < segment.len() {
        if i + 3 <= segment.len()
            && bytes[i] == b'%'
            && (bytes[i + 1] == b'2')
            && (bytes[i + 2] == b'F' || bytes[i + 2] == b'f')
        {
            parts.push(&segment[start..i]);
            delimiters.push(&segment[i..i + 3]);
            i += 3;
            start = i;
        } else if bytes[i] == b';' || bytes[i] == b'=' {
            parts.push(&segment[start..i]);
            delimiters.push(&segment[i..i + 1]);
            i += 1;
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
/// all-upper, all-digit, `prefix_<opaque>`, and hyphen-broken reset / invite /
/// OTP / share tokens that the conservative `is_token_shaped` test would miss.
///
/// We still require `len >= 12` so a short dictionary word after a keyword
/// (e.g. `verify/email`) survives. Unlike the backstop, the armed path does NOT
/// exempt hyphen word slugs: once a credential keyword has armed the position,
/// intent is established, so `reset-password/abc-9f3a2b-def-1c4e` is redacted
/// just like `reset/<opaque>`.
fn is_armed_opaque(segment: &str) -> bool {
    segment.chars().count() >= 12
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

/// Opaque-token shape test for the backstop — `len >= 12`, drawn entirely from
/// the OPAQUE charset (`A-Za-z0-9` plus the token separators `-`, `_`, `.`,
/// `~`, `+`), with a mixed character class.
///
/// Counting `-`/`_`/`.`/`~`/`+` as part of one opaque run is the priority fix:
/// real session / share / magic-link / reset tokens are overwhelmingly
/// base64url (`A-Za-z0-9-_`), underscore-joined, or dotted (`v1.<rand>`), and a
/// single such separator must NOT take the token out of scope.
///
/// A segment qualifies only if every char is in the opaque charset AND `len >=
/// 12` AND (`has_digit && has_ascii_alpha`) OR (`has_uppercase &&
/// has_lowercase`). Dictionary words like `email` survive (not mixed class, too
/// short), while `AbC9xK2mP4qR7s`, `ABCdef-123_GHIjkl`, and `v1.MR9aBcDeF_…`
/// all qualify.
///
/// The mixed-class requirement INTENTIONALLY lets single-character-class strings
/// pass: a long all-lowercase (or all-uppercase) word like `documentation` or
/// `accessibility` is exactly the human-readable URL content the backstop must
/// preserve. The unavoidable consequence is that a single-class opaque token
/// (`/s/abcdefghijklmnopqr`) with no armed predecessor also passes — an accepted
/// residual documented in the module header. The armed pass still covers
/// credential flows.
fn is_token_shaped(segment: &str) -> bool {
    // Standard-base64 (non-url-safe) tokens carry 0-2 trailing `=` padding
    // chars. `=` is not an opaque charset member, so without this strip a
    // padded base64 share/session token (`dGhpc2lzYXNlY3JldA==`) would bail the
    // opaque-char scan below and leak whole. Padding only ever appears at the
    // very end (max 2), and readable path words never end in `=`, so stripping
    // it here cannot rescue dictionary content into the token class.
    let segment = segment.trim_end_matches('=');
    if segment.chars().count() < 12 {
        return false;
    }
    let mut has_digit = false;
    let mut has_ascii_alpha = false;
    let mut has_upper = false;
    let mut has_lower = false;
    for c in segment.chars() {
        if !is_opaque_char(c) {
            // A char outside the opaque charset (e.g. an escaped `%`, a CJK
            // glyph) means this is not a single opaque run; leave it to the
            // known-shape pass.
            return false;
        }
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

/// True for a character that may appear inside one opaque token run: an ASCII
/// alphanumeric or one of the base64url / token separators `-`, `_`, `.`, `~`,
/// `+`. A single such separator no longer splits the token (the old
/// `is_single_run` bug, where `ABCdef-123_GHIjkl` read as a non-token).
fn is_opaque_char(c: char) -> bool {
    // The base64url / token separators PLUS the remaining RFC-3986 path
    // sub-delimiters and `:`/`@` pchars that the `url` crate keeps verbatim in
    // `path()`. Counting these as part of one opaque run is what stops a single
    // such char from bailing the backstop scan and leaking a mixed-class token
    // whole (e.g. `AbC9xK2m@P4qR7sT0`). `;` `=` `%2F` are deliberately EXCLUDED
    // (split into sub-parts by `split_encoded_slash`); `%` is excluded so
    // percent-encoded readable content (`Hello%20World`) is not mis-read as one
    // opaque token and over-redacted.
    //
    // `[` `]` `^` `|` are included too: the `url` crate's `SPECIAL_PATH_SEGMENT`
    // percent-encode set does NOT escape them, so they ride verbatim in `path()`
    // for http(s) URLs. Without them a mixed-class opaque token split by one of
    // these (`AbC9xK2mP4qR|7sT0xyz`) bailed the opaque scan and leaked whole —
    // the same class of leak the sub-delimiter pchars above closed.
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '-' | '_'
                | '.'
                | '~'
                | '+'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | ','
                | '@'
                | ':'
                | '['
                | ']'
                | '^'
                | '|'
        )
}

/// High-entropy backstop candidate test. Redact a segment when it is an opaque
/// token — but NEVER a human-readable slug or a recognized PUBLIC-ID shape.
///
/// This is the priority-bug fix: opacity, not "pure single alphanumeric run",
/// is the gate. A base64url / underscore / dotted token like
/// `ABCdef-123_GHIjkl` or `v1.MR9aBcDeF_…` is now redacted instead of passing
/// through on the strength of one separator.
///
/// A segment is a backstop token when:
///   - it is token-shaped (`len >= 12`, opaque charset, mixed character class),
///     AND
///   - it is NOT a hyphen WORD slug (`getting-started-with-rust`, ≥1 all-alpha
///     part), so human-readable content slugs are preserved, AND
///   - it is NOT a recognized public-id shape (UUID, or all-hex commit SHA).
///
/// (Resource-id carriers in front of the token are handled by the caller via
/// `is_resource_id_predecessor` + `is_public_id_shape`, so a UUID after `/d/`
/// survives while a mixed-class token after `/id/` does not.)
fn is_backstop_token(segment: &str) -> bool {
    // A percent-encoded sub-part can hide an opaque token behind `%XX` escapes
    // (standard-base64 `=` padding as `%3D`, `+` as `%2B`, or even the whole
    // token hex-escaped). `%` is intentionally NOT an opaque char — so readable
    // encoded content like `Hello%20World` is not mis-read as one token — which
    // means the raw `is_token_shaped` scan bails on the `%` and the encoded
    // token would leak whole. Decode-then-rescan closes that gap: a real token
    // (`dGhpc2lzYXNlY3JldA%3D%3D` -> `dGhpc2lzYXNlY3JldA==`) reads as a
    // mixed-class opaque run and redacts, while readable encoded content
    // (`Hello%20World` -> `Hello World`, carrying a space) stays non-opaque and
    // survives. The decoded form is used ONLY to DECIDE; the caller redacts the
    // ORIGINAL (still-encoded) sub-part, so no decoded text is ever emitted.
    let candidate: Cow<'_, str> = if segment.contains('%') {
        Cow::Owned(percent_decode_lenient(segment))
    } else {
        Cow::Borrowed(segment)
    };
    let candidate = candidate.as_ref();
    if !is_token_shaped(candidate) {
        return false;
    }
    if is_hyphen_word_slug(candidate) {
        return false;
    }
    if is_public_id_shape(candidate) {
        return false;
    }
    true
}

/// Percent-decode a path sub-part LENIENTLY into a UTF-8 string used ONLY for
/// the backstop's shape decision — never emitted. Valid `%XX` escapes are
/// decoded to their byte; a truncated or non-hex escape (`%`, `%Z`, `%ZZ`) is
/// left as a literal `%`, which (being a non-opaque char) keeps readable
/// content carrying a stray `%` (`50%off…`) out of the token class instead of
/// over-redacting it. Decoded bytes that are not valid UTF-8 collapse to the
/// replacement char (also non-opaque), so a non-ASCII decode can never be
/// mistaken for a base64 token — genuine base64/url tokens are pure ASCII.
///
/// The effect is that an encoded sub-part is judged exactly as its decoded
/// equivalent would be by the plain backstop path, keeping encoded and
/// non-encoded inputs consistent.
fn percent_decode_lenient(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hi = (bytes[i + 1] as char).to_digit(16).unwrap() as u8;
            let lo = (bytes[i + 2] as char).to_digit(16).unwrap() as u8;
            decoded.push((hi << 4) | lo);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// True when the segment is a recognized PUBLIC-ID shape: a UUID, an all-hex
/// string (commit SHA / object id), or an all-numeric id. These are the only
/// shapes a generic resource-id carrier is allowed to rescue, and the only
/// token-shaped strings the backstop preserves outright. All-numeric strings are
/// not token-shaped (no alpha) so they never reach the backstop, but the carrier
/// path uses this predicate directly.
fn is_public_id_shape(segment: &str) -> bool {
    is_uuid(segment) || is_all_hex(segment) || is_all_numeric(segment)
}

/// True when every character is an ASCII digit (a bare numeric resource id).
/// Requires at least one character.
fn is_all_numeric(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
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
    fn document_d_uuid_survives() {
        // Policy change: the `d` resource-id carrier now rescues its successor
        // ONLY when the successor is itself a public-id shape (UUID / all-hex /
        // all-numeric). A UUID document id after `/d/` survives.
        let out =
            guard("https://docs.google.com/document/d/550e8400-e29b-41d4-a716-446655440000/edit")
                .unwrap();
        assert!(
            out.contains("550e8400-e29b-41d4-a716-446655440000"),
            "UUID doc id after resource-id carrier `d` must survive: {out}"
        );
        assert_eq!(
            out,
            "docs.google.com/document/d/550e8400-e29b-41d4-a716-446655440000/edit"
        );
    }

    #[test]
    fn mixed_class_doc_id_after_carrier_is_redacted() {
        // Policy change (was `google_doc_id_survives`): a mixed-class opaque
        // Google-style doc id after `/d/` is NOT a public-id shape, so the
        // generic carrier no longer rescues it. Over-redacting an opaque public
        // id here is the accepted security-favoring tradeoff.
        let out =
            guard("https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz/edit").unwrap();
        assert!(
            !out.contains("1AbCdEfGhIjKlMnOpQrStUvWxYz"),
            "mixed-class opaque id after generic carrier must be redacted: {out}"
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

    // REGRESSION (deep-review finding): a mixed-class opaque token carrying ONE
    // RFC-3986 path sub-delimiter / pchar (`@ : , ! $ & ' ( ) *`) — chars the
    // `url` crate keeps verbatim in `path()` — bailed the backstop's opaque-char
    // scan and leaked the WHOLE token to the cloud model. No armed predecessor
    // (bare `/s/` carrier), so only the backstop could catch it. The module
    // header CLAIMS mixed-class opaque tokens are redacted and "a single such
    // separator must NOT take the token out of scope" — these chars were the
    // gap. Each must now redact.
    #[test]
    fn subdelim_broken_opaque_token_is_redacted_by_backstop() {
        for raw in [
            "https://app.com/s/AbC9xK2mP4qR@7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR:7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR,7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR!7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR$7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR&7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR'7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR(7sT0xyz)",
            "https://app.com/s/AbC9xK2mP4qR*7sT0xyz",
        ] {
            let out = guard(raw).unwrap();
            assert!(
                out.contains(ARMED_TOKEN_PLACEHOLDER),
                "sub-delim-broken mixed-class opaque token must redact: raw={raw} out={out}"
            );
            assert!(
                !out.contains("AbC9xK2mP4qR"),
                "the token must not leak: raw={raw} out={out}"
            );
        }
    }

    // REGRESSION (deep-review finding, sibling of the sub-delim gap above): the
    // `url` crate's SPECIAL_PATH_SEGMENT percent-encode set keeps `[`, `]`, `^`,
    // and `|` VERBATIM in `path()` (they are NOT in the set), yet none of them is
    // an `is_opaque_char` member and none is a split delimiter. A mixed-class
    // opaque token broken by ONE of these chars therefore bails the backstop's
    // opaque-char scan exactly like the `@ : , ! …` chars did before their fix —
    // and with a bare `/s/` carrier (no armed predecessor) ONLY the backstop can
    // catch it, so the WHOLE token leaks verbatim to the cloud model. Each must
    // now redact.
    #[test]
    fn bracket_caret_pipe_broken_opaque_token_is_redacted_by_backstop() {
        for raw in [
            "https://app.com/s/AbC9xK2mP4qR|7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR^7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR[7sT0xyz",
            "https://app.com/s/AbC9xK2mP4qR]7sT0xyz",
        ] {
            let out = guard(raw).unwrap();
            assert!(
                !out.contains("AbC9xK2mP4qR"),
                "the token must not leak: raw={raw} out={out}"
            );
            assert!(
                out.contains(ARMED_TOKEN_PLACEHOLDER),
                "bracket/caret/pipe-broken mixed-class opaque token must redact: raw={raw} out={out}"
            );
        }
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
    fn backstop_token_distinguishes_opaque_from_slug() {
        // Opaque tokens (alnum runs OR separator-broken base64url/dotted) are
        // backstop tokens; hyphen WORD slugs and public-id shapes are not.
        assert!(is_backstop_token("AbC9xK2mP4qR7sT0"));
        assert!(is_backstop_token("ABCdef-123_GHIjkl-456_MNOpqr"));
        assert!(is_backstop_token("v1.MR9aBcDeF_gHiJkLmNoPqRsTuVwXyZ"));
        assert!(!is_backstop_token("my-awesome-post-2024"));
        assert!(!is_backstop_token("getting-started-with-rust"));
        assert!(!is_backstop_token("550e8400-e29b-41d4-a716-446655440000")); // UUID
        assert!(!is_backstop_token("9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f")); // all-hex
    }

    #[test]
    fn opaque_char_counts_separators_as_one_run() {
        assert!(is_opaque_char('a'));
        assert!(is_opaque_char('9'));
        assert!(is_opaque_char('-'));
        assert!(is_opaque_char('_'));
        assert!(is_opaque_char('.'));
        assert!(is_opaque_char('~'));
        assert!(is_opaque_char('+'));
        assert!(!is_opaque_char('/'));
        assert!(!is_opaque_char('%'));
    }

    #[test]
    fn public_id_shape_covers_uuid_hex_numeric() {
        assert!(is_public_id_shape("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_public_id_shape("9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"));
        assert!(is_public_id_shape("019283746501928374"));
        assert!(!is_public_id_shape("1AbCdEfGhIjKlMnOpQrStUvWxYz")); // mixed class, not a public-id shape
    }

    // --- Priority bug: base64url / underscore / dotted opaque token leak ---
    // Each of these previously slipped through the `is_single_run` gate because
    // a single `-`/`_`/`.` made the run "not pure alphanumeric". They MUST now
    // redact via the opacity backstop.

    #[test]
    fn base64url_hyphen_underscore_token_is_redacted() {
        let out = guard("https://app.com/u/ABCdef-123_GHIjkl-456_MNOpqr").unwrap();
        assert!(
            !out.contains("ABCdef-123_GHIjkl-456_MNOpqr"),
            "base64url opaque token must be redacted: {out}"
        );
        assert_eq!(out, format!("app.com/u/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn dotted_prefix_token_is_redacted() {
        let out = guard("https://app.com/p/v1.MR9aBcDeF_gHiJkLmNoPqRsTuVwXyZ").unwrap();
        assert!(
            !out.contains("v1.MR9aBcDeF_gHiJkLmNoPqRsTuVwXyZ"),
            "dotted `v1.<rand>` token must be redacted: {out}"
        );
        assert_eq!(out, format!("app.com/p/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn underscore_joined_token_is_redacted() {
        let out = guard("https://app.com/p/aBcDeF123456_GHIjkl789012").unwrap();
        assert!(
            !out.contains("aBcDeF123456_GHIjkl789012"),
            "underscore-joined opaque token must be redacted: {out}"
        );
        assert_eq!(out, format!("app.com/p/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    // --- New policy boundary: mixed-class token after a generic carrier ---

    #[test]
    fn mixed_class_token_after_id_carrier_is_redacted() {
        // `id` is a resource-id carrier, but it only rescues public-id shapes.
        // A mixed-class opaque token after `/id/` is redacted, not preserved.
        let out = guard("https://app.com/id/SuperSecretSessionToken12345").unwrap();
        assert!(
            !out.contains("SuperSecretSessionToken12345"),
            "mixed-class token after `id` carrier must be redacted: {out}"
        );
        assert_eq!(out, format!("app.com/id/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    // --- New policy: armed path drops the hyphen-word-slug exemption ---

    #[test]
    fn armed_hyphen_broken_token_is_redacted() {
        // After a credential keyword, ANY `len >= 12` token redacts — even one
        // that would read as a hyphen word slug outside the armed position.
        let out = guard("https://app.com/reset-password/abc-9f3a2b-def-1c4e").unwrap();
        assert!(
            !out.contains("abc-9f3a2b-def-1c4e"),
            "armed hyphen-broken token must be redacted: {out}"
        );
        assert!(out.contains("reset-password"), "{out}");
        assert_eq!(out, format!("app.com/reset-password/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn armed_invite_hyphen_token_is_redacted() {
        let out = guard("https://app.com/invite/team-x9f3a2b1c4e7").unwrap();
        assert!(
            !out.contains("team-x9f3a2b1c4e7"),
            "armed invite token must be redacted: {out}"
        );
        assert!(out.contains("invite"), "{out}");
    }

    // --- New policy: carrier-less opaque public id over-redacts (tradeoff) ---

    #[test]
    fn carrier_less_opaque_id_is_over_redacted() {
        // ACCEPTED TRADEOFF: a Spotify-style base62 track id has no carrier and
        // no hyphen-word structure, so it is indistinguishable from a share
        // token at read time and is redacted. This is a false-positive, not a
        // leak — we favor redaction.
        let out = guard("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT").unwrap();
        assert!(
            !out.contains("4cOdK2wGLETKBW3PvgPWqT"),
            "carrier-less opaque id is over-redacted by design: {out}"
        );
    }

    // --- Documented residual: single-class opaque token with no armed
    //     predecessor passes (the mixed-class gate that preserves dictionary
    //     path words also lets this through). This pins an ACCEPTED residual,
    //     not a desired outcome — see the module header. ---

    #[test]
    fn single_class_lowercase_token_is_a_documented_residual() {
        // `s` is neither an armed predecessor nor a resource-id carrier, so the
        // 18-char all-lowercase token reaches only the backstop. The backstop's
        // mixed-class gate (which exists to preserve words like `documentation`)
        // does not fire on a single-class token, so it PASSES. This is the
        // accepted residual documented in the module header, not a goal.
        let out = guard("https://app.com/s/abcdefghijklmnopqr").unwrap();
        assert!(
            out.contains("abcdefghijklmnopqr"),
            "single-class lowercase token is preserved as a documented residual: {out}"
        );
        assert_eq!(out, "app.com/s/abcdefghijklmnopqr");
    }

    #[test]
    fn single_class_token_after_armed_predecessor_is_redacted() {
        // The SAME token, when it follows an armed credential keyword, IS
        // redacted by the armed pass (`is_armed_opaque`, len-only), proving the
        // armed path closes the residual for credential flows.
        let out = guard("https://app.com/reset/abcdefghijklmnopqr").unwrap();
        assert!(
            !out.contains("abcdefghijklmnopqr"),
            "single-class token after armed `reset` must be redacted: {out}"
        );
        assert!(out.contains("reset"), "{out}");
        assert_eq!(out, format!("app.com/reset/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn jsessionid_path_parameter_token_is_redacted() {
        // A Java-EE session id placed in the path as a matrix/path parameter
        // (`;jsessionid=<token>`) is a LIVE session credential. The `;` and `=`
        // are not opaque-charset members, so without splitting on them the whole
        // `page;jsessionid=ABCdef123456GHIjkl` segment bailed the backstop and the
        // session token leaked to the cloud model.
        let out = guard("https://site.com/page;jsessionid=ABCdef123456GHIjkl").unwrap();
        assert!(
            !out.contains("ABCdef123456GHIjkl"),
            "jsessionid path-parameter token must not leak: {out}"
        );
        assert!(
            out.contains("page"),
            "the resource name before the matrix param stays readable: {out}"
        );
    }

    #[test]
    fn standard_base64_padded_token_is_redacted_by_backstop() {
        // A standard-base64 (non-url-safe) share/session token carries `=`
        // padding. `=` is not in the opaque charset, so the old backstop bailed
        // and the FULL token leaked to the cloud model. `dGhpc2lzYXNlY3JldA==`
        // decodes to `thisisasecret`. It must be redacted.
        let out = guard("https://site.com/s/dGhpc2lzYXNlY3JldA==").unwrap();
        assert!(
            !out.contains("dGhpc2lzYXNlY3JldA=="),
            "standard-base64 padded token must not leak: {out}"
        );
    }

    #[test]
    fn percent_encoded_base64_token_is_redacted() {
        // Same secret as above, but its `=` padding is PERCENT-encoded (`%3D`).
        // `%` is deliberately not an opaque char (so readable `Hello%20World`
        // survives), which means the raw backstop scan bails and the encoded
        // token would leak. The backstop now decodes-then-rescans, so the
        // decoded `dGhpc2lzYXNlY3JldA==` reads as opaque and redacts.
        let out = guard("https://app.com/s/dGhpc2lzYXNlY3JldA%3D%3D").unwrap();
        assert!(
            !out.contains("dGhpc2lzYXNlY3JldA"),
            "percent-encoded token must not leak: {out}"
        );
        assert_eq!(out, format!("app.com/s/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn percent_encoded_readable_path_is_preserved() {
        // Guard against over-redaction: encoded spaces decode to readable text.
        let out = guard("https://docs.example.com/page/Hello%20World%20Foo").unwrap();
        assert!(
            out.contains("Hello%20World%20Foo") || out.contains("Hello World Foo"),
            "readable encoded path must survive: {out}"
        );
    }

    #[test]
    fn percent_encoded_short_word_is_preserved() {
        // `%64%6F%63%73` decodes to `docs` — short and single-class, so the
        // decoded form is not token-shaped and the sub-part survives.
        let out = guard("https://example.com/%64%6F%63%73/intro").unwrap();
        assert!(
            out.contains("%64%6F%63%73") || out.contains("docs"),
            "short encoded word must survive: {out}"
        );
    }

    #[test]
    fn percent_encoded_plus_base64_token_is_redacted() {
        // A standard-base64 token whose `+` and `=` are percent-encoded
        // (`%2B`, `%3D`). Decoding restores `+` (an opaque char) and trims the
        // `=` padding, so the token reads as a mixed-class opaque run and goes.
        let out = guard("https://app.com/s/AbC9%2BxK2mP4qR7sT0%3D").unwrap();
        assert!(
            !out.contains("AbC9") && !out.contains("xK2mP4qR7sT0"),
            "percent-encoded `+`/`=` base64 token must not leak: {out}"
        );
        assert_eq!(out, format!("app.com/s/{ARMED_TOKEN_PLACEHOLDER}"));
    }

    #[test]
    fn literal_percent_in_readable_content_is_preserved() {
        // A stray, undecodable `%` (here `%of` — `o` is not a hex digit) is left
        // literal by the lenient decoder, so the segment still carries a `%`
        // (non-opaque) and survives rather than being mistaken for a token.
        let out = guard("https://shop.example.com/sale/50%off-everything-today").unwrap();
        assert!(
            out.contains("50%off-everything-today"),
            "readable content with a stray `%` must survive: {out}"
        );
    }

    #[test]
    fn dictionary_word_after_plain_segment_is_preserved() {
        // Why we don't blanket-redact single-class `len >= 12` tokens: ordinary
        // English path words are all-lowercase and long, and must survive.
        let out = guard("https://docs.example.com/page/documentation").unwrap();
        assert!(
            out.contains("documentation"),
            "dictionary path word must survive (this is why the gate is mixed-class): {out}"
        );
        assert_eq!(out, "docs.example.com/page/documentation");
    }

    // --- INV-P2: the guard is linear in URL length with a bounded constant ---

    #[test]
    fn pathological_long_url_is_bounded_work_and_terminates() {
        // INV-P2 (no ReDoS / no super-linear blowup on a crafted long URL): the
        // guard partitions the path into NON-OVERLAPPING sub-parts on `/`, `%2F`,
        // `;`, `=` and runs the linear-time `regex`-crate redactor once per
        // sub-part, so total work is O(detectors x path length) — linear. The
        // `regex` crate is non-backtracking, so even a pathological 4 KB URL (many
        // segments + one long armed token) cannot trigger catastrophic
        // backtracking. This proves termination + bounded output on adversarial
        // shapes; the test would HANG (not just slow down) under any accidental
        // quadratic/exponential regression, so a normal test timeout fails it.

        // Shape 1: ~4 KB of many single-char segments (`/a/a/a/...`). Linear in
        // segment count; each `redact_text` call scans a tiny input.
        let many_segments = format!("https://site.com{}", "/a".repeat(2000));
        let out = guard(&many_segments).expect("many-segment url should guard");
        assert!(out.starts_with("site.com/"), "{}", &out[..40.min(out.len())]);

        // Shape 2: one 4 KB armed opaque token after a credential keyword. Must be
        // redacted by the armed pass, and the call must terminate quickly.
        let long_token = "Ab9".repeat(1400); // ~4.2 KB, mixed class, len >= 12
        let armed = format!("https://site.com/reset-password/{long_token}");
        let out = guard(&armed).expect("armed long-token url should guard");
        assert!(
            !out.contains(&long_token),
            "4 KB armed token must be redacted (and the guard must terminate)"
        );
        assert!(out.contains("reset-password"), "keyword stays visible: {out}");

        // Shape 3: one 4 KB opaque token with no armed predecessor. The PERF
        // invariant here is termination + bounded output (linear work), NOT
        // redaction coverage of an oversized token (a separate redaction-policy
        // concern). The call must return, and the guarded output must stay
        // O(input) — never blow up.
        let out = guard(&format!("https://site.com/s/{long_token}"))
            .expect("long-token url should guard");
        assert!(
            out.len() <= many_segments.len() + armed.len() + long_token.len() + 64,
            "guarded output is bounded by input size (no blowup): {} bytes",
            out.len()
        );
    }

    #[test]
    fn guarded_output_is_constant_bounded_regardless_of_url_length() {
        // The captured `browser_url` path is REMOTELY influenceable (a visited
        // page controls its own URL) and is re-guarded at the broker boundary on
        // every search hit / timeline interval. `MAX_GUARDED_PATH_BYTES` caps the
        // work + output: a 1 MB adversarial path produces the SAME bounded output
        // size as an 8 KB one, and the dropped overflow is redacted (no leak).
        let huge_path: String = std::iter::repeat('a').take(1_000_000).collect();
        let out = guard(&format!("https://site.com/{huge_path}")).unwrap();
        assert!(
            out.len() <= "site.com/".len() + MAX_GUARDED_PATH_BYTES + ARMED_TOKEN_PLACEHOLDER.len(),
            "guarded output must be constant-bounded by the path cap, got {} bytes",
            out.len()
        );
        assert!(
            out.contains(ARMED_TOKEN_PLACEHOLDER),
            "overflow past the cap must be redacted, not silently dropped: {}",
            &out[out.len().saturating_sub(40)..]
        );

        // A long single token straddling the cap must NOT leak its tail.
        let token: String = "Ab9".repeat(4_000); // ~12 KB, > cap
        let out = guard(&format!("https://site.com/s/{token}")).unwrap();
        assert!(!out.contains(&token), "oversized token must not leak whole");
        assert!(
            out.len() <= "site.com/".len() + MAX_GUARDED_PATH_BYTES + ARMED_TOKEN_PLACEHOLDER.len(),
            "oversized-token output stays bounded: {} bytes",
            out.len()
        );
    }

    // --- Fetch-target boundary (`secret_scrubbed_fetch_target`) ---

    #[test]
    fn fetch_target_drops_credential_named_param_keeps_others() {
        let out =
            secret_scrubbed_fetch_target("https://site.com/page?token=abcSECRET123&v=2").unwrap();
        assert!(!out.contains("token"), "credential-named param must drop: {out}");
        assert!(!out.contains("abcSECRET123"), "its value must not leak: {out}");
        assert!(out.contains("v=2"), "innocent param must survive: {out}");
    }

    #[test]
    fn fetch_target_drops_secret_valued_param() {
        // A known-shape GitHub token as a value with an innocent NAME (`x`) is
        // still dropped by the value detector.
        let out = secret_scrubbed_fetch_target(
            "https://site.com/page?x=ghp_1234567890abcdefABCDEF1234567890abcd&id=42",
        )
        .unwrap();
        assert!(
            !out.contains("ghp_1234567890abcdefABCDEF1234567890abcd"),
            "secret-valued param must drop: {out}"
        );
        assert!(out.contains("id=42"), "innocent param must survive: {out}");
    }

    #[test]
    fn fetch_target_drops_jwt_valued_param() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N";
        let out =
            secret_scrubbed_fetch_target(&format!("https://site.com/p?ref={jwt}&page=3")).unwrap();
        assert!(!out.contains(jwt), "JWT-valued param must drop: {out}");
        assert!(out.contains("page=3"), "innocent param must survive: {out}");
    }

    #[test]
    fn fetch_target_keeps_innocent_params_verbatim() {
        let out =
            secret_scrubbed_fetch_target("https://github.com/o/r/pulls?v=2&id=42&tab=files").unwrap();
        assert!(out.contains("v=2"), "{out}");
        assert!(out.contains("id=42"), "{out}");
        assert!(out.contains("tab=files"), "{out}");
        assert_eq!(out, "https://github.com/o/r/pulls?v=2&id=42&tab=files");
    }

    #[test]
    fn fetch_target_still_redacts_path_secret_fail_closed() {
        // A magic-link token in the PATH is redacted in the fetch target too, so
        // the origin sees a nonsense path and 404s — fail closed by design.
        let out =
            secret_scrubbed_fetch_target("https://app.com/reset-password/AbC9xK2mP4qR7sT0").unwrap();
        assert!(
            !out.contains("AbC9xK2mP4qR7sT0"),
            "path secret must be redacted in the fetch target: {out}"
        );
        assert!(out.contains("reset-password"), "keyword stays: {out}");
    }

    #[test]
    fn fetch_target_upgrades_http_to_https() {
        let out = secret_scrubbed_fetch_target("http://example.com/page?v=1").unwrap();
        assert!(out.starts_with("https://"), "scheme must be forced to https: {out}");
        assert_eq!(out, "https://example.com/page?v=1");
    }

    #[test]
    fn fetch_target_rejects_non_http_scheme() {
        assert_eq!(secret_scrubbed_fetch_target("file:///Users/me/secret.txt"), None);
        assert_eq!(secret_scrubbed_fetch_target("not a url"), None);
    }

    #[test]
    fn fetch_target_preserves_port_and_drops_fragment() {
        let out =
            secret_scrubbed_fetch_target("https://example.com:8443/dashboard?tab=x#frag").unwrap();
        assert_eq!(out, "https://example.com:8443/dashboard?tab=x");
    }

    #[test]
    fn fetch_target_strips_userinfo_credentials() {
        // A URL-embedded Basic-auth credential is a secret. The fetch target must
        // NOT carry it: the cookie-less client would turn `user:pass@` into an
        // `Authorization: Basic` header, leaking the credential to the origin and
        // riding the user's session. The model-text boundary (`guard_url`) already
        // drops userinfo; fail closed here too.
        let out = secret_scrubbed_fetch_target("https://user:hunter2@internal.example.com/dash?v=1")
            .unwrap();
        assert!(!out.contains("hunter2"), "password must not leak: {out}");
        assert!(!out.contains("user:"), "username must not leak: {out}");
        assert_eq!(out, "https://internal.example.com/dash?v=1");
    }
}

#[cfg(test)]
mod ssrf_review_security_a {
    use super::secret_scrubbed_fetch_target;

    // SSRF: the fetch target reaches the network and its body is streamed into a
    // (potentially CLOUD) model's context. A captured or attacker-redirected URL
    // whose host is loopback / private / link-local / metadata must NOT produce a
    // fetch target — otherwise internal-network / localhost service content is
    // exfiltrated to the cloud model. `secret_scrubbed_fetch_target` is the
    // fetch-target boundary; it must fail closed (None) on such hosts.
    #[test]
    fn fetch_target_refuses_private_and_loopback_hosts() {
        for raw in [
            "https://127.0.0.1/admin",
            "http://localhost:3000/admin",
            "https://[::1]:8443/",
            "https://192.168.1.1/router",
            "https://10.0.0.5/internal",
            "https://172.16.9.9/internal",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::ffff:169.254.169.254]/",
            "https://0.0.0.0/",
        ] {
            assert_eq!(
                secret_scrubbed_fetch_target(raw),
                None,
                "private/loopback host must not produce a fetch target: {raw}"
            );
        }
    }

    // A normal public host still produces a fetch target (no false-positive that
    // would break the feature outright).
    #[test]
    fn fetch_target_still_allows_public_hosts() {
        assert!(secret_scrubbed_fetch_target("https://github.com/o/r/pull/1?v=2").is_some());
        assert!(secret_scrubbed_fetch_target("https://example.com/page").is_some());
    }

    // The classifier is also applied on every redirect hop. Cover the hop path
    // (reqwest's `Attempt` is not constructible in a unit test) and the numeric
    // IPv4 encodings the `url` crate normalizes, so a decimal/hex-encoded loopback
    // redirect (`http://2130706433`) is screened too.
    #[test]
    fn classifier_blocks_private_hosts_including_numeric_encodings() {
        use super::is_disallowed_fetch_url;
        use url::Url;
        for raw in [
            "https://127.0.0.1/",
            "http://localhost/",
            "https://[::1]/",
            "https://10.1.2.3/",
            "https://169.254.169.254/",
            "https://2130706433/",   // decimal 127.0.0.1
            "https://0x7f000001/",   // hex 127.0.0.1
            "https://[::ffff:10.0.0.1]/",
        ] {
            let url = Url::parse(raw).unwrap();
            assert!(
                is_disallowed_fetch_url(&url),
                "must be screened as a disallowed fetch host: {raw}"
            );
        }
        for raw in ["https://github.com/", "https://8.8.8.8/", "https://example.com/"] {
            let url = Url::parse(raw).unwrap();
            assert!(
                !is_disallowed_fetch_url(&url),
                "public host must be allowed: {raw}"
            );
        }
    }

    // The connect-time (resolved-IP) screen is what closes the realistic exploit:
    // a public host NAME with a valid public cert that resolves to a private IP
    // (enterprise split-horizon DNS / DNS rebinding). The URL host is a domain, so
    // only the resolved-IP predicate can catch it. Assert the predicate directly.
    #[test]
    fn resolved_ip_predicate_blocks_private_and_metadata_addresses() {
        use super::ip_is_disallowed_fetch_target;
        use std::net::IpAddr;
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.9.9",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "::1",
            "fc00::1",         // unique-local
            "fe80::1",         // link-local
            "::ffff:10.0.0.1", // v4-mapped private
            "::7f00:1",        // deprecated v4-compatible loopback (127.0.0.1)
            "64:ff9b::7f00:1", // NAT64 loopback (127.0.0.1)
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(
                ip_is_disallowed_fetch_target(ip),
                "resolved private/metadata address must be refused: {ip}"
            );
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "140.82.121.3",
            "2606:4700:4700::1111",
            "64:ff9b::808:808", // NAT64 8.8.8.8 — public, stays allowed
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(
                !ip_is_disallowed_fetch_target(ip),
                "public address must be allowed: {ip}"
            );
        }
    }
}
