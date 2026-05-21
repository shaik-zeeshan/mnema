use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "browser-integration-runtime.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcConfig {
    host: String,
    port: u16,
    ipc_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostEnvelope {
    request_id: Option<String>,
    ipc_token: String,
    pairing_token: String,
    channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    secure_entry: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

fn main() {
    while let Ok(message) = read_native_message() {
        let Some(config) = load_ipc_config() else {
            let _ = write_native_message(&serde_json::json!({ "ok": false, "error": "mnema_not_running" }));
            continue;
        };
        let pairing_token = message
            .get("pairingToken")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let channel = message
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let envelope = HostEnvelope {
            request_id: message
                .get("requestId")
                .and_then(Value::as_str)
                .map(str::to_string),
            ipc_token: config.ipc_token.clone(),
            pairing_token,
            secure_entry: (channel == "secureEntry" || channel == "pair")
                .then(|| message.get("signal").cloned())
                .flatten(),
            metadata: (channel == "metadata")
                .then(|| message.get("signal").cloned())
                .flatten(),
            channel,
        };
        let ok = forward_to_app(&config, &envelope).is_ok();
        let _ = write_native_message(&serde_json::json!({
            "ok": ok,
            "requestId": message.get("requestId").and_then(Value::as_str)
        }));
    }
}

fn read_native_message() -> std::io::Result<Value> {
    let mut len = [0u8; 4];
    std::io::stdin().read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    let mut buffer = vec![0u8; len.min(1024 * 1024)];
    std::io::stdin().read_exact(&mut buffer)?;
    serde_json::from_slice(&buffer)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn write_native_message(value: &Value) -> std::io::Result<()> {
    let raw = serde_json::to_vec(value)?;
    std::io::stdout().write_all(&(raw.len() as u32).to_le_bytes())?;
    std::io::stdout().write_all(&raw)?;
    std::io::stdout().flush()
}

fn forward_to_app(config: &IpcConfig, envelope: &HostEnvelope) -> std::io::Result<()> {
    let mut stream = TcpStream::connect((config.host.as_str(), config.port))?;
    let raw = serde_json::to_string(envelope)?;
    stream.write_all(raw.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn load_ipc_config() -> Option<IpcConfig> {
    ipc_config_candidates()
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

fn ipc_config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(
            PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join("com.shaikzeeshan.mnema")
                .join(CONFIG_FILE_NAME),
        );
        candidates.push(PathBuf::from(home).join(".mnema").join(CONFIG_FILE_NAME));
    }
    candidates.push(PathBuf::from(".mnema").join(CONFIG_FILE_NAME));
    candidates
}
