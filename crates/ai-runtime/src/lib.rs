//! Provider-agnostic AI "Reasoning Engine" built on `rig-core`.
//!
//! The engine runs entirely inside the Rust process: no Node, no shim, no
//! bundled JS runtime. A caller selects a [`EngineConfig`] — either a cloud
//! provider reached over HTTPS with a bring-your-own-key credential, or a local
//! Ollama/Llamafile endpoint with no key — and drives structured extraction
//! through [`extract`]. [`run_connection_probe`] proves connectivity with one
//! structured round trip, and [`ping_endpoint`] is a cheap reachability check
//! for the local engines.
//!
//! This crate owns `rig-core` and deliberately depends only on the small set of
//! primitives needed to model an engine config and its results. It does not
//! depend on `capture-types` or `app-infra`; the Tauri layer maps its own wire
//! settings onto [`EngineConfig`] and supplies the keychain-resident key.

use rig_core::client::CompletionClient;
use rig_core::providers::{anthropic, llamafile, ollama, openai};

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

mod agent_loop;

pub use agent_loop::{
    run_agent_loop, AgentHistoryTurn, AgentLoopEvent, AgentRole, AgentTool, ToolExecutor,
};

/// Maximum time [`ping_endpoint`] waits for a TCP connection to a local engine.
const PING_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Cloud LLM provider reached over HTTPS with a bring-your-own-key credential.
#[derive(Debug, Clone, Copy)]
pub enum CloudProvider {
    Anthropic,
    Openai,
    OpenAiCompatible,
}

/// Local LLM runtime exposed on a user-controlled endpoint with no credential.
#[derive(Debug, Clone, Copy)]
pub enum LocalKind {
    Ollama,
    Llamafile,
}

/// A fully resolved engine selection: which provider/runtime to talk to, which
/// model to ask for, and (for cloud) the credential to authenticate with.
///
/// The credential lives here only for the duration of a call; it is sourced
/// from the OS keychain by the caller and is never persisted by this crate.
#[derive(Debug, Clone)]
pub enum EngineConfig {
    /// A cloud provider reached over HTTPS with a bring-your-own-key credential.
    ///
    /// `base_url` is `None` for the first-party Anthropic/OpenAI providers (which
    /// use the provider's default endpoint) and `Some(_)` for an
    /// OpenAI-compatible provider reached at a custom Chat Completions base URL.
    Cloud {
        provider: CloudProvider,
        model: String,
        api_key: String,
        base_url: Option<String>,
    },
    /// A local runtime reached on `endpoint` with no credential.
    Local {
        kind: LocalKind,
        endpoint: String,
        model: String,
    },
}

/// Errors surfaced while building an engine client or running an extraction.
#[derive(Debug, thiserror::Error)]
pub enum AiRuntimeError {
    /// The selected engine has no model configured.
    #[error("no model is configured for the selected engine")]
    MissingModel,
    /// A cloud engine was selected without a bring-your-own-key credential.
    #[error("no API key is configured for the selected cloud provider")]
    MissingKey,
    /// An OpenAI-compatible engine was selected without a base URL.
    #[error("no base URL is configured for the OpenAI-compatible provider")]
    MissingBaseUrl,
    /// Constructing the provider client failed.
    #[error("failed to build the engine client: {0}")]
    Build(#[from] rig_core::http_client::Error),
    /// Constructing a provider client from a URL failed.
    #[error("failed to build the engine client: {0}")]
    ClientBuild(#[from] rig_core::client::ProviderClientError),
    /// The structured-extraction round trip failed.
    #[error("structured extraction failed: {0}")]
    Extraction(#[from] rig_core::extractor::ExtractionError),
    /// The streaming agent loop ([`run_agent_loop`]) failed mid-stream — a
    /// provider/completion error or an unrecoverable prompt error. Hitting the
    /// tool-call cap is *not* surfaced here; it ends the loop cleanly.
    #[error("agent loop failed: {0}")]
    AgentLoop(String),
}

/// The structured shape proved by [`run_connection_probe`].
///
/// Both fields are optional so the model can always satisfy the schema even if
/// it omits one of them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionProbe {
    /// Whether the model reports structured output is working.
    pub ok: Option<bool>,
    /// A short free-form confirmation message from the model.
    pub message: Option<String>,
}

/// The generic preamble used by [`extract`] and [`run_connection_probe`].
const PREAMBLE: &str = "You verify structured output works.";

/// Run a generic structured extraction against the selected engine.
///
/// Builds the appropriate provider client for `config`, asks the model to
/// extract a value of type `T` from `prompt`, and returns the deserialized
/// value. `T` must describe its own JSON schema so the model knows the shape to
/// produce. Uses the crate's generic [`PREAMBLE`]; callers that need their own
/// system instruction use [`extract_with_preamble`].
pub async fn extract<T>(config: &EngineConfig, prompt: &str) -> Result<T, AiRuntimeError>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    extract_with_preamble(config, PREAMBLE, prompt).await
}

/// Run a generic structured extraction with a caller-supplied `preamble`.
///
/// Identical to [`extract`] but lets the caller drive the system instruction —
/// used by the User Context derivation worker, which needs task-specific
/// preambles (Activity segmentation, Conclusion distillation, the Sensitive
/// Category Guardrail instruction) rather than the generic structured-output
/// check.
pub async fn extract_with_preamble<T>(
    config: &EngineConfig,
    preamble: &str,
    prompt: &str,
) -> Result<T, AiRuntimeError>
where
    T: schemars::JsonSchema + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    match config {
        EngineConfig::Cloud {
            provider,
            model,
            api_key,
            base_url,
        } => {
            if model.trim().is_empty() {
                return Err(AiRuntimeError::MissingModel);
            }
            if api_key.trim().is_empty() {
                return Err(AiRuntimeError::MissingKey);
            }

            match provider {
                CloudProvider::Anthropic => {
                    let client = anthropic::Client::builder().api_key(api_key).build()?;
                    let extractor = client
                        .extractor::<T>(model.as_str())
                        .preamble(preamble)
                        .build();
                    Ok(extractor.extract(prompt).await?)
                }
                CloudProvider::Openai => {
                    let client = openai::Client::builder().api_key(api_key).build()?;
                    let extractor = client
                        .extractor::<T>(model.as_str())
                        .preamble(preamble)
                        .build();
                    Ok(extractor.extract(prompt).await?)
                }
                CloudProvider::OpenAiCompatible => {
                    // OpenAI-compatible providers (Fireworks, OpenRouter, Together,
                    // …) implement the Chat Completions API, not OpenAI's default
                    // Responses API, so build a `CompletionsClient` pointed at the
                    // user-supplied base URL.
                    let base_url = base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .ok_or(AiRuntimeError::MissingBaseUrl)?;
                    let client = openai::CompletionsClient::builder()
                        .api_key(api_key)
                        .base_url(base_url)
                        .build()?;
                    let extractor = client
                        .extractor::<T>(model.as_str())
                        .preamble(preamble)
                        .build();
                    Ok(extractor.extract(prompt).await?)
                }
            }
        }
        EngineConfig::Local {
            kind,
            endpoint,
            model,
        } => {
            if model.trim().is_empty() {
                return Err(AiRuntimeError::MissingModel);
            }

            match kind {
                LocalKind::Ollama => {
                    // Ollama needs no credential; mirror the provider's own
                    // `from_env` idiom and supply an empty key marker.
                    let client = ollama::Client::builder()
                        .api_key(rig_core::client::Nothing)
                        .base_url(endpoint)
                        .build()?;
                    let extractor = client
                        .extractor::<T>(model.as_str())
                        .preamble(preamble)
                        .build();
                    Ok(extractor.extract(prompt).await?)
                }
                LocalKind::Llamafile => {
                    let client = llamafile::Client::from_url(endpoint)?;
                    let extractor = client
                        .extractor::<T>(model.as_str())
                        .preamble(preamble)
                        .build();
                    Ok(extractor.extract(prompt).await?)
                }
            }
        }
    }
}

