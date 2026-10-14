//! The developer window's 12 health rows (`get_debug_health`, polled every 2 s).
//!
//! This file gathers; [`rules`] decides. The gather reuses what the window
//! already reads: B2's capture snapshot, the pipeline `GROUP BY`, the semantic
//! index status, the startup report and two in-memory reads. The slow-moving
//! parts (CLI files, User Context runs, the vault-backed AI check) are cached
//! for [`SLOW_TTL`]. No network, and never logs: it is a poll.

mod rules;

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::Manager;

use crate::app_infra::{AppInfraState, StartupReport};
use crate::debug_status::{
    get_semantic_index_status, SemanticIndexStatusDto, SemanticWorkerHealthState,
};
use crate::native_capture::capture_health::{capture_health_snapshot, CaptureHealthDto};
use crate::native_capture::metadata::capture_privacy_debug_info;
use crate::native_capture::{
    read_recording_settings, CaptureMetadataState, NativeCaptureState, RecordingSettingsState,
};

pub use rules::roll_up;

/// A row's tone. The frontend ranks them (error > warn > info > ok > off), so
/// no `Ord` here on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugSeverity {
    Ok,
    Info,
    Warn,
    Error,
    Off,
}

/// One health row, in [`roll_up`]'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugFeature {
    Screen,
    Microphone,
    SystemAudio,
    Reading,
    Transcription,
    Speakers,
    MeaningIndex,
    AiEngine,
    UserContext,
    Disk,
    CliAccess,
    Startup,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureHealthDto {
    pub feature: DebugFeature,
    pub severity: DebugSeverity,
    /// One plain sentence in the mockup's voice.
    pub reason: String,
    /// Short right-column text ("14 waiting", "182 GB", "3rd today").
    pub metric: Option<String>,
    /// When set, the UI shows this as a local HH:MM instead of `metric`.
    pub metric_at_ms: Option<i64>,
}

pub struct AiEngineInputs {
    pub enabled: bool,
    /// The default model's provider kind, as a display name.
    pub provider: Option<&'static str>,
    /// The sync, no-network prerequisite. `None` until startup has unlocked
    /// the vault (reading it earlier could raise a keychain prompt).
    pub status: Option<Result<(), String>>,
    pub last_turn_at_ms: Option<i64>,
}

pub struct UserContextInputs {
    pub enabled: bool,
    pub last_run_failed: bool,
    pub last_derived_at_ms: Option<i64>,
    /// `None` when no day digest exists yet.
    pub digest_fresh: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum CliInputs {
    NotInstalled,
    Unreadable,
    Ready {
        allowed: usize,
        blocked: usize,
        /// Audit events since local midnight.
        today: usize,
    },
}

/// Everything [`roll_up`] needs, already gathered.
pub struct DebugHealthInputs {
    pub now_ms: i64,
    pub capture: CaptureHealthDto,
    /// Apps the latest privacy decision excludes that the live screen filter
    /// doesn't hide yet (decided but not applied).
    pub privacy_unhidden_app_count: usize,
    /// Only processors with at least one job appear; a missing lane is idle.
    pub pipeline: Vec<::app_infra::ProcessorPipelineStatus>,
    pub transcription_provider: &'static str,
    pub ocr_enabled: bool,
    pub transcription_enabled: bool,
    pub speakers_enabled: bool,
    pub semantic_enabled: bool,
    /// The selected Semantic Search Model is on disk. Without one the worker
    /// idles and search is keyword-only, however long the backlog grows.
    pub semantic_model_installed: bool,
    pub semantic: SemanticIndexStatusDto,
    pub ai: AiEngineInputs,
    pub user_context: UserContextInputs,
    pub cli: CliInputs,
    pub startup: Option<StartupReport>,
    /// Deferred startup unwound before it could file `startup`.
    pub startup_panicked: bool,
}

// ── Slow inputs, cached ────────────────────────────────────────────────────

const SLOW_TTL: Duration = Duration::from_secs(30);
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone)]
struct SlowInputs {
    ai_status: Option<Result<(), String>>,
    uc_last_run_failed: bool,
    uc_last_derived_at_ms: Option<i64>,
    digest_fresh: Option<bool>,
    cli: CliInputs,
}

