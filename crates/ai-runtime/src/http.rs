//! The HTTP client every rig provider client is built on.
//!
//! rig's plain `build()` falls back to `reqwest::Client::default()`, which has
//! no timeouts at all (reqwest `ClientBuilder::timeout`/`read_timeout`: "Default
//! is no timeout"), so a provider that accepts the connection and then goes
//! silent hangs the call forever — and with it the User Context worker tick and
//! the forward-derivation lock it holds.

use std::time::Duration;

use rig_core::http_client::{Error, ReqwestClient};

/// Longest wait for a TCP/TLS connection to a provider.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest silence tolerated on a request. reqwest arms this from send until
/// the response headers and then re-arms it per body read, so for a
/// non-streamed completion it bounds the whole generation (headers only arrive
/// once the model is done) and for a streamed one the gap between chunks. Ten
/// minutes matches the Anthropic and OpenAI SDKs' default request timeout and
/// leaves room for a local Ollama cold-loading a large model.
const READ_TIMEOUT: Duration = Duration::from_secs(600);

/// A reqwest client with connect and read timeouts, for `ClientBuilder::http_client`.
pub(crate) fn client() -> Result<ReqwestClient, Error> {
    ReqwestClient::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|error| Error::Instance(Box::new(error)))
}
