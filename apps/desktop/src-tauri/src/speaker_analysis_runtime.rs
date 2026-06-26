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

/// Subprocess wrapper for on-device speaker analysis.
///
/// The on-device diarization engine (speakrs) runs in an isolated helper
/// subprocess so a native crash or memory blow-up never takes down the main app.
/// The subprocess forwards the request to the engine inside the helper (see
/// [`analyze_request_for_provider`], which remaps any legacy non-speakrs provider
/// to speakrs). One instance of this struct is registered per provider id,
/// sharing the same base `speaker-analysis-models` dir (the per-model subdir is
/// derived inside `analyze_speakrs_request_blocking`).
#[derive(Debug, Clone)]
pub struct SubprocessSpeakerAnalysisProvider {
    provider_id: &'static str,
    models_dir: PathBuf,
}

impl SubprocessSpeakerAnalysisProvider {
    pub fn with_provider(provider_id: &'static str, models_dir: impl Into<PathBuf>) -> Self {
        Self {
            provider_id,
            models_dir: models_dir.into(),
        }
    }
}

#[async_trait]
impl SpeakerAnalysisProvider for SubprocessSpeakerAnalysisProvider {
    fn provider(&self) -> &'static str {
        self.provider_id
    }

    async fn analyze(
        &self,
        request: SpeakerAnalysisRequest,
    ) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
        run_analysis_subprocess(&self.models_dir, &request).await
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
    let output = analyze_request_for_provider(payload.request, &models_dir)
        .map_err(|error| format!("speaker-analysis helper failed: {error}"))?;
    serde_json::to_writer(std::io::stdout(), &output).map_err(|error| {
        format!("failed writing speaker-analysis helper response json: {error}")
    })?;
    std::io::stdout()
        .flush()
        .map_err(|error| format!("failed flushing speaker-analysis helper stdout: {error}"))?;
    Ok(())
}

/// Dispatch a decoded helper request to the on-device engine, rooted at the base
/// `speaker-analysis-models` dir.
///
/// speakrs is the sole on-device provider. Its arm is gated on
/// `all(any(target_os = "macos", target_os = "windows"), feature =
/// "speaker-analysis-speakrs")`: the speakrs engine is compiled into the crate on
/// macOS (CoreML + OpenBLAS) and Windows (CPU via intel-mkl-static) — its
/// Execution Backend is derived per platform (ADR 0004) — so the arm must not
/// reference it on other platforms. When the arm is off (other OS, or the feature
/// disabled) the `#[cfg(not(...))]` branch returns a typed `ProviderUnavailable`;
/// the desktop crate enables the feature by default so the shipped macOS and
/// Windows builds' arm is live.
///
/// MIGRATION: sherpa-onnx is removed. A request that still carries the legacy
/// `provider = "sherpa_onnx"` (or any other non-speakrs provider) — e.g. an
/// in-flight/queued **Speaker Analysis Job** frozen before the removal — is
/// remapped to the speakrs arm rather than erroring, so legacy work re-runs
/// through speakrs instead of failing with an unknown-provider error.
fn analyze_request_for_provider(
    mut request: SpeakerAnalysisRequest,
    models_dir: &Path,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    // Legacy remap: anything that is not the speakrs provider id routes to
    // speakrs (sherpa is gone). Normalize the request's provider so the speakrs
    // engine sees a consistent provider id in its output metadata.
    if request.provider != speaker_analysis::SPEAKRS_PROVIDER_ID {
        request.provider = speaker_analysis::SPEAKRS_PROVIDER_ID.to_string();
    }

    #[cfg(all(
        any(target_os = "macos", target_os = "windows"),
        feature = "speaker-analysis-speakrs"
    ))]
    {
        speaker_analysis::providers::speakrs::analyze_speakrs_request_blocking(request, models_dir)
    }
    #[cfg(not(all(
        any(target_os = "macos", target_os = "windows"),
        feature = "speaker-analysis-speakrs"
    )))]
    {
        let _ = models_dir;
        Err(SpeakerAnalysisError::ProviderUnavailable(format!(
            "speaker-analysis provider '{}' was not compiled into this build",
            request.provider
        )))
    }
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

