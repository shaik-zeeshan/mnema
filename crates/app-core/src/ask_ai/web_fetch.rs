//! `fetch_url` — the opt-in Ask AI tool that re-fetches the CURRENT content of a
//! web page the user actually visited (Workstream B).
//!
//! The model NEVER supplies a URL. It passes an opaque capture id (from a
//! `search` result's `context.url`); we resolve that id to the frame the user
//! captured, read its recorded `browser_url`, and fetch it. Two boundaries keep
//! secrets contained (grill G6):
//!
//!   - **Fetch target** (goes to the origin, which already knows its own URL):
//!     [`secret_scrubbed_fetch_target`] — https forced, credential query params
//!     dropped, path secrets redacted (a redacted path 404s → fail closed).
//!   - **Model-facing text** (`url`/`finalUrl`): [`guard_url`] — query stripped,
//!     path secrets redacted — so the raw URL/query never reaches a cloud model.
//!
//! The HTTP client is cookie-less (no CSRF primitive), https-only on every hop,
//! caps redirects, caps the body at 2 MB, and gates on a text-ish content type.
//! HTML is converted to Markdown (via `htmd`) and capped at ~24k chars.

use std::sync::OnceLock;
use std::time::Duration;

use app_infra::brokered_access::{
    guard_url, is_disallowed_fetch_url, secret_scrubbed_fetch_target,
    signed_opaque_capture_reference, BrokerOpaqueCaptureReference,
};
use futures_util::StreamExt;
use htmd::HtmlToMarkdown;

use crate::AppCore;

/// Hard cap on the response body we buffer (2 MiB). A page past this is
/// truncated at the cut — enough to answer "what's the current state" without
/// letting a hostile origin stream unbounded bytes into the turn.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Cap on the extracted Markdown / text handed to the model (~24k chars). Keeps
/// one fetched page from flooding the prompt budget.
const MAX_MARKDOWN_CHARS: usize = 24_000;

/// Maximum redirect hops. Each hop must ALSO be https (enforced in the policy).
const MAX_REDIRECTS: usize = 5;

/// Whole-request timeout, mirroring the semantic-search download client's bound.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Explicit, honest User-Agent so origins can identify + rate-limit the fetch.
const USER_AGENT: &str = "Mnema-AskAI-fetch/1.0";

/// The capture id resolved to something with no fetchable web page.
const NO_CAPTURE_ERROR: &str =
    "couldn't resolve that capture id — it may be stale or out of scope; run `search` again to \
get a fresh result whose context carries a url";

/// The capture exists but is audio or has no recorded URL.
const NO_URL_ERROR: &str =
    "that capture has no web page to fetch (it's audio or has no recorded URL); use `search` to \
find a page that has a url first";

/// Which extraction path a response body takes, decided by its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    /// HTML → prune + convert to Markdown.
    Html,
    /// Plain text / JSON → pass through (still capped).
    Text,
}

/// Pure content-type gate: map a raw `Content-Type` header value to the body
/// kind we accept, or `None` for anything else (images, PDFs, binaries…). The
/// charset/parameter suffix (`; charset=utf-8`) is stripped before matching.
fn content_type_kind(content_type: &str) -> Option<BodyKind> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "text/html" | "application/xhtml+xml" => Some(BodyKind::Html),
        "text/plain" | "application/json" => Some(BodyKind::Text),
        _ => None,
    }
}

/// Decode the raw response body using the charset declared in its `Content-Type`
/// header, defaulting to UTF-8 when the label is absent or unrecognized. We read
/// the body as raw bytes (to enforce the 2 MB cap), so — unlike reqwest's
/// `.text()` — no charset decoding happens for free: a legacy page served as
/// `windows-1252` / `shift_jis` / `iso-8859-1` would otherwise be mangled into
/// U+FFFD by a bare `from_utf8_lossy`, silently corrupting the content the model
/// reads while `truncated` stays false. `encoding_rs::decode` also strips a
/// leading BOM and substitutes on genuinely undecodable bytes.
fn decode_body(body: &[u8], content_type: Option<&str>) -> String {
    let label = content_type.and_then(|ct| {
        ct.split(';').skip(1).find_map(|param| {
            let param = param.trim();
            let rest = param.strip_prefix("charset")?.trim_start();
            let value = rest.strip_prefix('=')?.trim();
            Some(value.trim_matches('"').to_string())
        })
    });
    let encoding = label
        .as_deref()
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// Cap `text` at `max_chars` on a char boundary. Returns `(text, truncated)`.
fn cap_text(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        (text.to_string(), false)
    } else {
        (text.chars().take(max_chars).collect(), true)
    }
}

