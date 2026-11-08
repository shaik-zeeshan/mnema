use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

/// A page served in a NON-UTF-8 charset it explicitly declares (windows-1252 /
/// iso-8859-1 is still common on legacy sites) is decoded with
/// `String::from_utf8_lossy`, which replaces every non-ASCII byte with U+FFFD.
/// The extracted content the (possibly cloud) model reads is corrupted — real
/// characters are silently dropped — yet `truncated` stays false, so the model
/// answers as if it saw the complete page. Falsifies INV-A3 ("real content
/// dropped but truncated:false").
#[test]
fn declared_charset_body_is_not_lossily_corrupted() {
    // "café résumé" encoded in windows-1252/latin-1: é is the single byte 0xE9,
    // which is invalid UTF-8 on its own, so `from_utf8_lossy` maps it to U+FFFD.
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(b"<html><body><p>caf");
    body.push(0xE9);
    body.extend_from_slice(b" r");
    body.push(0xE9);
    body.extend_from_slice(b"sum");
    body.push(0xE9);
    body.extend_from_slice(b"</p></body></html>");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=windows-1252\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(header.as_bytes());
        let _ = sock.write_all(&body);
        let _ = sock.flush();
    });

    let target = format!("http://{addr}/");
    let page = crate::test_support::block_on(fetch_page(&target)).expect("fetch ok");
    let _ = server.join();

    assert_eq!(page.status, 200);
    assert!(
        !page.truncated,
        "small complete page must not report truncated"
    );
    assert!(
        !page.content.contains('\u{FFFD}'),
        "declared-charset body must not be lossily corrupted into U+FFFD: {}",
        page.content
    );
    assert!(
        page.content.contains("café") && page.content.contains("résumé"),
        "windows-1252 bytes must decode to their real characters, not be dropped: {}",
        page.content
    );
}