async fn run_analysis_subprocess(
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

    let mut child = speaker_analysis_helper_command(&current_exe, models_dir)
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
    // Drive the stdin write on its own task so a request larger than the OS pipe
    // buffer can keep flowing while the stdout/stderr readers below drain the
    // child. Writing inline before spawning the readers risks a pipe-full
    // deadlock on Windows (write_all blocks with no timeout while the helper
    // waits to be read). The reader tasks plus the child.wait() timeout below
    // bound the overall operation, so this writer needs no separate timeout.
    let stdin_task = tokio::spawn(async move {
        let result = stdin.write_all(&request_json).await;
        drop(stdin);
        result
    });

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
            stdin_task.abort();
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

    // The child has exited; join the stdin writer so a write failure is not lost.
    // A helper that exits before draining stdin yields BrokenPipe here, which we
    // tolerate because the child's own exit status and stderr already describe
    // the failure; any other write error is surfaced.
    match stdin_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        Ok(Err(error)) => {
            return Err(SpeakerAnalysisError::Subprocess {
                stage: "write_stdin".to_string(),
                message: format!("failed to write speaker-analysis helper stdin: {error}"),
            });
        }
        Err(error) => {
            return Err(SpeakerAnalysisError::Subprocess {
                stage: "write_stdin".to_string(),
                message: format!(
                    "failed joining speaker-analysis helper stdin writer: {error}"
                ),
            });
        }
    }

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

