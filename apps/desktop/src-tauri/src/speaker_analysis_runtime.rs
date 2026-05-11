use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Instant,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use speaker_analysis::{
    providers::sherpa_onnx::{analyze_sherpa_request_blocking, SherpaOnnxSpeakerAnalysisProvider},
    SpeakerAnalysisError, SpeakerAnalysisOutput, SpeakerAnalysisProvider, SpeakerAnalysisRequest,
    SpeakerAnalysisResult,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::{timeout, Duration},
};

const SPEAKER_ANALYSIS_HELPER_ENV: &str = "MNEMA_SPEAKER_ANALYSIS_HELPER";
const SPEAKER_ANALYSIS_MODELS_DIR_ARG: &str = "--speaker-analysis-models-dir";
const DEFAULT_HELPER_TIMEOUT_SECONDS: u64 = 600;
const MIN_HELPER_TIMEOUT_SECONDS: u64 = 60;
const MAX_HELPER_TIMEOUT_SECONDS: u64 = 3600;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerAnalysisHelperPayload {
    request: SpeakerAnalysisRequest,
}

#[derive(Debug, Clone)]
pub struct SubprocessSherpaOnnxSpeakerAnalysisProvider {
    models_dir: PathBuf,
}

impl SubprocessSherpaOnnxSpeakerAnalysisProvider {
    pub fn with_models_dir(models_dir: impl Into<PathBuf>) -> Self {
        Self {
            models_dir: models_dir.into(),
        }
    }
}

#[async_trait]
impl SpeakerAnalysisProvider for SubprocessSherpaOnnxSpeakerAnalysisProvider {
    fn provider(&self) -> &'static str {
        SherpaOnnxSpeakerAnalysisProvider::with_models_dir(&self.models_dir).provider()
    }

    async fn analyze(
        &self,
        request: SpeakerAnalysisRequest,
    ) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
        run_sherpa_analysis_subprocess(&self.models_dir, &request).await
    }
}

pub fn maybe_run_subprocess_helper_and_exit() {
    if std::env::var_os(SPEAKER_ANALYSIS_HELPER_ENV).is_none() {
        return;
    }

    let exit_code = match run_subprocess_helper() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run_subprocess_helper() -> Result<(), String> {
    let models_dir = parse_models_dir_from_args(std::env::args_os())?;
    let mut request_json = String::new();
    std::io::stdin()
        .read_to_string(&mut request_json)
        .map_err(|error| format!("failed reading speaker-analysis helper stdin: {error}"))?;
    let payload: SpeakerAnalysisHelperPayload = serde_json::from_str(&request_json)
        .map_err(|error| format!("failed parsing speaker-analysis helper request json: {error}"))?;
    let output = analyze_sherpa_request_blocking(payload.request, &models_dir)
        .map_err(|error| format!("speaker-analysis helper failed: {error}"))?;
    serde_json::to_writer(std::io::stdout(), &output).map_err(|error| {
        format!("failed writing speaker-analysis helper response json: {error}")
    })?;
    std::io::stdout()
        .flush()
        .map_err(|error| format!("failed flushing speaker-analysis helper stdout: {error}"))?;
    Ok(())
}

fn parse_models_dir_from_args(args: impl IntoIterator<Item = OsString>) -> Result<PathBuf, String> {
    let mut args = args.into_iter();
    let _ = args.next();
    while let Some(arg) = args.next() {
        if arg == SPEAKER_ANALYSIS_MODELS_DIR_ARG {
            let Some(value) = args.next() else {
                return Err(format!(
                    "{SPEAKER_ANALYSIS_MODELS_DIR_ARG} requires a path argument"
                ));
            };
            return Ok(PathBuf::from(value));
        }
    }

    Err(format!(
        "speaker-analysis helper requires {SPEAKER_ANALYSIS_MODELS_DIR_ARG}"
    ))
}

async fn run_sherpa_analysis_subprocess(
    models_dir: &Path,
    request: &SpeakerAnalysisRequest,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    let helper_timeout_seconds = helper_timeout_seconds_from_request(request);
    let started_at = Instant::now();
    let current_exe =
        std::env::current_exe().map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "locate_executable".to_string(),
            message: format!(
                "failed to locate Mnema executable for speaker-analysis helper: {error}"
            ),
        })?;
    let payload = SpeakerAnalysisHelperPayload {
        request: request.clone(),
    };
    let request_json =
        serde_json::to_vec(&payload).map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "serialize_request".to_string(),
            message: format!("failed to serialize speaker-analysis helper request: {error}"),
        })?;

    eprintln!(
        "speaker-analysis helper starting: audio_segment_id={} session_id='{}' timeout_seconds={} audio_path='{}'",
        request.audio_segment_id,
        request.session_id,
        helper_timeout_seconds,
        request.audio_path.display()
    );

    let mut child = Command::new(current_exe)
        .env(SPEAKER_ANALYSIS_HELPER_ENV, "1")
        .arg(SPEAKER_ANALYSIS_MODELS_DIR_ARG)
        .arg(models_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "spawn_helper".to_string(),
            message: format!("failed to spawn Mnema speaker-analysis helper: {error}"),
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| SpeakerAnalysisError::Subprocess {
            stage: "write_stdin".to_string(),
            message: "speaker-analysis helper stdin was unavailable".to_string(),
        })?;
    stdin
        .write_all(&request_json)
        .await
        .map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "write_stdin".to_string(),
            message: format!("failed to write speaker-analysis helper stdin: {error}"),
        })?;
    drop(stdin);

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| SpeakerAnalysisError::Subprocess {
            stage: "wait".to_string(),
            message: "speaker-analysis helper stdout was unavailable".to_string(),
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| SpeakerAnalysisError::Subprocess {
            stage: "wait".to_string(),
            message: "speaker-analysis helper stderr was unavailable".to_string(),
        })?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let status = match timeout(Duration::from_secs(helper_timeout_seconds), child.wait()).await {
        Ok(result) => result.map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "wait".to_string(),
            message: format!("failed waiting for speaker-analysis helper: {error}"),
        })?,
        Err(_) => {
            let elapsed_ms = started_at.elapsed().as_millis();
            let kill_result = child.kill().await;
            let _ = child.wait().await;
            eprintln!(
                "speaker-analysis helper timeout: elapsed_ms={} timeout_seconds={} kill_result={:?}",
                elapsed_ms, helper_timeout_seconds, kill_result
            );
            stdout_task.abort();
            stderr_task.abort();
            return Err(match kill_result {
                Ok(()) => SpeakerAnalysisError::Subprocess {
                    stage: "timeout".to_string(),
                    message: format!(
                        "speaker-analysis helper timed out after {helper_timeout_seconds}s"
                    ),
                },
                Err(error) => SpeakerAnalysisError::Subprocess {
                    stage: "kill_timeout".to_string(),
                    message: format!(
                        "speaker-analysis helper timed out after {helper_timeout_seconds}s and kill failed: {error}"
                    ),
                },
            });
        }
    };
    let stdout = join_reader_task(stdout_task, "stdout").await?;
    let stderr = join_reader_task(stderr_task, "stderr").await?;

    if !status.success() {
        let stderr = trimmed_stderr(&stderr);
        eprintln!(
            "speaker-analysis helper failed: elapsed_ms={} status={:?} stderr='{}'",
            started_at.elapsed().as_millis(),
            status.code(),
            stderr
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Err(SpeakerAnalysisError::Subprocess {
                    stage: "helper_signal".to_string(),
                    message: format!(
                        "speaker-analysis helper crashed with signal {signal}: {stderr}"
                    ),
                });
            }
        }

        return Err(SpeakerAnalysisError::Subprocess {
            stage: "helper_exit".to_string(),
            message: format!(
                "speaker-analysis helper exited with status {:?}: {}",
                status.code(),
                stderr
            ),
        });
    }

    let mut output: SpeakerAnalysisOutput =
        serde_json::from_slice(&stdout).map_err(|error| SpeakerAnalysisError::MalformedOutput {
            message: format!(
                "stage parse_stdout: failed to parse speaker-analysis helper response: {error}"
            ),
        })?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    output
        .metadata
        .provenance
        .insert("elapsedMs".to_string(), json!(elapsed_ms));
    output.metadata.provenance.insert(
        "helperTimeoutSeconds".to_string(),
        json!(helper_timeout_seconds),
    );
    eprintln!(
        "speaker-analysis helper completed: elapsed_ms={} timeout_seconds={} stdout_bytes={}",
        elapsed_ms,
        helper_timeout_seconds,
        stdout.len()
    );
    Ok(output)
}