/// Run one structured-extraction round trip proving connectivity.
///
/// Asks the selected engine to confirm structured output works and returns the
/// typed [`ConnectionProbe`] it produced. A successful return means the engine
/// was reachable, authenticated, and able to emit schema-conformant JSON.
pub async fn run_connection_probe(config: &EngineConfig) -> Result<ConnectionProbe, AiRuntimeError> {
    extract::<ConnectionProbe>(config, "Reply confirming structured output is working.").await
}

/// Fast reachability check for a local engine endpoint.
///
/// Parses `endpoint` into a `host:port` socket address and attempts a TCP
/// connection with a short timeout. Returns `true` when the connection
/// succeeds. This is a liveness probe only — it does not authenticate or run a
/// model call.
pub async fn ping_endpoint(endpoint: &str) -> bool {
    let Some((host, port)) = parse_host_port(endpoint) else {
        return false;
    };

    let Ok(addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };

    for addr in addrs {
        if TcpStream::connect_timeout(&addr, PING_CONNECT_TIMEOUT).is_ok() {
            return true;
        }
    }

    false
}

/// Resolve a `host` and `port` from an endpoint string.
///
/// Accepts a full URL (`http://localhost:11434`) or a bare `host:port`. When a
/// URL omits the port, the scheme's default is used.
fn parse_host_port(endpoint: &str) -> Option<(String, u16)> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            if let Some(port) = parsed.port_or_known_default() {
                return Some((host.to_string(), port));
            }
        }
    }

    // Fall back to a bare `host:port` form.
    let (host, port) = trimmed.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_endpoint_returns_false_for_closed_port() {
        // 127.0.0.1:1 is a privileged, almost-certainly-closed local port, so
        // the connect attempt fails fast without touching the network.
        let reachable = futures_executor_block_on(ping_endpoint("http://127.0.0.1:1"));
        assert!(!reachable);
    }

    #[test]
    fn parse_host_port_handles_url_and_bare_forms() {
        assert_eq!(
            parse_host_port("http://localhost:11434"),
            Some(("localhost".to_string(), 11434))
        );
        assert_eq!(
            parse_host_port("127.0.0.1:8080"),
            Some(("127.0.0.1".to_string(), 8080))
        );
        // A URL without an explicit port falls back to the scheme default.
        assert_eq!(
            parse_host_port("http://localhost"),
            Some(("localhost".to_string(), 80))
        );
        assert_eq!(parse_host_port(""), None);
        assert_eq!(parse_host_port("not a url"), None);
    }

    /// Minimal blocking driver so the `async` [`ping_endpoint`] can be exercised
    /// without pulling in a full async runtime dependency. The future only
    /// performs a synchronous `std::net` connect, so polling it once to
    /// completion is sufficient.
    fn futures_executor_block_on<F: std::future::Future>(mut fut: F) -> F::Output {
        use std::pin::Pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        fn noop_clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        fn noop(_: *const ()) {}
        static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);

        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        // SAFETY: `fut` is owned and not moved for the duration of the loop.
        let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(out) => return out,
                Poll::Pending => continue,
            }
        }
    }
}
