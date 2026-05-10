use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use speaker_analysis::{
    providers::sherpa_onnx::{analyze_sherpa_request_blocking, SherpaOnnxSpeakerAnalysisProvider},
    SpeakerAnalysisOutput, SpeakerAnalysisProvider, SpeakerAnalysisRequest, SpeakerAnalysisResult,
};

const SPEAKER_ANALYSIS_HELPER_ENV: &str = "MNEMA_SPEAKER_ANALYSIS_HELPER";
const SPEAKER_ANALYSIS_MODELS_DIR_ARG: &str = "--speaker-analysis-models-dir";

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
        run_sherpa_analysis_subprocess(&self.models_dir, &request)
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

fn run_sherpa_analysis_subprocess(
    models_dir: &Path,
    request: &SpeakerAnalysisRequest,
) -> SpeakerAnalysisResult<SpeakerAnalysisOutput> {
    let current_exe = std::env::current_exe().map_err(|error| {
        speaker_analysis::SpeakerAnalysisError::Analysis(format!(
            "failed to locate Mnema executable for speaker-analysis helper: {error}"
        ))
    })?;
    let payload = SpeakerAnalysisHelperPayload {
        request: request.clone(),
    };
    let request_json = serde_json::to_vec(&payload).map_err(|error| {
        speaker_analysis::SpeakerAnalysisError::Analysis(format!(
            "failed to serialize speaker-analysis helper request: {error}"
        ))
    })?;

    let mut child = Command::new(current_exe)
        .env(SPEAKER_ANALYSIS_HELPER_ENV, "1")
        .arg(SPEAKER_ANALYSIS_MODELS_DIR_ARG)
        .arg(models_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            speaker_analysis::SpeakerAnalysisError::Analysis(format!(
                "failed to spawn Mnema speaker-analysis helper: {error}"
            ))
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&request_json).map_err(|error| {
            speaker_analysis::SpeakerAnalysisError::Analysis(format!(
                "failed to write speaker-analysis helper stdin: {error}"
            ))
        })?;
    }

    let output = child.wait_with_output().map_err(|error| {
        speaker_analysis::SpeakerAnalysisError::Analysis(format!(
            "failed waiting for speaker-analysis helper: {error}"
        ))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = output.status.signal() {
                return Err(speaker_analysis::SpeakerAnalysisError::Analysis(format!(
                    "speaker-analysis helper crashed with signal {signal}: {stderr}"
                )));
            }
        }

        return Err(speaker_analysis::SpeakerAnalysisError::Analysis(format!(
            "speaker-analysis helper exited with status {:?}: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    serde_json::from_slice(&output.stdout).map_err(|error| {
        speaker_analysis::SpeakerAnalysisError::Analysis(format!(
            "failed to parse speaker-analysis helper response: {error}"
        ))
    })
}