async fn join_reader_task(
    task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stream_name: &str,
) -> SpeakerAnalysisResult<Vec<u8>> {
    task.await
        .map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "wait".to_string(),
            message: format!(
                "failed joining speaker-analysis helper {stream_name} reader: {error}"
            ),
        })?
        .map_err(|error| SpeakerAnalysisError::Subprocess {
            stage: "wait".to_string(),
            message: format!("failed reading speaker-analysis helper {stream_name}: {error}"),
        })
}

pub fn helper_timeout_seconds_from_request(request: &SpeakerAnalysisRequest) -> u64 {
    request
        .options
        .get(::app_infra::HELPER_TIMEOUT_SECONDS_OPTION)
        .and_then(serde_json::Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_HELPER_TIMEOUT_SECONDS)
        .clamp(MIN_HELPER_TIMEOUT_SECONDS, MAX_HELPER_TIMEOUT_SECONDS)
}

fn trimmed_stderr(stderr: &[u8]) -> String {
    let trimmed = String::from_utf8_lossy(stderr).trim().to_string();
    if trimmed.is_empty() {
        "<empty stderr>".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_timeout(value: Option<serde_json::Value>) -> SpeakerAnalysisRequest {
        let mut request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            speaker_analysis::SHERPA_ONNX_PROVIDER_ID,
            Some(speaker_analysis::DEFAULT_SHERPA_ONNX_MODEL_ID.to_string()),
            "session-1",
            42,
        );
        if let Some(value) = value {
            request.options.insert(
                ::app_infra::HELPER_TIMEOUT_SECONDS_OPTION.to_string(),
                value,
            );
        }
        request
    }

    #[test]
    fn helper_timeout_defaults_when_missing_or_invalid() {
        assert_eq!(
            helper_timeout_seconds_from_request(&request_with_timeout(None)),
            600
        );
        assert_eq!(
            helper_timeout_seconds_from_request(&request_with_timeout(Some(serde_json::json!(
                "slow"
            )))),
            600
        );
    }

    #[test]
    fn helper_timeout_clamps_to_supported_range() {
        assert_eq!(
            helper_timeout_seconds_from_request(&request_with_timeout(Some(serde_json::json!(1)))),
            60
        );
        assert_eq!(
            helper_timeout_seconds_from_request(&request_with_timeout(Some(serde_json::json!(
                7200
            )))),
            3600
        );
        assert_eq!(
            helper_timeout_seconds_from_request(&request_with_timeout(Some(serde_json::json!(
                900
            )))),
            900
        );
    }
}
