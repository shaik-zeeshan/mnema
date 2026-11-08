use super::*;

#[test]
fn fetch_error_does_not_leak_target_url_to_model() {
    // The two-boundary rule: the model may only ever see the query-stripped
    // `guard_url` form. A failed fetch stringifies the reqwest error, whose
    // Display appends ` for url (<target>)` — the raw fetch-target incl. the
    // non-credential query params `guard_url` hides. `without_url()` strips it.
    // A refused loopback port fails immediately and offline.
    let target = "https://127.0.0.1:9/inbox?email=alice@corp.com&team=secret-project";
    let err = match crate::test_support::block_on(fetch_page(target)) {
        Ok(_) => panic!("a connect to a refused port must fail"),
        Err(err) => err,
    };
    assert!(
        !err.contains("secret-project") && !err.contains("team") && !err.contains("email"),
        "fetch error leaked query params to the model: {err}"
    );
    assert!(
        !err.contains("127.0.0.1"),
        "fetch error leaked the target host/URL to the model: {err}"
    );
}

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
    assert!(
        !content.contains("alert"),
        "script must be pruned: {content}"
    );
    assert!(
        !content.contains("color:red"),
        "style must be pruned: {content}"
    );
}

#[test]
fn extract_title_reads_title_tag() {
    assert_eq!(
        extract_title("<html><head><title>My Page</title></head><body>x</body></html>").as_deref(),
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
fn screen_redirect_hop_refuses_https_to_http_downgrade() {
    // The origin controls `Location`; a downgrade to http would move the
    // rest of the fetch off TLS. Any non-https hop is refused.
    let http = reqwest::Url::parse("http://example.com/next").expect("parse");
    assert_eq!(
        screen_redirect_hop(&http, 1),
        Err("fetch_url refuses a non-https redirect hop")
    );
}

#[test]
fn screen_redirect_hop_refuses_private_and_loopback_hosts() {
    for target in [
        "https://127.0.0.1/internal",
        "https://[::1]/internal",
        "https://10.0.0.8/internal",
        "https://192.168.1.1/router",
        "https://169.254.169.254/latest/meta-data/",
        // CGNAT (100.64.0.0/10), broadcast, and the RFC 6761 loopback names.
        "https://100.64.0.1/",
        "https://100.127.255.255/",
        "https://255.255.255.255/",
        "https://localhost/",
        "https://app.localhost/",
    ] {
        let url = reqwest::Url::parse(target).expect("parse");
        assert_eq!(
            screen_redirect_hop(&url, 1),
            Err("fetch_url refuses a redirect to a private or loopback host"),
            "{target} must be refused"
        );
    }
}

#[test]
fn screen_redirect_hop_allows_public_https_and_caps_hops() {
    // Host-name screening only (no DNS), so a public domain is offline-safe.
    let url = reqwest::Url::parse("https://example.com/page").expect("parse");
    assert_eq!(screen_redirect_hop(&url, MAX_REDIRECTS), Ok(()));
    assert_eq!(
        screen_redirect_hop(&url, MAX_REDIRECTS + 1),
        Err("fetch_url exceeded the redirect limit")
    );
}

#[test]
fn disallowed_fetch_target_ip_covers_cgnat_and_broadcast() {
    // The resolver-side screen shares `ipv4_is_disallowed` with the URL
    // screen; pin the CGNAT and broadcast branches directly.
    for ip in ["100.64.0.1", "100.127.255.255", "255.255.255.255"] {
        let ip: std::net::IpAddr = ip.parse().expect("parse ip");
        assert!(is_disallowed_fetch_target_ip(ip), "{ip} must be refused");
    }
    // The CGNAT block is exactly /10 — its neighbors stay reachable.
    for ip in ["100.63.255.255", "100.128.0.0", "8.8.8.8"] {
        let ip: std::net::IpAddr = ip.parse().expect("parse ip");
        assert!(!is_disallowed_fetch_target_ip(ip), "{ip} must be allowed");
    }
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