/// Convert one HTML document to Markdown, pruning non-content chrome, and cap the
/// result. Returns `(markdown, truncated)`. The single HTML→Markdown seam.
fn html_to_markdown(html: &str) -> (String, bool) {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec![
            "script", "style", "noscript", "nav", "header", "footer", "svg",
        ])
        .build();
    // A conversion error (malformed HTML) yields empty content rather than
    // aborting the whole fetch — the status + url are still useful.
    let markdown = converter.convert(html).unwrap_or_default();
    cap_text(markdown.trim(), MAX_MARKDOWN_CHARS)
}

/// Best-effort `<title>` extraction from raw HTML (htmd drops the head). Byte
/// indices from the ascii-lowercased copy align with the original (ASCII case
/// folding is length-preserving), so slicing `html` stays UTF-8 valid.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let content_start = lower[open..].find('>')? + open + 1;
    let close = lower[content_start..].find("</title>")? + content_start;
    let title = html[content_start..close].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// Resolve the reference into a frame id, or a READABLE error the model relays.
/// Pure (no IO) so the audio / stale-id branches are unit-testable.
fn frame_id_for_fetch(reference: Option<&BrokerOpaqueCaptureReference>) -> Result<i64, String> {
    match reference {
        None => Err(NO_CAPTURE_ERROR.to_string()),
        Some(reference) => reference.frame_id.ok_or_else(|| NO_URL_ERROR.to_string()),
    }
}

/// Shared cookie-less HTTP client (rustls, explicit UA, 15s timeout, https-only
/// redirects capped at [`MAX_REDIRECTS`]). Cookie-less by construction: the
/// `cookies` reqwest feature is not compiled, so no cookie jar exists (no CSRF
/// primitive — a fetch can never ride the user's authenticated session).
fn fetch_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(https_only_redirect_policy())
            // SSRF: screen the RESOLVED connection address of every host — the
            // initial target AND each redirect hop. This is the real boundary:
            // it catches a public host NAME that resolves to a private/loopback/
            // metadata IP (enterprise split-horizon DNS / DNS rebinding), which
            // the URL-host checks cannot see, so internal-network content can
            // never be fetched and streamed into the (possibly cloud) model.
            .dns_resolver(std::sync::Arc::new(PrivateIpBlockingResolver))
            .build()
            // A build failure is a programmer error (rustls is always compiled
            // in); fall back to the default rather than panicking the turn.
            .unwrap_or_default()
    })
}

/// Custom DNS resolver that refuses to resolve a host to any private, loopback,
/// link-local, or metadata address. reqwest consults it for every connection —
/// the initial fetch and each redirect hop — so it screens IP-literal hosts,
/// hostnames that resolve to a private IP, and redirect targets in one place.
/// Rejects the whole resolution if ANY returned address is disallowed (a mixed
/// public/private DNS answer cannot smuggle a private connect).
struct PrivateIpBlockingResolver;

impl reqwest::dns::Resolve for PrivateIpBlockingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            // Port 0: reqwest overrides the port after resolution; we only need
            // the addresses to screen. Do the DNS lookup ourselves (reqwest's
            // default resolver is not publicly constructible).
            let addrs: Vec<std::net::SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs
                .iter()
                .any(|addr| is_disallowed_fetch_target_ip(addr.ip()))
            {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "fetch_url refuses a host that resolves to a private or loopback address",
                ));
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Local alias so the resolver reads cleanly; the policy lives in `app-infra`.
fn is_disallowed_fetch_target_ip(ip: std::net::IpAddr) -> bool {
    app_infra::brokered_access::ip_is_disallowed_fetch_target(ip)
}

