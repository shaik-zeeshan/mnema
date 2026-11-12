//! The HTTP fetch the OCR, transcription and speaker model downloads share: a
//! client with connect and read timeouts, and a chunk read that Cancel can
//! interrupt while the body is stalled.
//!
//! A bare `reqwest::get` has no timeouts at all (reqwest `ClientBuilder::timeout`
//! / `read_timeout`: "Default is no timeout"), and a per-chunk cancel check is
//! never reached when no chunk arrives, so a wedged CDN held the download — and
//! its `downloading` model lock — until relaunch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{Stream, StreamExt};

/// Longest wait for a TCP/TLS connection to the model host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest gap between body reads (reqwest re-arms it after every read), so a
/// slow-but-live multi-GB transfer is never aborted, only a stalled one. Same
/// bound as the semantic-search downloader.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How often a stalled read looks at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(200);

/// `GET url` with connect and read timeouts; a non-2xx status is an error.
pub(crate) async fn get(url: &str) -> reqwest::Result<reqwest::Response> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_IDLE_TIMEOUT)
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()
}

/// What [`next_chunk`] got.
pub(crate) enum NextChunk<T> {
    Chunk(reqwest::Result<T>),
    End,
    Cancelled,
}

/// The next body chunk, or `Cancelled` as soon as `cancel` is set — even while
/// no bytes are arriving.
pub(crate) async fn next_chunk<S, T>(stream: &mut S, cancel: &AtomicBool) -> NextChunk<T>
where
    S: Stream<Item = reqwest::Result<T>> + Unpin,
{
    let cancelled = async {
        while !cancel.load(Ordering::SeqCst) {
            tokio::time::sleep(CANCEL_POLL).await;
        }
    };
    tokio::select! {
        biased;
        () = cancelled => NextChunk::Cancelled,
        chunk = stream.next() => match chunk {
            Some(chunk) => NextChunk::Chunk(chunk),
            None => NextChunk::End,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::io::AsyncWriteExt;

    /// A server that sends headers promising a body, then goes silent.
    async fn stalled_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\npartial")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(120)).await;
            drop(socket);
        });
        (url, server)
    }

    #[tokio::test]
    async fn cancel_interrupts_a_stalled_body() {
        let (url, server) = stalled_server().await;
        let mut stream = get(&url).await.expect("headers arrive").bytes_stream();
        let cancel = Arc::new(AtomicBool::new(false));

        assert!(matches!(
            next_chunk(&mut stream, &cancel).await,
            NextChunk::Chunk(Ok(_))
        ));

        let setter = Arc::clone(&cancel);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            setter.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        assert!(matches!(
            next_chunk(&mut stream, &cancel).await,
            NextChunk::Cancelled
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        server.abort();
    }
}