fn speaker_analysis_helper_command(current_exe: &Path, models_dir: &Path) -> Command {
    let mut command = Command::new(current_exe);
    command
        .env(SPEAKER_ANALYSIS_HELPER_ENV, "1")
        .arg(SPEAKER_ANALYSIS_MODELS_DIR_ARG)
        .arg(models_dir)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
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
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use tokio::time::{sleep, Duration};

    fn request_with_timeout(value: Option<serde_json::Value>) -> SpeakerAnalysisRequest {
        let mut request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            speaker_analysis::SPEAKRS_PROVIDER_ID,
            Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
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

    fn dispatch_request_for(provider: &str, models_dir: &Path) -> SpeakerAnalysisRequest {
        // A nonexistent audio path is fine: every arm validates inputs (audio or
        // model dir) before touching native code, so this exercises *routing*
        // without needing real models.
        SpeakerAnalysisRequest::new(
            models_dir.join("missing-audio.m4a"),
            provider,
            None,
            "session-1",
            7,
        )
    }

    /// The routing invariant: with sherpa removed, EVERY provider (speakrs,
    /// legacy sherpa, or any unknown string) routes to the speakrs arm. The only
    /// failures permitted are the speakrs engine's own input/model errors (or, on
    /// a misconfigured feature-off build, `ProviderUnavailable`) — never an
    /// `InvalidRequest("unknown speaker-analysis provider ...")`, because the
    /// dispatcher no longer has an unknown-provider branch.
    fn assert_reached_speakrs_arm(error: &SpeakerAnalysisError) {
        if let SpeakerAnalysisError::InvalidRequest(message) = error {
            assert!(
                !message.contains("unknown speaker-analysis provider"),
                "request fell through to an unknown-provider branch instead of routing to speakrs: {message}"
            );
        }
    }

    #[test]
    fn dispatch_routes_speakrs_to_speakrs_arm() {
        // Routes into the speakrs arm against a nonexistent models dir. With the
        // `speaker-analysis-speakrs` feature ON (the shipped build), the speakrs
        // path surfaces an engine error (MissingModel / audio decode). With the
        // feature OFF, the ProviderUnavailable fallthrough fires. Either way the
        // request reached the arm — never an "unknown" branch.
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nonexistent = tempdir.path().join("does-not-exist");
        let request = dispatch_request_for(speaker_analysis::SPEAKRS_PROVIDER_ID, &nonexistent);
        let error =
            analyze_request_for_provider(request, &nonexistent).expect_err("should fail routing");
        assert_reached_speakrs_arm(&error);
        // When the feature is off, pin the exact fallthrough so a misconfigured
        // build is caught loudly.
        #[cfg(not(feature = "speaker-analysis-speakrs"))]
        assert!(
            matches!(error, SpeakerAnalysisError::ProviderUnavailable(_)),
            "speakrs feature off should yield ProviderUnavailable, got {error:?}"
        );
    }

    #[test]
    fn dispatch_remaps_legacy_sherpa_provider_to_speakrs_arm() {
        // MIGRATION: a legacy job payload frozen with provider "sherpa_onnx" must
        // route to the speakrs arm (sherpa is removed), never the old
        // unknown-provider error.
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nonexistent = tempdir.path().join("does-not-exist");
        let request = dispatch_request_for("sherpa_onnx", &nonexistent);
        let error =
            analyze_request_for_provider(request, &nonexistent).expect_err("should fail routing");
        assert_reached_speakrs_arm(&error);
        #[cfg(not(feature = "speaker-analysis-speakrs"))]
        assert!(
            matches!(error, SpeakerAnalysisError::ProviderUnavailable(_)),
            "speakrs feature off should yield ProviderUnavailable, got {error:?}"
        );
    }

    #[test]
    fn dispatch_remaps_unknown_provider_to_speakrs_arm() {
        // Any unknown provider string also remaps to speakrs rather than erroring.
        let tempdir = tempfile::tempdir().expect("tempdir");
        let nonexistent = tempdir.path().join("does-not-exist");
        let request = dispatch_request_for("totally-made-up", &nonexistent);
        let error =
            analyze_request_for_provider(request, &nonexistent).expect_err("should fail routing");
        assert_reached_speakrs_arm(&error);
        #[cfg(not(feature = "speaker-analysis-speakrs"))]
        assert!(
            matches!(error, SpeakerAnalysisError::ProviderUnavailable(_)),
            "speakrs feature off should yield ProviderUnavailable, got {error:?}"
        );
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

    #[test]
    fn parse_models_dir_accepts_platform_native_path() {
        // The helper resolves the models dir from argv. On Windows the path
        // carries a drive letter and backslashes; `PathBuf::from(OsString)`
        // preserves it verbatim, so the helper never assumes a POSIX layout.
        #[cfg(windows)]
        let models_dir = r"C:\Users\example\AppData\mnema\speaker-analysis-models";
        #[cfg(not(windows))]
        let models_dir = "/home/example/.mnema/speaker-analysis-models";

        let args = vec![
            OsString::from("mnema.exe"),
            OsString::from(SPEAKER_ANALYSIS_MODELS_DIR_ARG),
            OsString::from(models_dir),
        ];
        let parsed = parse_models_dir_from_args(args).expect("models dir parses");
        assert_eq!(parsed, PathBuf::from(models_dir));
    }

    #[test]
    fn helper_request_payload_round_trips_over_byte_stream() {
        // The helper frames its request/response as raw UTF-8 JSON bytes over
        // stdin/stdout with no newline delimiter — the host writes
        // `serde_json::to_vec`, the helper reads the whole stream and parses it.
        // This must survive byte-for-byte on Windows (Rust stdio is binary, so
        // there is no CRLF translation), so assert the exact serialize -> parse
        // round trip the two sides rely on.
        let request = SpeakerAnalysisRequest::new(
            "/tmp/audio.m4a",
            speaker_analysis::SPEAKRS_PROVIDER_ID,
            Some(speaker_analysis::SPEAKRS_DEFAULT_MODEL_ID.to_string()),
            "session-round-trip",
            7,
        );
        let payload = SpeakerAnalysisHelperPayload {
            request: request.clone(),
        };
        let bytes = serde_json::to_vec(&payload).expect("serialize request payload");
        assert!(
            !bytes.contains(&b'\n'),
            "payload framing must not depend on a trailing newline"
        );
        let decoded: SpeakerAnalysisHelperPayload =
            serde_json::from_slice(&bytes).expect("parse request payload");
        assert_eq!(decoded.request.session_id, request.session_id);
        assert_eq!(decoded.request.audio_segment_id, request.audio_segment_id);
        assert_eq!(decoded.request.provider, request.provider);
    }

    // Windows kill-on-drop: spawning a long-lived child through tokio with the
    // same `kill_on_drop(true)` flag the helper sets must terminate it when the
    // handle is dropped (tokio maps the flag to `TerminateProcess` on Windows).
    // We spawn `ping -n 30 127.0.0.1` directly rather than through
    // `speaker_analysis_helper_command` because the helper builder hard-codes
    // the `--speaker-analysis-models-dir` argv that `ping` would reject; the
    // kill semantics under test are identical (same `kill_on_drop` flag), and
    // the production argv/env/stdio shape is already covered by the JSON
    // round-trip and `parse_models_dir` tests above. Verifying the kill against
    // the *real speakrs helper subprocess* on multi-speaker hardware is the
    // operator-deferred gap (and speakrs is macOS-only, so the live subprocess
    // never runs on Windows).
    #[cfg(windows)]
    #[test]
    fn kill_on_drop_terminates_child_on_windows() {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("runtime")
            .block_on(async {
                use tokio::time::{sleep, Duration};

                let child = Command::new("ping")
                    .arg("-n")
                    .arg("30")
                    .arg("127.0.0.1")
                    .kill_on_drop(true)
                    .stdout(Stdio::null())
                    .spawn()
                    .expect("spawn ping");
                let pid = child.id().expect("child has a pid");

                sleep(Duration::from_millis(300)).await;
                assert!(process_is_alive(pid), "ping child did not start");

                drop(child);
                sleep(Duration::from_millis(1_500)).await;

                assert!(
                    !process_is_alive(pid),
                    "dropped child {pid} kept running after kill_on_drop"
                );
            });
    }

    #[cfg(windows)]
    fn process_is_alive(pid: u32) -> bool {
        // `tasklist /FI "PID eq <pid>"` lists the process row when alive and
        // prints "INFO: No tasks..." otherwise; the PID appears in the CSV row
        // (quoted) only when the process exists.
        let output = std::process::Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {pid}"))
            .arg("/NH")
            .arg("/FO")
            .arg("CSV")
            .output()
            .expect("run tasklist");
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains(&format!("\"{pid}\""))
    }

    #[cfg(unix)]
    #[test]
    fn helper_command_kills_child_when_dropped() {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("runtime")
            .block_on(async {
                assert_helper_command_kills_child_when_dropped().await;
            });
    }

    #[cfg(unix)]
    async fn assert_helper_command_kills_child_when_dropped() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let helper_path = tempdir.path().join("helper.sh");
        let started_path = tempdir.path().join("started");
        let survived_path = tempdir.path().join("survived");
        fs::write(
            &helper_path,
            format!(
                "#!/bin/sh\nprintf started > '{}'\nsleep 1\nprintf survived > '{}'\n",
                started_path.display(),
                survived_path.display()
            ),
        )
        .expect("write helper");
        let mut permissions = fs::metadata(&helper_path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&helper_path, permissions).expect("chmod helper");

        let child = speaker_analysis_helper_command(&helper_path, tempdir.path())
            .spawn()
            .expect("spawn helper");

        for _ in 0..50 {
            if started_path.exists() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(started_path.exists(), "helper did not start");

        drop(child);
        sleep(Duration::from_millis(1_200)).await;

        assert!(
            !survived_path.exists(),
            "dropped helper child kept running long enough to write survived marker"
        );
    }
}