/// Pure screen for ONE redirect hop: refuse a hop past [`MAX_REDIRECTS`], any
/// non-https target (an https→http downgrade included), and a private /
/// loopback / link-local / metadata host. Extracted from the redirect policy
/// closure so the decision is unit-testable (`reqwest::redirect::Attempt`
/// cannot be constructed in tests). `prior_hops` is the redirect chain length
/// BEFORE this hop.
fn screen_redirect_hop(next: &reqwest::Url, prior_hops: usize) -> Result<(), &'static str> {
    if prior_hops > MAX_REDIRECTS {
        return Err("fetch_url exceeded the redirect limit");
    }
    if next.scheme() != "https" {
        return Err("fetch_url refuses a non-https redirect hop");
    }
    // SSRF: the origin controls `Location`, so a benign captured URL can be
    // redirected into internal infrastructure. Re-screen every hop's host —
    // a redirect to a loopback / private / link-local / metadata address is
    // refused, matching the initial-target screen in
    // `secret_scrubbed_fetch_target`.
    if is_disallowed_fetch_url(next) {
        return Err("fetch_url refuses a redirect to a private or loopback host");
    }
    Ok(())
}

/// Redirect policy: refuse any hop whose scheme is not https, and cap the chain
/// at [`MAX_REDIRECTS`]. A redirect to http (or any non-https scheme) aborts the
/// fetch rather than downgrading the transport. Thin adapter over
/// [`screen_redirect_hop`].
fn https_only_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        match screen_redirect_hop(attempt.url(), attempt.previous().len()) {
            Err(reason) => attempt.error(std::io::Error::other(reason)),
            Ok(()) => attempt.follow(),
        }
    })
}

/// One fetched, extracted page. `final_url_raw` is the post-redirect URL BEFORE
/// guarding; the caller guards it for the model.
struct FetchedPage {
    status: u16,
    final_url_raw: String,
    title: Option<String>,
    content: String,
    truncated: bool,
}

/// Stream the response body into a buffer, stopping at `cap` bytes. Returns
/// `(bytes, capped)` where `capped` is true only when the body actually exceeded
/// `cap` and real bytes were dropped — an exactly-`cap` body is NOT flagged. The
/// caller ORs `capped` into the model-facing `truncated` flag so a body cut is
/// never reported as a complete page.
async fn read_capped_body(
    response: reqwest::Response,
    cap: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut capped = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| format!("failed to read the page body: {}", error.without_url()))?;
        buffer.extend_from_slice(&chunk);
        if buffer.len() > cap {
            buffer.truncate(cap);
            capped = true;
            break;
        }
    }
    Ok((buffer, capped))
}

/// GET `target_url`, gate on content type, stream the body under the cap, and
/// extract text. Never sees the opaque id or the raw stored URL — only the
/// already-scrubbed fetch target.
async fn fetch_page(target_url: &str) -> Result<FetchedPage, String> {
    let response = fetch_client()
        .get(target_url)
        .send()
        .await
        // `without_url()` strips the URL reqwest embeds in its Display — the raw
        // fetch-target (query included) must never cross the model boundary; the
        // model only ever sees the query-stripped `guard_url` form.
        .map_err(|error| format!("failed to fetch the page: {}", error.without_url()))?;

    let status = response.status().as_u16();
    let final_url_raw = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let kind = content_type
        .as_deref()
        .and_then(content_type_kind)
        .ok_or_else(|| {
            format!(
                "that page isn't fetchable text (content type: {}); fetch_url only reads html, \
plain text, or json",
                content_type.as_deref().unwrap_or("unknown")
            )
        })?;

    let (body, body_capped) = read_capped_body(response, MAX_BODY_BYTES).await?;
    // Decode by the declared charset, not a bare UTF-8 assumption — a legacy
    // non-UTF-8 page must not be silently corrupted into replacement chars.
    let text = decode_body(&body, content_type.as_deref());

    let (title, content, truncated) = match kind {
        BodyKind::Html => {
            let title = extract_title(&text);
            let (content, truncated) = html_to_markdown(&text);
            (title, content, truncated)
        }
        BodyKind::Text => {
            let (content, truncated) = cap_text(text.trim(), MAX_MARKDOWN_CHARS);
            (None, content, truncated)
        }
    };

    Ok(FetchedPage {
        status,
        final_url_raw,
        title,
        content,
        // A body cut at MAX_BODY_BYTES drops real bytes even when the extracted
        // markdown fits under the 24k cap (e.g. a huge skipped <script> before the
        // real content). Report that as truncated so the model never treats a
        // body-capped page as complete.
        truncated: truncated || body_capped,
    })
}

