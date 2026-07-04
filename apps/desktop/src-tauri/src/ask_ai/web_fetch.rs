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
    guard_url, secret_scrubbed_fetch_target, signed_opaque_capture_reference,
    BrokerOpaqueCaptureReference,
};
use futures_util::StreamExt;
use htmd::HtmlToMarkdown;

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
            .build()
            // A build failure is a programmer error (rustls is always compiled
            // in); fall back to the default rather than panicking the turn.
            .unwrap_or_default()
    })
}

/// Redirect policy: refuse any hop whose scheme is not https, and cap the chain
/// at [`MAX_REDIRECTS`]. A redirect to http (or any non-https scheme) aborts the
/// fetch rather than downgrading the transport.
fn https_only_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(std::io::Error::other(
                "fetch_url exceeded the redirect limit",
            ));
        }
        if attempt.url().scheme() != "https" {
            return attempt.error(std::io::Error::other(
                "fetch_url refuses a non-https redirect hop",
            ));
        }
        attempt.follow()
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

/// Stream the response body into a buffer, stopping at `cap` bytes.
async fn read_capped_body(response: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("failed to read the page body: {error}"))?;
        buffer.extend_from_slice(&chunk);
        if buffer.len() >= cap {
            buffer.truncate(cap);
            break;
        }
    }
    Ok(buffer)
}

/// GET `target_url`, gate on content type, stream the body under the cap, and
/// extract text. Never sees the opaque id or the raw stored URL — only the
/// already-scrubbed fetch target.
async fn fetch_page(target_url: &str) -> Result<FetchedPage, String> {
    let response = fetch_client()
        .get(target_url)
        .send()
        .await
        .map_err(|error| format!("failed to fetch the page: {error}"))?;

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

    let body = read_capped_body(response, MAX_BODY_BYTES).await?;
    let text = String::from_utf8_lossy(&body);

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
        truncated,
    })
}

/// The `fetch_url` tool description handed to the model.
pub(crate) fn web_fetch_tool() -> ai_engine::AgentTool {
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
pub(crate) async fn execute_web_fetch(
    app_handle: &tauri::AppHandle,
    params: serde_json::Value,
) -> Result<String, String> {
    // Defense in depth: the tool is only built when enabled, but re-check so a
    // stale toolset can never fetch after the user turned it off.
    if !super::read_ask_ai_web_fetch_enabled(app_handle) {
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
    let config_dir = super::access_config_dir(app_handle)?;
    let reference = signed_opaque_capture_reference(&config_dir, &opaque_id)
        .map_err(|error| format!("failed to resolve the capture: {error}"))?;
    let frame_id = frame_id_for_fetch(reference.as_ref())?;

    let infra = super::app_infra(app_handle)?;
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
mod tests {
    use super::*;

    #[test]
    fn content_type_gate_accepts_text_kinds() {
        assert_eq!(content_type_kind("text/html"), Some(BodyKind::Html));
        assert_eq!(
            content_type_kind("text/html; charset=utf-8"),
            Some(BodyKind::Html)
        );
        assert_eq!(
            content_type_kind("application/xhtml+xml"),
            Some(BodyKind::Html)
        );
        assert_eq!(content_type_kind("text/plain"), Some(BodyKind::Text));
        assert_eq!(
            content_type_kind("application/json; charset=utf-8"),
            Some(BodyKind::Text)
        );
    }

    #[test]
    fn content_type_gate_rejects_disallowed_types() {
        assert_eq!(content_type_kind("image/png"), None);
        assert_eq!(content_type_kind("application/pdf"), None);
        assert_eq!(content_type_kind("application/octet-stream"), None);
        assert_eq!(content_type_kind("video/mp4"), None);
        assert_eq!(content_type_kind(""), None);
    }

    #[test]
    fn cap_text_truncates_and_flags() {
        let (out, truncated) = cap_text("abcdef", 3);
        assert_eq!(out, "abc");
        assert!(truncated);

        let (out, truncated) = cap_text("abc", 10);
        assert_eq!(out, "abc");
        assert!(!truncated);
    }

    #[test]
    fn html_to_markdown_truncates_long_document_and_sets_flag() {
        // A body that yields far more than MAX_MARKDOWN_CHARS of Markdown.
        let long_paragraph = "word ".repeat(20_000);
        let html = format!("<html><body><p>{long_paragraph}</p></body></html>");
        let (content, truncated) = html_to_markdown(&html);
        assert!(truncated, "over-long markdown must set truncated");
        assert_eq!(
            content.chars().count(),
            MAX_MARKDOWN_CHARS,
            "content is capped at the char limit"
        );
    }

    #[test]
    fn html_to_markdown_short_document_is_not_truncated() {
        let (content, truncated) =
            html_to_markdown("<html><body><h1>Title</h1><p>Hello world</p></body></html>");
        assert!(!truncated);
        assert!(content.contains("Hello world"), "content: {content}");
    }

    #[test]
    fn html_to_markdown_prunes_script_and_style() {
        let (content, _) = html_to_markdown(
            "<html><head><style>.a{color:red}</style></head><body>\
<script>alert('x')</script><p>Visible</p></body></html>",
        );
        assert!(content.contains("Visible"), "content: {content}");
        assert!(!content.contains("alert"), "script must be pruned: {content}");
        assert!(!content.contains("color:red"), "style must be pruned: {content}");
    }

    #[test]
    fn extract_title_reads_title_tag() {
        assert_eq!(
            extract_title("<html><head><title>My Page</title></head><body>x</body></html>")
                .as_deref(),
            Some("My Page")
        );
        // Attributes on the tag are handled.
        assert_eq!(
            extract_title("<title lang=\"en\">  Spaced  </title>").as_deref(),
            Some("Spaced")
        );
        assert_eq!(extract_title("<html><body>no title</body></html>"), None);
    }

    #[test]
    fn frame_id_for_fetch_errors_on_missing_reference() {
        let error = frame_id_for_fetch(None).unwrap_err();
        assert_eq!(error, NO_CAPTURE_ERROR);
    }

    #[test]
    fn frame_id_for_fetch_errors_on_audio_or_no_frame() {
        // An audio capture (no frame_id) yields the readable no-URL error.
        let audio = BrokerOpaqueCaptureReference {
            opaque_id: "op-1".to_string(),
            kind: "audio".to_string(),
            frame_id: None,
            audio_segment_id: Some(7),
            grant_id: Some("g-1".to_string()),
        };
        let error = frame_id_for_fetch(Some(&audio)).unwrap_err();
        assert_eq!(error, NO_URL_ERROR);
    }

    #[test]
    fn frame_id_for_fetch_returns_frame_id_for_a_frame_capture() {
        let frame = BrokerOpaqueCaptureReference {
            opaque_id: "op-2".to_string(),
            kind: "frame".to_string(),
            frame_id: Some(42),
            audio_segment_id: None,
            grant_id: Some("g-1".to_string()),
        };
        assert_eq!(frame_id_for_fetch(Some(&frame)).unwrap(), 42);
    }

    #[test]
    fn web_fetch_tool_shape() {
        let tool = web_fetch_tool();
        assert_eq!(tool.name, "fetch_url");
        assert!(!tool.description.trim().is_empty());
        assert_eq!(tool.parameters_schema["type"], serde_json::json!("object"));
        assert_eq!(
            tool.parameters_schema["required"],
            serde_json::json!(["opaqueId"])
        );
    }
}
