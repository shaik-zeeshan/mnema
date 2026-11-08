use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

/// End-to-end wiring check that [`screen_redirect_hop`] actually gates the
/// shared client: a canned 302 whose `Location` is an http URL must abort
/// the fetch. The policy runs BEFORE any connection to the redirect target,
/// so this is fully offline; the specific refusal reasons are pinned by the
/// pure `screen_redirect_hop_*` tests (reqwest's error Display does not
/// chain the policy's message — only its "error following redirect" kind).
#[test]
fn redirect_to_http_aborts_the_fetch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    // Points back at this listener, but the scheme alone refuses the hop —
    // no second connection is ever attempted (the accept below is the only
    // one served, so a followed redirect could not succeed either).
    let location = format!("http://{addr}/next");
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf); // drain request headers; partial is fine
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = sock.write_all(response.as_bytes());
        let _ = sock.flush();
    });

    let target = format!("http://{addr}/");
    let err = match crate::test_support::block_on(fetch_page(&target)) {
        Ok(page) => panic!(
            "a redirect to http must abort the fetch, got status {}",
            page.status
        ),
        Err(err) => err,
    };
    let _ = server.join();
    assert!(
        err.contains("error following redirect"),
        "the abort must come from the redirect policy, not a connect failure: {err}"
    );
}