/// The `fetch_url` tool description handed to the model.
pub fn web_fetch_tool() -> ai_engine::AgentTool {
    ai_engine::AgentTool {
        name: "fetch_url".to_string(),
        description:
            "Re-fetch the CURRENT content of a web page the user actually visited, keyed by the \
opaque id of a `search` result whose `context.url` is set (you NEVER supply a URL yourself). Use \
this when the answer needs the page's live state — a PR's status now, a ticket's current state, a \
live article — not its capture-time snapshot. The id comes from a `search` result's `context.url`; \
to fetch a page you only saw in the `timeline`, `search` for it first (timeline intervals carry no \
opaque ids). Audio or no-URL captures return a readable error. Returns `{ url, finalUrl, status, \
title, content, truncated }` with the page as Markdown; the URLs are shown redacted."
                .to_string(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "opaqueId": {
                    "type": "string",
                    "description": "The opaque id of a `search` result whose context carried a url."
                }
            },
            "required": ["opaqueId"]
        }),
    }
}

/// Execute `fetch_url`: re-check the setting (defense in depth), resolve the
/// opaque id → stored URL, scrub it into a fetch target, fetch + extract, and
/// return the model-facing JSON string with BOTH URLs guarded.
pub async fn execute_web_fetch(
    core: &AppCore,
    params: serde_json::Value,
) -> Result<String, String> {
    // Defense in depth: the tool is only built when enabled, but re-check so a
    // stale toolset can never fetch after the user turned it off.
    if !super::read_ask_ai_web_fetch_enabled(core) {
        return Err("web fetch is disabled".to_string());
    }

    let opaque_id = params
        .get("opaqueId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "fetch_url requires a non-empty opaqueId".to_string())?
        .to_string();

    // Resolve the opaque id → frame → recorded browser URL. Uses the SAME
    // signature-only resolver as `build_ask_ai_sources` (ask_ai.rs): Ask AI
    // opaque ids are minted under an ephemeral in-memory grant that is never
    // persisted, so the grant-checked `authorize_active_opaque_capture_reference`
    // would reject every one of them. The HMAC signature is the trust boundary —
    // the model can only pass ids it actually received from a `search` result.
    let config_dir = super::access_config_dir(core)?;
    let reference = signed_opaque_capture_reference(&config_dir, &opaque_id)
        .map_err(|error| format!("failed to resolve the capture: {error}"))?;
    let frame_id = frame_id_for_fetch(reference.as_ref())?;

    let infra = super::app_infra(core)?;
    let frame = infra
        .get_frame(frame_id)
        .await
        .map_err(|error| format!("failed to load the capture: {error}"))?
        .ok_or_else(|| NO_CAPTURE_ERROR.to_string())?;
    let stored_url = frame
        .metadata_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.browser_url.clone())
        .ok_or_else(|| NO_URL_ERROR.to_string())?;

    // Two boundaries: the origin gets the secret-scrubbed target; the model gets
    // the guarded (query-stripped, path-redacted) form and never the raw URL.
    let target_url = secret_scrubbed_fetch_target(&stored_url)
        .ok_or_else(|| "that page's address can't be safely fetched".to_string())?;
    let model_url = guard_url(&stored_url).unwrap_or_default();

    let page = fetch_page(&target_url).await?;
    let final_model_url = guard_url(&page.final_url_raw).unwrap_or_default();

    let result = serde_json::json!({
        "url": model_url,
        "finalUrl": final_model_url,
        "status": page.status,
        "title": page.title,
        "content": page.content,
        "truncated": page.truncated,
    });
    serde_json::to_string(&result)
        .map_err(|error| format!("failed to serialize fetch result: {error}"))
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "web_fetch_tests_body_cap.rs"]
mod web_fetch_body_cap_review_dataint_a;

#[cfg(test)]
#[path = "web_fetch_tests_ssrf.rs"]
mod ssrf_resolver_review_security_a;

#[cfg(test)]
#[path = "web_fetch_tests_charset.rs"]
mod web_fetch_charset_review_dataint_a;

#[cfg(test)]
#[path = "web_fetch_tests_redirect.rs"]
mod web_fetch_redirect_screen;
