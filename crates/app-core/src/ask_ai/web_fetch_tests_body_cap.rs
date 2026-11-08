use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

/// A response body larger than `MAX_BODY_BYTES` is silently cut in
/// `read_capped_body`. When the extracted markdown still fits under the 24k
/// cap (a huge skipped `<script>` before the real content is the common SPA
/// case), the model-facing `truncated` flag MUST still be true — otherwise
/// the model treats a body-capped page as the complete page and answers from
/// content it never saw.
#[test]
fn body_over_cap_sets_truncated_even_when_markdown_fits() {
    // Tiny visible content BEFORE a >2 MiB skipped <script>. The markdown is
    // just "VISIBLESENTINEL" (well under 24k), but the raw body far exceeds
    // MAX_BODY_BYTES, so the 2 MiB cut drops real bytes.
    let filler = "A".repeat(MAX_BODY_BYTES + 4096);
    let html = format!("<html><body><p>VISIBLESENTINEL</p><script>{filler}</script></body></html>");
    let body = html.into_bytes();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf); // drain request headers; partial is fine
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(header.as_bytes());
        // The client caps its read at MAX_BODY_BYTES and drops the connection,
        // so the tail write races a broken pipe — expected, ignore it.
        let _ = sock.write_all(&body);
        let _ = sock.flush();
    });

    let target = format!("http://{addr}/");
    let page = crate::test_support::block_on(fetch_page(&target)).expect("fetch ok");
    let _ = server.join();

    assert_eq!(page.status, 200);
    assert!(
        page.content.contains("VISIBLESENTINEL"),
        "pre-cut content should still extract: {}",
        page.content
    );
    assert!(
        page.content.chars().count() <= MAX_MARKDOWN_CHARS,
        "markdown fits under the 24k cap, so the 24k cap is not what truncated it"
    );
    assert!(
        page.truncated,
        "body exceeded MAX_BODY_BYTES and was cut; truncated must be true"
    );
}