// ponytail: one process-wide slot; the window is the only caller.
static SLOW: Mutex<Option<(Instant, SlowInputs)>> = Mutex::new(None);

/// Excluded apps the latest decision wants hidden that the applied filter
/// doesn't hide. (The applied decision's own `privacy_filter_applied` is just
/// "its list is non-empty", so it can't tell this.)
pub(crate) fn unhidden_app_count(decided: &[String], applied: &[String]) -> usize {
    decided.iter().filter(|id| !applied.contains(id)).count()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn provider_label(kind: capture_types::AiProviderKind) -> &'static str {
    use capture_types::AiProviderKind as K;
    match kind {
        K::Anthropic => "Anthropic",
        K::Openai => "OpenAI",
        K::OpenaiCompatible => "OpenAI-compatible",
        K::Chatgpt => "ChatGPT",
        K::Ollama => "Ollama",
        K::Llamafile => "Llamafile",
    }
}

fn transcription_label(provider: capture_types::AudioTranscriptionProvider) -> &'static str {
    use capture_types::AudioTranscriptionProvider as P;
    match provider {
        P::LocalWhisper => "Whisper",
        P::AppleSpeechOnDevice => "Apple Speech",
        P::Parakeet => "Parakeet",
        P::Deepgram => "Deepgram",
    }
}

async fn read_cli(app_handle: &tauri::AppHandle, day_start_ms: i64) -> CliInputs {
    let installed = crate::app_infra::get_cli_status_inner(app_handle.clone())
        .await
        .is_ok_and(|status| status.installed);
    if !installed {
        return CliInputs::NotInstalled;
    }
    let Ok(config_dir) = app_handle.path().app_config_dir() else {
        return CliInputs::Unreadable;
    };
    let access = ::app_infra::brokered_access::BrokeredCaptureAccess::from_config_dir(config_dir);
    let (Ok(grants), Ok(history)) = (access.list_grants(), access.list_history()) else {
        return CliInputs::Unreadable;
    };
    let blocked = grants.grants.iter().filter(|grant| grant.blocked).count();
    CliInputs::Ready {
        allowed: grants.grants.len() - blocked,
        blocked,
        today: history
            .events
            .iter()
            .filter(|event| event.timestamp_unix_ms as i64 >= day_start_ms)
            .count(),
    }
}

async fn read_slow(
    app_handle: &tauri::AppHandle,
    infra: &AppInfraState,
    ai_settings: &capture_types::AiRuntimeSettings,
    startup: Option<&StartupReport>,
    now: i64,
) -> SlowInputs {
    // Only after startup tried the vault: before it, a read could prompt. A
    // denied vault short-circuits so the resolver never re-reads (and re-logs).
    let ai_status = startup.filter(|_| ai_settings.enabled).map(|report| {
        if report.vault_unlocked {
            crate::ai_runtime::engine_prerequisite_static(ai_settings).map(|_| ())
        } else {
            Err("vault_denied".to_string())
        }
    });

    // Best-effort: a read error degrades to "unknown", never fails the rollup.
    let store = infra.user_context();
    let uc_last_run_failed = store
        .list_derivation_runs(1)
        .await
        .ok()
        .and_then(|runs| runs.into_iter().next())
        .is_some_and(|run| run.status == "failed");
    let uc_last_derived_at_ms = store.last_derived_at_ms().await.ok().flatten();
    // Fresh = some day digest covers now. Not "the latest-generated one ends
    // after now": regenerating yesterday's would make today's look missing.
    let digest_fresh = match store.list_day_digests_in_range(now, now + 1).await {
        Ok(covering) if !covering.is_empty() => Some(true),
        Ok(_) => store
            .latest_day_digest()
            .await
            .ok()
            .flatten()
            .map(|_| false),
        Err(_) => None,
    };
    // Local midnight from the frontend-stamped offset (UTC when never stamped);
    // the `time` crate can't read the local offset soundly here.
    let offset_ms = store
        .local_offset_minutes()
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
        * 60_000;
    let day_start_ms = (now + offset_ms).div_euclid(DAY_MS) * DAY_MS - offset_ms;

    SlowInputs {
        ai_status,
        uc_last_run_failed,
        uc_last_derived_at_ms,
        digest_fresh,
        cli: read_cli(app_handle, day_start_ms).await,
    }
}

#[tauri::command]
pub async fn get_debug_health(
    app_handle: tauri::AppHandle,
    infra: tauri::State<'_, AppInfraState>,
    worker_health: tauri::State<'_, SemanticWorkerHealthState>,
    metadata_state: tauri::State<'_, CaptureMetadataState>,
    settings_state: tauri::State<'_, RecordingSettingsState>,
) -> Result<Vec<FeatureHealthDto>, String> {
    let privacy = capture_privacy_debug_info(metadata_state.inner());
    let settings = read_recording_settings(settings_state.inner());
    // Off the async workers: the free-space read can block on a slow volume.
    let capture = {
        let app = app_handle.clone();
        tauri::async_runtime::spawn_blocking(move || {
            capture_health_snapshot(&app.state::<NativeCaptureState>(), &app)
        })
        .await
        .map_err(|error| format!("failed to read capture health: {error}"))?
    };
    let pipeline = infra
        .processing_pipeline_status()
        .await
        .map_err(|error| format!("failed to read processing pipeline status: {error}"))?;
    let semantic = get_semantic_index_status(infra.clone(), worker_health).await?;
    let startup = crate::app_infra::startup_report();
    let now = now_ms();

    let cached = SLOW
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < SLOW_TTL)
        .map(|(_, slow)| slow.clone());
    let slow = match cached {
        Some(slow) => slow,
        None => {
            let slow = read_slow(
                &app_handle,
                &infra,
                &settings.ai_runtime,
                startup.as_ref(),
                now,
            )
            .await;
            *SLOW.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some((Instant::now(), slow.clone()));
            slow
        }
    };

    Ok(roll_up(&DebugHealthInputs {
        now_ms: now,
        capture,
        privacy_unhidden_app_count: unhidden_app_count(
            &privacy.latest_decision.excluded_bundle_ids,
            &privacy.currently_excluded_bundle_ids,
        ),
        pipeline,
        transcription_provider: transcription_label(settings.transcription.provider),
        ocr_enabled: settings.ocr.enabled,
        transcription_enabled: settings.transcription.enabled,
        // Speaker splitting runs on transcription output, so it needs both.
        speakers_enabled: settings.transcription.enabled
            && settings.speaker_analysis.separate_speakers,
        semantic_enabled: settings.semantic_search.enabled,
        semantic_model_installed: app_handle.path().app_data_dir().is_ok_and(|dir| {
            crate::semantic_search_worker::selected_model_available(&dir, &settings.semantic_search)
        }),
        semantic,
        ai: AiEngineInputs {
            enabled: settings.ai_runtime.enabled,
            provider: crate::user_context::worker::default_provider_kind(&settings.ai_runtime)
                .map(provider_label),
            status: slow.ai_status,
            last_turn_at_ms: crate::ask_ai::get_ask_ai_last_turn_usage()
                .map(|usage| usage.started_at_ms),
        },
        user_context: UserContextInputs {
            enabled: settings.user_context.enabled,
            last_run_failed: slow.uc_last_run_failed,
            last_derived_at_ms: slow.uc_last_derived_at_ms,
            digest_fresh: slow.digest_fresh,
        },
        cli: slow.cli,
        startup,
        startup_panicked: crate::app_infra::startup_panicked(),
    }))
}
