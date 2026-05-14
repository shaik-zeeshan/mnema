use capture_types::{
    CaptureErrorResponse, CaptureOutputFiles, CapturePermissionState, ScreenResolution,
    ScreenResolutionPreset,
};
#[cfg(target_os = "macos")]
use capture_writers::{
    append_audio_sample_to_writer, append_video_sample_to_writer, create_audio_asset_writer,
    create_video_asset_writer_for_sample_buf,
    finalize_screen_video_output_context as writers_finalize_screen_video_output_context,
    finish_audio_asset_writer, finish_audio_asset_writer_discarding_inactivity_tail,
    finish_video_asset_writer, set_audio_writer_inactivity_tail_trim_seconds,
    AudioAssetWriterState, VideoAssetWriterState,
};

#[cfg(target_os = "macos")]
use cidre::arc::Retain;
#[cfg(target_os = "macos")]
use cidre::dispatch;
#[cfg(target_os = "macos")]
use cidre::objc;
#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
mod equivalence;

use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::fmt::Display;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU32, AtomicU64};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCaptureSources {
    pub screen: bool,
    pub system_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureResolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFrameArtifact {
    pub file_path: String,
    pub captured_at_unix_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub captured_frame_equivalence: CapturedFrameEquivalenceOutcome,
}

pub use equivalence::{
    captured_frame_equivalence_from_image_path, captured_frame_equivalence_from_interleaved_bytes,
    captured_frame_equivalence_proofs_match, CapturedFrameEquivalence,
    CapturedFrameEquivalenceOutcome, CAPTURED_FRAME_EQUIVALENCE_VERSION,
};

pub type ScreenFrameArtifactHandler =
    std::sync::Arc<dyn Fn(ScreenFrameArtifact) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyContentFilter {
    pub display_id: u32,
    pub excluded_bundle_ids: Vec<String>,
    pub excluded_window_ids: Vec<u32>,
}

impl PrivacyContentFilter {
    pub fn key(&self) -> capture_metadata::PrivacyFilterKey {
        capture_metadata::PrivacyFilterKey::new(
            self.display_id,
            self.excluded_bundle_ids.clone(),
            self.excluded_window_ids.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivacyFilterApplyErrorKind {
    DisplayUnavailable,
    FilterUpdateFailed,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyFilterApplyError {
    pub kind: PrivacyFilterApplyErrorKind,
    pub message: String,
}

pub fn privacy_filter_changed(
    previous: Option<&PrivacyContentFilter>,
    next: &PrivacyContentFilter,
) -> bool {
    previous.map(PrivacyContentFilter::key).as_ref() != Some(&next.key())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivacyContentFilterStrategy {
    ExcludingWindows,
    ExcludingApps,
    ExcludingAppWindowsAndWindows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyFilterAvailableApp {
    bundle_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyFilterAvailableWindow {
    id: u32,
    owning_bundle_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyContentFilterPlan {
    strategy: PrivacyContentFilterStrategy,
    excluded_bundle_ids: std::collections::BTreeSet<String>,
    excluded_window_ids: std::collections::BTreeSet<u32>,
    unresolved_bundle_ids: Vec<String>,
    unresolved_window_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyFilterAppliedKey {
    display_id: u32,
    strategy: PrivacyContentFilterStrategy,
    excluded_bundle_ids: Vec<String>,
    excluded_window_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrivacyFilterApplicationState {
    requested_key: capture_metadata::PrivacyFilterKey,
    applied_key: PrivacyFilterAppliedKey,
    unresolved_bundle_ids: Vec<String>,
    unresolved_window_ids: Vec<u32>,
}

impl PrivacyFilterApplicationState {
    fn from_plan(privacy_filter: &PrivacyContentFilter, plan: &PrivacyContentFilterPlan) -> Self {
        Self {
            requested_key: privacy_filter.key(),
            applied_key: PrivacyFilterAppliedKey {
                display_id: privacy_filter.display_id,
                strategy: plan.strategy,
                excluded_bundle_ids: plan.excluded_bundle_ids.iter().cloned().collect(),
                excluded_window_ids: plan.excluded_window_ids.iter().copied().collect(),
            },
            unresolved_bundle_ids: plan.unresolved_bundle_ids.clone(),
            unresolved_window_ids: plan.unresolved_window_ids.clone(),
        }
    }

    fn satisfies_request(&self, requested_key: &capture_metadata::PrivacyFilterKey) -> bool {
        self.requested_key == *requested_key
            && self.unresolved_bundle_ids.is_empty()
            && self.unresolved_window_ids.is_empty()
            && !self.requires_content_refresh()
    }

    fn applied_filter_matches(&self, next: &Self) -> bool {
        self.applied_key == next.applied_key
    }

    fn diagnostic_state_matches(&self, next: &Self) -> bool {
        self.requested_key == next.requested_key
            && self.applied_key == next.applied_key
            && self.unresolved_bundle_ids == next.unresolved_bundle_ids
            && self.unresolved_window_ids == next.unresolved_window_ids
    }

    fn requires_content_refresh(&self) -> bool {
        self.applied_key.strategy == PrivacyContentFilterStrategy::ExcludingAppWindowsAndWindows
    }
}

fn should_log_privacy_filter_diagnostics(
    previous: Option<&PrivacyFilterApplicationState>,
    next: &PrivacyFilterApplicationState,
) -> bool {
    previous.map_or(true, |previous| !previous.diagnostic_state_matches(next))
}

fn log_privacy_filter_diagnostics(state: &PrivacyFilterApplicationState) {
    if !state.unresolved_bundle_ids.is_empty() {
        capture_runtime::debug_log!(
            "privacy filter requested bundle ids not present in ScreenCaptureKit content: {:?}",
            state.unresolved_bundle_ids
        );
    }
    if !state.unresolved_window_ids.is_empty() {
        capture_runtime::debug_log!(
            "privacy filter requested window ids not present in ScreenCaptureKit content: {:?}",
            state.unresolved_window_ids
        );
    }
    if state.applied_key.strategy == PrivacyContentFilterStrategy::ExcludingAppWindowsAndWindows {
        capture_runtime::debug_log!(
            "privacy filter using mixed bundle/window request as excluded app windows plus requested windows: bundles={:?}, windows={:?}, resolved_windows={:?}",
            state.requested_key.bundle_ids,
            state.requested_key.window_ids,
            state.applied_key.excluded_window_ids
        );
    }
}

fn plan_privacy_content_filter(
    requested_bundle_ids: &std::collections::BTreeSet<String>,
    requested_window_ids: &std::collections::BTreeSet<u32>,
    available_apps: &[PrivacyFilterAvailableApp],
    available_windows: &[PrivacyFilterAvailableWindow],
) -> PrivacyContentFilterPlan {
    let mut available_bundle_ids = std::collections::BTreeSet::new();
    for app in available_apps {
        available_bundle_ids.insert(app.bundle_id.clone());
    }
    for window in available_windows {
        if let Some(bundle_id) = window.owning_bundle_id.as_deref() {
            available_bundle_ids.insert(bundle_id.to_string());
        }
    }

    let excluded_bundle_ids = requested_bundle_ids
        .intersection(&available_bundle_ids)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let available_window_ids = available_windows
        .iter()
        .map(|window| window.id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut excluded_window_ids = requested_window_ids
        .intersection(&available_window_ids)
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let unresolved_bundle_ids = requested_bundle_ids
        .difference(&available_bundle_ids)
        .cloned()
        .collect();
    let unresolved_window_ids = requested_window_ids
        .difference(&excluded_window_ids)
        .copied()
        .collect();
    let has_bundle_exclusions = !excluded_bundle_ids.is_empty();
    let has_window_exclusions = !excluded_window_ids.is_empty();
    let strategy = if has_bundle_exclusions && has_window_exclusions {
        for window in available_windows {
            if window
                .owning_bundle_id
                .as_ref()
                .is_some_and(|bundle_id| excluded_bundle_ids.contains(bundle_id))
            {
                excluded_window_ids.insert(window.id);
            }
        }
        PrivacyContentFilterStrategy::ExcludingAppWindowsAndWindows
    } else if has_bundle_exclusions {
        PrivacyContentFilterStrategy::ExcludingApps
    } else {
        PrivacyContentFilterStrategy::ExcludingWindows
    };

    PrivacyContentFilterPlan {
        strategy,
        excluded_bundle_ids,
        excluded_window_ids,
        unresolved_bundle_ids,
        unresolved_window_ids,
    }
}

pub const SCREEN_SEGMENT_FRAME_INDEX_VERSION: u32 = 1;
const SCREEN_SEGMENT_FRAME_INDEX_MAGIC: &[u8; 4] = b"SFI1";
const SCREEN_SEGMENT_FRAME_INDEX_HEADER_LEN: usize = 12;
const SCREEN_SEGMENT_FRAME_INDEX_ENTRY_LEN: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSegmentFrameIndexEntry {
    pub captured_at_unix_ms: u64,
    pub frame_index: u64,
    pub video_offset_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSegmentFrameIndex {
    pub version: u32,
    pub entries: Vec<ScreenSegmentFrameIndexEntry>,
}

#[derive(Clone)]
pub struct ScreenFrameExportConfig {
    pub on_frame_exported: ScreenFrameArtifactHandler,
}

impl std::fmt::Debug for ScreenFrameExportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenFrameExportConfig")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScreenCaptureSessionOptions {
    pub frame_export: Option<ScreenFrameExportConfig>,
    pub system_audio_inactivity_tail_trim_seconds: u64,
    pub system_audio_writer_active: Option<bool>,
    pub initial_privacy_filter: Option<PrivacyContentFilter>,
}

#[cfg(target_os = "macos")]
fn system_audio_writer_active_for_options(
    sources: &ScreenCaptureSources,
    options: &ScreenCaptureSessionOptions,
) -> bool {
    sources.system_audio && options.system_audio_writer_active.unwrap_or(true)
}

fn even_dimension(value: u32) -> u32 {
    let at_least_two = value.max(2);
    if at_least_two % 2 == 0 {
        at_least_two
    } else {
        at_least_two + 1
    }
}

fn screen_frame_artifact_path(
    artifact_dir: &Path,
    frame_index: u64,
    captured_at_unix_ms: u64,
) -> PathBuf {
    artifact_dir.join(format!("frame-{captured_at_unix_ms}-{frame_index:06}.jpg"))
}

pub fn screen_segment_frame_index_path(video_path: &Path) -> PathBuf {
    let parent = video_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = video_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("segment");
    parent.join(format!("{stem}.frame-index.bin"))
}

pub fn legacy_screen_segment_frame_index_path(video_path: &Path) -> PathBuf {
    let parent = video_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = video_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("segment");
    parent.join(format!("{stem}.frame-index.json"))
}

pub fn encode_screen_segment_frame_index(index: &ScreenSegmentFrameIndex) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        SCREEN_SEGMENT_FRAME_INDEX_HEADER_LEN
            + index
                .entries
                .len()
                .saturating_mul(SCREEN_SEGMENT_FRAME_INDEX_ENTRY_LEN),
    );
    bytes.extend_from_slice(SCREEN_SEGMENT_FRAME_INDEX_MAGIC);
    bytes.extend_from_slice(&index.version.to_le_bytes());
    bytes.extend_from_slice(&(index.entries.len() as u32).to_le_bytes());
    for entry in &index.entries {
        bytes.extend_from_slice(&entry.captured_at_unix_ms.to_le_bytes());
        bytes.extend_from_slice(&entry.frame_index.to_le_bytes());
        bytes.extend_from_slice(&entry.video_offset_ms.to_le_bytes());
    }
    bytes
}

pub fn decode_screen_segment_frame_index(bytes: &[u8]) -> Result<ScreenSegmentFrameIndex, String> {
    if bytes.len() < SCREEN_SEGMENT_FRAME_INDEX_HEADER_LEN {
        return Err("frame index payload is too short".to_string());
    }
    if &bytes[0..4] != SCREEN_SEGMENT_FRAME_INDEX_MAGIC {
        return Err("frame index payload has invalid magic".to_string());
    }

    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("version bytes"));
    let count = u32::from_le_bytes(bytes[8..12].try_into().expect("count bytes")) as usize;
    let expected_len = SCREEN_SEGMENT_FRAME_INDEX_HEADER_LEN
        .checked_add(count.saturating_mul(SCREEN_SEGMENT_FRAME_INDEX_ENTRY_LEN))
        .ok_or_else(|| "frame index payload length overflowed".to_string())?;
    if bytes.len() != expected_len {
        return Err(format!(
            "frame index payload length {} did not match expected {}",
            bytes.len(),
            expected_len
        ));
    }

    let mut entries = Vec::with_capacity(count);
    let mut offset = SCREEN_SEGMENT_FRAME_INDEX_HEADER_LEN;
    while offset < bytes.len() {
        let captured_at_unix_ms = u64::from_le_bytes(
            bytes[offset..offset + 8]
                .try_into()
                .expect("captured_at bytes"),
        );
        let frame_index = u64::from_le_bytes(
            bytes[offset + 8..offset + 16]
                .try_into()
                .expect("frame_index bytes"),
        );
        let video_offset_ms = u64::from_le_bytes(
            bytes[offset + 16..offset + 24]
                .try_into()
                .expect("video_offset bytes"),
        );
        entries.push(ScreenSegmentFrameIndexEntry {
            captured_at_unix_ms,
            frame_index,
            video_offset_ms,
        });
        offset += SCREEN_SEGMENT_FRAME_INDEX_ENTRY_LEN;
    }

    Ok(ScreenSegmentFrameIndex { version, entries })
}

#[cfg(target_os = "macos")]
fn resolve_stream_resolution(
    requested: &ScreenResolution,
    display_width: u32,
    display_height: u32,
) -> ScreenCaptureResolution {
    let display_width = display_width.max(1);
    let display_height = display_height.max(1);

    let requested_height = match requested {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => {
                return ScreenCaptureResolution {
                    width: display_width,
                    height: display_height,
                }
            }
            ScreenResolutionPreset::P1080 => 1080,
            ScreenResolutionPreset::P720 => 720,
            ScreenResolutionPreset::P540 => 540,
        },
        ScreenResolution::Custom { width, height } => {
            return ScreenCaptureResolution {
                width: even_dimension(*width),
                height: even_dimension(*height),
            };
        }
    };

    if requested_height >= display_height {
        return ScreenCaptureResolution {
            width: display_width,
            height: display_height,
        };
    }

    let width = ((display_width as f64) * (requested_height as f64) / (display_height as f64))
        .round() as u32;

    ScreenCaptureResolution {
        width: even_dimension(width),
        height: even_dimension(requested_height),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureSupport {
    pub platform: String,
    pub native_capture_supported: bool,
    pub screen: bool,
    pub system_audio: bool,
}

fn output_files_for_session(
    session_dir: &Path,
    system_audio_output_path: Option<&Path>,
    sources: &ScreenCaptureSources,
) -> CaptureOutputFiles {
    let screen_file = sources
        .screen
        .then_some(session_dir.join("screen.mov").to_string_lossy().to_string());
    let system_audio_file = sources
        .system_audio
        .then(|| system_audio_output_path.map(|p| p.to_string_lossy().to_string()))
        .flatten();

    CaptureOutputFiles {
        screen_file: screen_file.clone(),
        screen_files: screen_file.into_iter().collect(),
        microphone_file: None,
        microphone_files: Vec::new(),
        system_audio_file: system_audio_file.clone(),
        system_audio_files: system_audio_file.into_iter().collect(),
    }
}

#[cfg(target_os = "macos")]
fn log_capture_error(context: &str, error: &CaptureErrorResponse) {
    capture_runtime::debug_log!(
        "[capture-screen] {context}: [{}] {}",
        error.code,
        error.message
    );
}

#[cfg(target_os = "macos")]
static SCREEN_PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static LAST_SCREEN_ACTIVITY_UNIX_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_SCREEN_ACTIVITY_MONOTONIC_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_SCREEN_ACTIVITY_FINGERPRINT: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_SYSTEM_AUDIO_ACTIVITY_UNIX_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_SYSTEM_AUDIO_ACTIVITY_MONOTONIC_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static LAST_SYSTEM_AUDIO_ACTIVITY_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "macos")]
static LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);
#[cfg(target_os = "macos")]
static LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_SAMPLE_COUNT: AtomicU32 = AtomicU32::new(0);

#[cfg(target_os = "macos")]
// Coalesce noisy per-frame screen samples without approaching the minimum
// supported inactivity timeout (1s), which would risk false inactivity pauses
// for low-FPS or jittery sessions.
const SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS: u64 = 250;
#[cfg(target_os = "macos")]
const SCREEN_ACTIVITY_FINGERPRINT_GRID_SIZE: usize = 8;
#[cfg(target_os = "macos")]
const SCREEN_ACTIVITY_FINGERPRINT_BYTES_PER_PROBE: usize = 4;
#[cfg(target_os = "macos")]
const SCREEN_ACTIVITY_TILE_SAMPLE_GRID_SIZE: usize = 3;
#[cfg(target_os = "macos")]
const SCREEN_ACTIVITY_TILE_QUANTIZATION_STEP: u64 = 32;
#[cfg(target_os = "macos")]
const SCREEN_ACTIVITY_FINGERPRINT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
#[cfg(target_os = "macos")]
const MAX_ACTIVE_DISPLAY_COUNT: u32 = 16;
#[cfg(target_os = "macos")]
const SCREEN_VIDEO_WRITER_FAILURE_PREFIX: &str = "screen video writer failed: ";
#[cfg(target_os = "macos")]
const SCREEN_STREAM_OUTPUT_PROCESSING_FAILURE_PREFIX: &str =
    "stream output failed: [capture_output_processing_failed] ";
#[cfg(target_os = "macos")]
const SCREEN_VIDEO_APPEND_SAMPLE_FAILURE_PREFIX: &str =
    "Failed to append screen video sample to asset writer: ";
#[cfg(target_os = "macos")]
const SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX: &str =
    "Failed to finalize screen video asset writer: ";
#[cfg(target_os = "macos")]
const AVFOUNDATION_FAILURE_CODE_11800_SUFFIX: &str = "(code: -11800)";
#[cfg(target_os = "macos")]
const SCREEN_SEGMENT_FINALIZE_FAILURE_PREFIXES: [&str; 3] = [
    "stream output failed: [",
    SCREEN_VIDEO_WRITER_FAILURE_PREFIX,
    "system audio writer failed: ",
];
#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
const FINALIZED_SCREEN_RECORDING_INSPECTION_ERROR_PREFIX: &str =
    "Failed to inspect finalized screen recording: ";
#[cfg(target_os = "macos")]
const FINALIZED_SCREEN_RECORDING_EMPTY_ERROR_MESSAGE: &str = "Finalized screen recording is empty";
#[cfg(target_os = "macos")]
const FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE: &str =
    "Finalized screen recording has no playable video track";
#[cfg(target_os = "macos")]
const FINALIZED_SCREEN_RECORDING_MISSING_FILE_MARKERS: [&str; 2] =
    ["No such file or directory", "os error 2"];

#[cfg(target_os = "macos")]
type CGDirectDisplayID = u32;
#[cfg(target_os = "macos")]
type CGImageRef = *const c_void;
#[cfg(target_os = "macos")]
type CGDataProviderRef = *const c_void;
#[cfg(target_os = "macos")]
type CFDataRef = *const c_void;

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayCreateImage(display: CGDirectDisplayID) -> CGImageRef;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

#[cfg(target_os = "macos")]
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn screen_activity_monotonic_epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

#[cfg(target_os = "macos")]
fn now_monotonic_ms() -> u64 {
    screen_activity_monotonic_epoch()
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(target_os = "macos")]
fn now_monotonic_marker_ms() -> u64 {
    // Reserve 0 as the "no sample observed" sentinel in the atomic state.
    now_monotonic_ms().saturating_add(1)
}

#[cfg(target_os = "macos")]
fn store_system_audio_activity(level: f32, now_monotonic_ms: u64, now_unix_ms: u64) {
    let level = level.clamp(0.0, 1.0);
    LAST_SYSTEM_AUDIO_ACTIVITY_LEVEL_BITS.store(level.to_bits(), Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_MONOTONIC_MS.store(now_monotonic_ms, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_UNIX_MS.store(now_unix_ms, Ordering::Relaxed);
    record_system_audio_activity_window_peak(level);
}

#[cfg(target_os = "macos")]
fn record_system_audio_activity_window_peak(level: f32) {
    LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_SAMPLE_COUNT.fetch_add(1, Ordering::Relaxed);

    let level_bits = level.to_bits();
    let mut observed_bits =
        LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    while f32::from_bits(observed_bits) < level {
        match LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.compare_exchange_weak(
            observed_bits,
            level_bits,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(next_bits) => observed_bits = next_bits,
        }
    }
}

#[cfg(target_os = "macos")]
fn maybe_mark_system_audio_activity_for_sample(sample_buf: &cidre::cm::SampleBuf) {
    if !sample_buf.data_is_ready() {
        return;
    }

    let level = match capture_writers::derive_audio_activity_level_from_sample_buf(sample_buf) {
        Some(l) => l,
        None => return,
    };
    store_system_audio_activity(level, now_monotonic_marker_ms(), now_unix_ms());
}

#[cfg(target_os = "macos")]
fn should_mark_screen_activity(last_activity_monotonic_ms: u64, now_monotonic_ms: u64) -> bool {
    last_activity_monotonic_ms == 0
        || now_monotonic_ms.saturating_sub(last_activity_monotonic_ms)
            >= SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS
}

#[cfg(target_os = "macos")]
fn mix_screen_activity_fingerprint(hash: &mut u64, value: u64) {
    *hash ^= value.wrapping_add(0x9E37_79B9_7F4A_7C15).rotate_left(25);
    *hash = hash.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
}

#[cfg(target_os = "macos")]
fn mix_screen_activity_fingerprint_bytes(hash: &mut u64, bytes: &[u8]) {
    let mut chunk = [0_u8; 8];
    chunk[..bytes.len()].copy_from_slice(bytes);
    mix_screen_activity_fingerprint(hash, u64::from_le_bytes(chunk) ^ (bytes.len() as u64));
}

#[cfg(target_os = "macos")]
fn finalize_screen_activity_fingerprint(mut hash: u64) -> u64 {
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    hash ^= hash >> 33;

    if hash == 0 {
        SCREEN_ACTIVITY_FINGERPRINT_SEED
    } else {
        hash
    }
}

#[cfg(target_os = "macos")]
fn screen_activity_attachment_value<'a>(
    sample_buf: &'a cidre::cm::SampleBuf,
    key: &cidre::sc::FrameInfo,
) -> Option<&'a cidre::cf::Type> {
    use cidre::cm;

    let mut attachment_mode = cm::AttachMode::Propagate;
    let key = key.as_type_ref().try_as_string()?;
    sample_buf.attach(key, &mut attachment_mode)
}

#[cfg(target_os = "macos")]
fn mix_screen_activity_attachment_fingerprint(
    hash: &mut u64,
    sample_buf: &cidre::cm::SampleBuf,
    key: &cidre::sc::FrameInfo,
) -> bool {
    use cidre::cf;

    let Some(value) = screen_activity_attachment_value(sample_buf, key) else {
        return false;
    };

    mix_screen_activity_fingerprint(hash, value.hash() as u64);

    if value.get_type_id() == cf::Array::type_id() {
        let array: &cf::Array = unsafe { std::mem::transmute(value) };
        mix_screen_activity_fingerprint(hash, array.len() as u64);
    }

    true
}

#[cfg(target_os = "macos")]
fn non_planar_screen_activity_sample_width(
    pixel_buf: &cidre::cv::PixelBuf,
    bytes_per_row: usize,
) -> usize {
    let estimated_bytes_per_pixel = match pixel_buf.pixel_format() {
        cidre::cv::PixelFormat::_32_BGRA
        | cidre::cv::PixelFormat::_32_ARGB
        | cidre::cv::PixelFormat::_32_ABGR
        | cidre::cv::PixelFormat::_32_RGBA
        | cidre::cv::PixelFormat::_30_RGB
        | cidre::cv::PixelFormat::_30_RGB_R210
        | cidre::cv::PixelFormat::ARGB_2101010_LE_PACKED => 4,
        cidre::cv::PixelFormat::_64_ARGB
        | cidre::cv::PixelFormat::_64_RGBALE
        | cidre::cv::PixelFormat::_64_RGBA_HALF => 8,
        cidre::cv::PixelFormat::_128_RGBA_FLOAT => 16,
        _ => 1,
    };

    bytes_per_row.min(pixel_buf.width().saturating_mul(estimated_bytes_per_pixel))
}

#[cfg(target_os = "macos")]
fn mix_screen_activity_pixel_probe_bytes(
    hash: &mut u64,
    base_address: *const u8,
    bytes_per_row: usize,
    sample_width: usize,
    height: usize,
    total_accessible_bytes: usize,
) -> bool {
    if base_address.is_null()
        || bytes_per_row == 0
        || sample_width == 0
        || height == 0
        || total_accessible_bytes == 0
    {
        return false;
    }

    let sample_width = sample_width.min(bytes_per_row);
    if sample_width == 0 {
        return false;
    }

    let tile_rows = height.min(SCREEN_ACTIVITY_FINGERPRINT_GRID_SIZE).max(1);
    let tile_cols = sample_width
        .div_ceil(SCREEN_ACTIVITY_FINGERPRINT_BYTES_PER_PROBE)
        .min(SCREEN_ACTIVITY_FINGERPRINT_GRID_SIZE)
        .max(1);
    let max_start_col = sample_width.saturating_sub(SCREEN_ACTIVITY_FINGERPRINT_BYTES_PER_PROBE);
    let mut sampled_any_probe = false;

    for tile_row in 0..tile_rows {
        let row = (((tile_row.saturating_mul(2)).saturating_add(1)).saturating_mul(height)
            / tile_rows.saturating_mul(2))
        .min(height.saturating_sub(1));

        for tile_col in 0..tile_cols {
            let desired_col = (((tile_col.saturating_mul(2)).saturating_add(1))
                .saturating_mul(sample_width)
                / tile_cols.saturating_mul(2))
            .saturating_sub(SCREEN_ACTIVITY_FINGERPRINT_BYTES_PER_PROBE / 2);
            let col = desired_col.min(max_start_col);
            let sample_len = (sample_width - col).min(SCREEN_ACTIVITY_FINGERPRINT_BYTES_PER_PROBE);
            let Some(sample_end_in_row) = col.checked_add(sample_len) else {
                continue;
            };
            if sample_end_in_row > bytes_per_row {
                continue;
            }

            let Some(sample_start) = row
                .checked_mul(bytes_per_row)
                .and_then(|row_offset| row_offset.checked_add(col))
            else {
                continue;
            };
            let Some(sample_end) = sample_start.checked_add(sample_len) else {
                continue;
            };
            if sample_end > total_accessible_bytes {
                continue;
            }

            let sample =
                unsafe { std::slice::from_raw_parts(base_address.add(sample_start), sample_len) };

            mix_screen_activity_fingerprint(hash, row as u64);
            mix_screen_activity_fingerprint(hash, col as u64);
            mix_screen_activity_fingerprint_bytes(hash, sample);
            sampled_any_probe = true;
        }
    }

    sampled_any_probe
}

#[cfg(target_os = "macos")]
fn screen_activity_pixel_fingerprint(sample_buf: &mut cidre::cm::SampleBuf) -> Option<u64> {
    let pixel_buf = sample_buf.image_buf_mut()?;
    let plane_count = pixel_buf.plane_count();
    let width = pixel_buf.width();
    let height = pixel_buf.height();
    let pixel_format = pixel_buf.pixel_format();
    let lock_flags = cidre::cv::pixel_buffer::LockFlags::READ_ONLY;

    unsafe {
        pixel_buf.lock_base_addr(lock_flags).result().ok()?;
    }

    let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;
    mix_screen_activity_fingerprint(&mut hash, width as u64);
    mix_screen_activity_fingerprint(&mut hash, height as u64);
    mix_screen_activity_fingerprint(&mut hash, pixel_format.0 as u64);
    mix_screen_activity_fingerprint(&mut hash, plane_count as u64);

    let mut sampled_any_plane = false;

    if plane_count == 0 {
        let bytes_per_row = unsafe { CVPixelBufferGetBytesPerRow(pixel_buf) };
        let sample_width = non_planar_screen_activity_sample_width(pixel_buf, bytes_per_row);
        let base_address = unsafe { CVPixelBufferGetBaseAddress(pixel_buf) } as *const u8;
        let total_accessible_bytes = unsafe { CVPixelBufferGetDataSize(pixel_buf) };

        sampled_any_plane = mix_screen_activity_pixel_probe_bytes(
            &mut hash,
            base_address,
            bytes_per_row,
            sample_width,
            height,
            total_accessible_bytes,
        );
    } else {
        for plane_index in 0..plane_count {
            let plane_bytes_per_row = pixel_buf.plane_bytes_per_row(plane_index);
            let plane_width = pixel_buf.plane_width(plane_index);
            let plane_height = pixel_buf.plane_height(plane_index);
            let plane_base_address = pixel_buf.plane_base_address(plane_index);
            let Some(plane_total_accessible_bytes) = plane_bytes_per_row.checked_mul(plane_height)
            else {
                continue;
            };

            mix_screen_activity_fingerprint(&mut hash, plane_index as u64);
            mix_screen_activity_fingerprint(&mut hash, plane_width as u64);
            mix_screen_activity_fingerprint(&mut hash, plane_height as u64);

            sampled_any_plane |= mix_screen_activity_pixel_probe_bytes(
                &mut hash,
                plane_base_address,
                plane_bytes_per_row,
                plane_bytes_per_row.min(plane_width),
                plane_height,
                plane_total_accessible_bytes,
            );
        }
    }

    let _ = unsafe { pixel_buf.unlock_lock_base_addr(lock_flags).result() };

    sampled_any_plane.then(|| finalize_screen_activity_fingerprint(hash))
}

#[cfg(target_os = "macos")]
fn screen_activity_sample_fingerprint(sample_buf: &mut cidre::cm::SampleBuf) -> Option<u64> {
    let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;
    let mut has_content_signal = false;

    // `dirty_rects` and `bounding_rect` churn on cursor movement and other tiny
    // overlays. Keep only the stable geometry attachments in the exported frame
    // fingerprint so cursor flicker does not look like new content.
    for key in [
        cidre::sc::FrameInfo::content_rect(),
        cidre::sc::FrameInfo::screen_rect(),
    ] {
        has_content_signal |=
            mix_screen_activity_attachment_fingerprint(&mut hash, sample_buf, key);
    }

    if let Some(pixel_hash) = screen_activity_pixel_fingerprint(sample_buf) {
        mix_screen_activity_fingerprint(&mut hash, pixel_hash);
        has_content_signal = true;
    }

    has_content_signal.then(|| finalize_screen_activity_fingerprint(hash))
}

#[cfg(target_os = "macos")]
fn screen_activity_sample_captured_frame_equivalence(
    sample_buf: &mut cidre::cm::SampleBuf,
) -> Option<CapturedFrameEquivalenceOutcome> {
    let pixel_buf = sample_buf.image_buf_mut()?;
    let plane_count = pixel_buf.plane_count();
    let pixel_width = pixel_buf.width();
    if plane_count != 0 {
        return None;
    }

    let height = pixel_buf.height();
    let bytes_per_row = unsafe { CVPixelBufferGetBytesPerRow(pixel_buf) };
    let pixel_format = pixel_buf.pixel_format();
    let lock_flags = cidre::cv::pixel_buffer::LockFlags::READ_ONLY;

    if bytes_per_row == 0 || pixel_width == 0 || height == 0 {
        return None;
    }

    unsafe {
        if pixel_buf.lock_base_addr(lock_flags).result().is_err() {
            return None;
        }
    }

    let base_address = unsafe { CVPixelBufferGetBaseAddress(pixel_buf) } as *const u8;
    let total_accessible_bytes = unsafe { CVPixelBufferGetDataSize(pixel_buf) };
    if base_address.is_null() || total_accessible_bytes == 0 {
        let _ = unsafe { pixel_buf.unlock_lock_base_addr(lock_flags).result() };
        return None;
    }

    let total_bytes = bytes_per_row.checked_mul(height)?;
    let accessible_bytes = total_accessible_bytes.min(total_bytes);
    let bytes = unsafe { std::slice::from_raw_parts(base_address, accessible_bytes) };

    let equivalence = non_planar_screen_activity_captured_frame_equivalence(
        bytes,
        bytes_per_row,
        pixel_width,
        height,
        pixel_format,
    );

    let _ = unsafe { pixel_buf.unlock_lock_base_addr(lock_flags).result() };
    equivalence
}

#[cfg(target_os = "macos")]
fn non_planar_screen_activity_captured_frame_equivalence(
    bytes: &[u8],
    bytes_per_row: usize,
    pixel_width: usize,
    height: usize,
    pixel_format: cidre::cv::PixelFormat,
) -> Option<CapturedFrameEquivalenceOutcome> {
    if bytes.is_empty() || bytes_per_row == 0 || pixel_width == 0 || height == 0 {
        return None;
    }

    let channel_order = match pixel_format {
        cidre::cv::PixelFormat::_32_BGRA => [2, 1, 0, 3],
        cidre::cv::PixelFormat::_32_RGBA => [0, 1, 2, 3],
        cidre::cv::PixelFormat::_32_ARGB => [1, 2, 3, 0],
        cidre::cv::PixelFormat::_32_ABGR => [3, 2, 1, 0],
        _ => return None,
    };

    captured_frame_equivalence_from_interleaved_bytes(
        bytes,
        bytes_per_row,
        pixel_width,
        height,
        channel_order,
    )
    .map(CapturedFrameEquivalenceOutcome::ready)
}

#[cfg(target_os = "macos")]
fn should_mark_screen_activity_for_fingerprint(
    last_activity_fingerprint: u64,
    fingerprint: Option<u64>,
) -> bool {
    match fingerprint {
        None => false,
        Some(fingerprint) => {
            last_activity_fingerprint == 0 || last_activity_fingerprint != fingerprint
        }
    }
}

#[cfg(target_os = "macos")]
fn mark_screen_activity(now_monotonic_ms: u64, now_unix_ms: u64) -> bool {
    let mut last_activity_monotonic_ms = LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);

    loop {
        if !should_mark_screen_activity(last_activity_monotonic_ms, now_monotonic_ms) {
            return false;
        }

        match LAST_SCREEN_ACTIVITY_MONOTONIC_MS.compare_exchange_weak(
            last_activity_monotonic_ms,
            now_monotonic_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                LAST_SCREEN_ACTIVITY_UNIX_MS.fetch_max(now_unix_ms, Ordering::Relaxed);
                return true;
            }
            Err(current) => last_activity_monotonic_ms = current,
        }
    }
}

#[cfg(target_os = "macos")]
fn maybe_mark_screen_activity_for_fingerprint(fingerprint: Option<u64>) -> bool {
    let last_activity_fingerprint = LAST_SCREEN_ACTIVITY_FINGERPRINT.load(Ordering::Relaxed);

    if !should_mark_screen_activity_for_fingerprint(last_activity_fingerprint, fingerprint) {
        return false;
    }

    if mark_screen_activity(now_monotonic_marker_ms(), now_unix_ms()) {
        if let Some(fingerprint) = fingerprint {
            LAST_SCREEN_ACTIVITY_FINGERPRINT.store(fingerprint, Ordering::Relaxed);
        }

        return true;
    }

    false
}

#[cfg(target_os = "macos")]
fn screen_activity_bitmap_fingerprint(
    bytes: &[u8],
    bytes_per_row: usize,
    width: usize,
    height: usize,
) -> Option<u64> {
    if bytes.is_empty() || bytes_per_row == 0 || width == 0 || height == 0 {
        return None;
    }

    let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;
    let tile_rows = height.min(SCREEN_ACTIVITY_FINGERPRINT_GRID_SIZE).max(1);
    let tile_cols = width.min(SCREEN_ACTIVITY_FINGERPRINT_GRID_SIZE).max(1);
    let mut has_content_signal = false;

    for tile_row in 0..tile_rows {
        let row_start = tile_row.saturating_mul(height) / tile_rows;
        let row_end = ((tile_row.saturating_add(1)).saturating_mul(height) / tile_rows)
            .max(row_start.saturating_add(1))
            .min(height);

        for tile_col in 0..tile_cols {
            let col_start = tile_col.saturating_mul(width) / tile_cols;
            let col_end = ((tile_col.saturating_add(1)).saturating_mul(width) / tile_cols)
                .max(col_start.saturating_add(1))
                .min(width);

            let Some(value) = screen_activity_bitmap_tile_summary(
                bytes,
                bytes_per_row,
                width,
                height,
                row_start,
                row_end,
                col_start,
                col_end,
            ) else {
                continue;
            };

            has_content_signal |= value != 0;
            mix_screen_activity_fingerprint(&mut hash, row_start as u64);
            mix_screen_activity_fingerprint(&mut hash, col_start as u64);
            mix_screen_activity_fingerprint(&mut hash, value);
        }
    }

    has_content_signal.then(|| finalize_screen_activity_fingerprint(hash))
}

#[cfg(target_os = "macos")]
fn screen_activity_bitmap_tile_summary(
    bytes: &[u8],
    bytes_per_row: usize,
    width: usize,
    height: usize,
    row_start: usize,
    row_end: usize,
    col_start: usize,
    col_end: usize,
) -> Option<u64> {
    if row_start >= row_end || col_start >= col_end || width == 0 || height == 0 {
        return None;
    }

    let sample_rows = (row_end - row_start)
        .min(SCREEN_ACTIVITY_TILE_SAMPLE_GRID_SIZE)
        .max(1);
    let sample_cols = (col_end - col_start)
        .min(SCREEN_ACTIVITY_TILE_SAMPLE_GRID_SIZE)
        .max(1);
    let mut sum_r = 0_u64;
    let mut sum_g = 0_u64;
    let mut sum_b = 0_u64;
    let mut sum_a = 0_u64;
    let mut sample_count = 0_u64;

    for sample_row in 0..sample_rows {
        let row = row_start
            + (((sample_row.saturating_mul(2)).saturating_add(1))
                .saturating_mul(row_end - row_start)
                / sample_rows.saturating_mul(2))
            .min(row_end - row_start - 1);

        for sample_col in 0..sample_cols {
            let col = col_start
                + (((sample_col.saturating_mul(2)).saturating_add(1))
                    .saturating_mul(col_end - col_start)
                    / sample_cols.saturating_mul(2))
                .min(col_end - col_start - 1);
            let pixel_offset = row
                .checked_mul(bytes_per_row)?
                .checked_add(col.checked_mul(4)?)?;
            let pixel = bytes.get(pixel_offset..pixel_offset + 4)?;

            sum_b += pixel[0] as u64;
            sum_g += pixel[1] as u64;
            sum_r += pixel[2] as u64;
            sum_a += pixel[3] as u64;
            sample_count += 1;
        }
    }

    if sample_count == 0 {
        return None;
    }

    let avg_b = (sum_b / sample_count) / SCREEN_ACTIVITY_TILE_QUANTIZATION_STEP;
    let avg_g = (sum_g / sample_count) / SCREEN_ACTIVITY_TILE_QUANTIZATION_STEP;
    let avg_r = (sum_r / sample_count) / SCREEN_ACTIVITY_TILE_QUANTIZATION_STEP;
    let avg_a = (sum_a / sample_count) / SCREEN_ACTIVITY_TILE_QUANTIZATION_STEP;

    Some(avg_b | (avg_g << 8) | (avg_r << 16) | (avg_a << 24))
}

#[cfg(all(target_os = "macos", test))]
fn screen_frame_content_bitmap_fingerprint(
    bytes: &[u8],
    bytes_per_row: usize,
    width: usize,
    height: usize,
) -> Option<u64> {
    screen_activity_bitmap_fingerprint(bytes, bytes_per_row, width, height)
}

#[cfg(target_os = "macos")]
fn screen_activity_display_fingerprint(display_id: CGDirectDisplayID) -> Option<u64> {
    let image = unsafe { CGDisplayCreateImage(display_id) };
    if image.is_null() {
        return None;
    }

    let fingerprint = (|| {
        let width = unsafe { CGImageGetWidth(image) };
        let height = unsafe { CGImageGetHeight(image) };
        let bytes_per_row = unsafe { CGImageGetBytesPerRow(image) };
        let provider = unsafe { CGImageGetDataProvider(image) };
        if provider.is_null() {
            return None;
        }

        let data = unsafe { CGDataProviderCopyData(provider) };
        if data.is_null() {
            return None;
        }

        let fingerprint = unsafe {
            let length = CFDataGetLength(data);
            let bytes = CFDataGetBytePtr(data);
            if bytes.is_null() || length <= 0 {
                None
            } else {
                let slice = std::slice::from_raw_parts(bytes, length as usize);
                screen_activity_bitmap_fingerprint(slice, bytes_per_row, width, height)
            }
        };

        unsafe { CFRelease(data) };
        fingerprint
    })();

    unsafe { CFRelease(image) };
    fingerprint
}

#[cfg(target_os = "macos")]
fn polled_screen_activity_fingerprint() -> Option<u64> {
    let mut display_ids = [0; MAX_ACTIVE_DISPLAY_COUNT as usize];
    let mut display_count = 0;
    let status = unsafe {
        CGGetActiveDisplayList(
            MAX_ACTIVE_DISPLAY_COUNT,
            display_ids.as_mut_ptr(),
            &mut display_count,
        )
    };
    if status != 0 || display_count == 0 {
        return None;
    }

    let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;
    let mut has_content_signal = false;

    for display_id in display_ids.into_iter().take(display_count as usize) {
        let Some(fingerprint) = screen_activity_display_fingerprint(display_id) else {
            continue;
        };

        has_content_signal = true;
        mix_screen_activity_fingerprint(&mut hash, fingerprint);
    }

    has_content_signal.then(|| finalize_screen_activity_fingerprint(hash))
}

#[cfg(target_os = "macos")]
pub fn poll_screen_activity() -> bool {
    maybe_mark_screen_activity_for_fingerprint(polled_screen_activity_fingerprint())
}

#[cfg(not(target_os = "macos"))]
pub fn poll_screen_activity() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn reset_last_screen_activity_unix_ms() {
    LAST_SCREEN_ACTIVITY_UNIX_MS.store(0, Ordering::Relaxed);
    LAST_SCREEN_ACTIVITY_MONOTONIC_MS.store(0, Ordering::Relaxed);
    LAST_SCREEN_ACTIVITY_FINGERPRINT.store(0, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_UNIX_MS.store(0, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_MONOTONIC_MS.store(0, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_LEVEL_BITS.store(0, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.store(0, Ordering::Relaxed);
    LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_SAMPLE_COUNT.store(0, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn reset_last_screen_activity_unix_ms() {}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct StreamOutputContext {
    screen_video_output_file: Option<String>,
    screen_video_writer: Option<VideoAssetWriterState>,
    video_bitrate_bps: Option<u32>,
    system_audio_output_file: Option<String>,
    system_audio_writer: Option<AudioAssetWriterState>,
    system_audio_tail_trim_seconds: u64,
    system_audio_inactivity_tail_buffer_seconds: u64,
    frame_export: Option<ScreenFrameExportRuntime>,
    first_error: Option<CaptureErrorResponse>,
}

#[cfg(target_os = "macos")]
struct ScreenFrameExportRuntime {
    artifact_dir: PathBuf,
    callback_queue: cidre::arc::R<dispatch::Queue>,
    on_frame_exported: ScreenFrameArtifactHandler,
    first_error: Arc<Mutex<Option<CaptureErrorResponse>>>,
    segment_frame_index: Arc<Mutex<ScreenSegmentFrameIndexState>>,
    next_frame_index: u64,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Default)]
struct ScreenSegmentFrameIndexState {
    entries: Vec<ScreenSegmentFrameIndexEntry>,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for ScreenFrameExportRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenFrameExportRuntime")
            .field("artifact_dir", &self.artifact_dir)
            .field("next_frame_index", &self.next_frame_index)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "macos")]
fn push_screen_segment_frame_index_entry(
    state: &Arc<Mutex<ScreenSegmentFrameIndexState>>,
    entry: ScreenSegmentFrameIndexEntry,
) {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .push(entry);
}

#[cfg(target_os = "macos")]
fn persist_screen_segment_frame_index(
    screen_video_output_file: &str,
    frame_export: &ScreenFrameExportRuntime,
) -> Result<(), CaptureErrorResponse> {
    let entries = frame_export
        .segment_frame_index
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .clone();
    if entries.is_empty() {
        return Ok(());
    }

    let index = finalized_screen_segment_frame_index(screen_video_output_file, &entries)?;
    let index_path = screen_segment_frame_index_path(Path::new(screen_video_output_file));
    let bytes = encode_screen_segment_frame_index(&index);
    std::fs::write(&index_path, bytes).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!(
            "Failed to write screen segment frame index {}: {error}",
            index_path.display()
        ),
    })
}

#[cfg(target_os = "macos")]
fn store_first_frame_export_error(
    cell: &Arc<Mutex<Option<CaptureErrorResponse>>>,
    error: CaptureErrorResponse,
) {
    let mut guard = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_none() {
        *guard = Some(error);
    }
}

#[cfg(target_os = "macos")]
fn take_frame_export_error(
    cell: &Arc<Mutex<Option<CaptureErrorResponse>>>,
) -> Option<CaptureErrorResponse> {
    cell.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

#[cfg(target_os = "macos")]
fn store_first_stream_output_error(
    first_error: &mut Option<CaptureErrorResponse>,
    error: CaptureErrorResponse,
) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(target_os = "macos")]
fn sample_time_to_ms(time: cidre::cm::Time) -> Option<u64> {
    if !time.is_numeric() || time.scale <= 0 {
        return None;
    }

    let value_ms = i128::from(time.value)
        .checked_mul(1_000)?
        .checked_add(i128::from(time.scale / 2))?
        / i128::from(time.scale);

    u64::try_from(value_ms).ok()
}

#[cfg(target_os = "macos")]
fn finalized_screen_segment_frame_index(
    screen_video_output_file: &str,
    entries: &[ScreenSegmentFrameIndexEntry],
) -> Result<ScreenSegmentFrameIndex, CaptureErrorResponse> {
    if entries.is_empty() {
        return Ok(ScreenSegmentFrameIndex {
            version: SCREEN_SEGMENT_FRAME_INDEX_VERSION,
            entries: Vec::new(),
        });
    }

    let result = {
        let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        use cidre::{av, cv, ns};

        let video_url = ns::Url::with_fs_path_str(screen_video_output_file, false);
        let asset = av::UrlAsset::with_url(&video_url, None).ok_or_else(|| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to open finalized screen recording for frame index extraction: {screen_video_output_file}"
            ),
        })?;
        let video_tracks = load_asset_tracks_with_timeout(
            asset.as_ref(),
            av::MediaType::video(),
            "capture_output_processing_failed",
            "Timed out while loading finalized screen recording video track",
        )?;
        let video_track = video_tracks.first().ok_or_else(|| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Finalized screen recording has no video track for frame index extraction: {screen_video_output_file}"
            ),
        })?;

        let mut reader =
            av::AssetReader::with_asset(asset.as_ref()).map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to create asset reader for finalized screen frame index extraction: {screen_video_output_file}"
                ),
            })?;
        let output_settings = ns::Dictionary::with_keys_values(
            &[cv::pixel_buffer::keys::pixel_format().as_ns()],
            &[cv::PixelFormat::_32_BGRA.to_ns_number().as_id_ref()],
        );
        let mut reader_output = av::AssetReaderTrackOutput::with_track(video_track, Some(&output_settings))
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to create video track reader output for finalized frame index extraction: {screen_video_output_file}"
                ),
            })?;
        reader_output.set_always_copies_sample_data(false);

        let reader_output_ref: &av::AssetReaderOutput =
            unsafe { &*(&*reader_output as *const _ as *const av::AssetReaderOutput) };
        if !reader.can_add_output(reader_output_ref) {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to add video reader output for finalized frame index extraction: {screen_video_output_file}"
                ),
            });
        }
        reader
            .add_output(reader_output_ref)
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to attach video reader output for finalized frame index extraction: {screen_video_output_file}"
                ),
            })?;

        let started = reader.start_reading().map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "Failed to start reading finalized screen recording for frame index extraction: {screen_video_output_file}"
            ),
        })?;
        if !started {
            if let Some(error) = reader.error() {
                return Err(error_with_ns_error(
                    "capture_output_processing_failed",
                    "Failed to start reading finalized screen recording for frame index extraction",
                    error.as_ref(),
                ));
            }
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to start reading finalized screen recording for frame index extraction: {screen_video_output_file}"
                ),
            });
        }

        let mut finalized_entries = Vec::with_capacity(entries.len());
        let mut entry_iter = entries.iter();
        let mut next_entry = entry_iter.next();
        let mut first_sample_offset_ms: Option<u64> = None;

        while let Some(entry) = next_entry {
            let sample_buf = reader_output
                .next_sample_buf()
                .map_err(|_| CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Failed to read video sample while building finalized frame index: {screen_video_output_file}"
                    ),
                })?;
            let Some(sample_buf) = sample_buf else {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Finalized screen recording ended before frame index extraction consumed all indexed frames: {screen_video_output_file}"
                    ),
                });
            };

            let sample_offset_ms = sample_time_to_ms(sample_buf.pts()).ok_or_else(|| {
                CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Finalized screen recording sample had non-numeric PTS during frame index extraction: {screen_video_output_file}"
                    ),
                }
            })?;
            let first_sample_offset_ms = *first_sample_offset_ms.get_or_insert(sample_offset_ms);
            finalized_entries.push(ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: entry.captured_at_unix_ms,
                frame_index: entry.frame_index,
                video_offset_ms: sample_offset_ms.saturating_sub(first_sample_offset_ms),
            });
            next_entry = entry_iter.next();
        }

        match reader.status() {
            cidre::av::asset::ReaderStatus::Completed | cidre::av::asset::ReaderStatus::Reading => {
            }
            cidre::av::asset::ReaderStatus::Failed => {
                if let Some(error) = reader.error() {
                    return Err(error_with_ns_error(
                        "capture_output_processing_failed",
                        "Finalized screen frame index reader failed",
                        error.as_ref(),
                    ));
                }
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Finalized screen frame index reader failed".to_string(),
                });
            }
            status => {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Finalized screen frame index reader ended unexpectedly (status: {:?})",
                        status
                    ),
                });
            }
        }

        if !screen_segment_frame_index_offsets_are_monotonic(&finalized_entries) {
            capture_runtime::debug_log!(
                "[capture-screen] finalized screen frame index offsets regressed for {}",
                screen_video_output_file
            );
        }

        let finalized_index = ScreenSegmentFrameIndex {
            version: SCREEN_SEGMENT_FRAME_INDEX_VERSION,
            entries: finalized_entries,
        };
        reader.cancel_reading();
        Ok(finalized_index)
    };

    result
}

pub fn screen_segment_frame_index_offsets_are_monotonic(
    entries: &[ScreenSegmentFrameIndexEntry],
) -> bool {
    entries
        .windows(2)
        .all(|pair| pair[0].video_offset_ms <= pair[1].video_offset_ms)
}

#[cfg(target_os = "macos")]
pub fn rebuild_screen_segment_frame_index_from_video(
    video_path: &Path,
    entries: &[ScreenSegmentFrameIndexEntry],
) -> Result<ScreenSegmentFrameIndex, String> {
    finalized_screen_segment_frame_index(&video_path.to_string_lossy(), entries).map_err(|error| {
        format!(
            "failed to rebuild screen segment frame index from {}: [{}] {}",
            video_path.display(),
            error.code,
            error.message
        )
    })
}

#[cfg(not(target_os = "macos"))]
pub fn rebuild_screen_segment_frame_index_from_video(
    video_path: &Path,
    _entries: &[ScreenSegmentFrameIndexEntry],
) -> Result<ScreenSegmentFrameIndex, String> {
    Err(format!(
        "rebuilding screen segment frame index from {} is only supported on macOS",
        video_path.display()
    ))
}

#[cfg(target_os = "macos")]
fn stream_output_callback_panic_error(
    payload: Box<dyn std::any::Any + Send>,
) -> CaptureErrorResponse {
    let message = if let Some(message) = payload.downcast_ref::<&'static str>() {
        format!("ScreenCaptureKit output callback panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("ScreenCaptureKit output callback panicked: {message}")
    } else {
        "ScreenCaptureKit output callback panicked with a non-string payload".to_string()
    };

    let error = CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message,
    };

    log_capture_error(
        "panic boundary captured in ScreenCaptureKit output callback",
        &error,
    );

    error
}

#[cfg(target_os = "macos")]
fn stream_output_callback_objc_exception_error(
    exception: &cidre::ns::Exception,
) -> CaptureErrorResponse {
    let name_ref = exception.name();
    // ExceptionName is a newtype over ns::String; deref twice to reach &ns::String.
    let name = fmt_ns(&**name_ref);
    let reason = exception
        .reason()
        .map(|r| fmt_ns(r.as_ref()))
        .unwrap_or_else(|| "unknown reason".to_string());

    let error = CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("ScreenCaptureKit output callback ObjC exception: {name} - {reason}"),
    };

    log_capture_error(
        "ObjC exception boundary captured in ScreenCaptureKit output callback",
        &error,
    );

    error
}

#[cfg(all(target_os = "macos", test))]
fn run_nested_objc_exception_and_panic_boundary_for_test() {
    let reason = cidre::ns::str!(c"nested test objc exception");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = cidre::ns::try_catch(|| {
            cidre::ns::Exception::raise(reason);
        });
    }));
}

#[cfg(target_os = "macos")]
struct PreparedScreenFrameExport {
    frame_index: u64,
    file_path: PathBuf,
    captured_at_unix_ms: u64,
    width: Option<u32>,
    height: Option<u32>,
    captured_frame_equivalence: CapturedFrameEquivalenceOutcome,
}

#[cfg(target_os = "macos")]
fn prepare_screen_frame_export(
    runtime: &mut ScreenFrameExportRuntime,
    sample_buf: &cidre::cm::SampleBuf,
    captured_frame_equivalence: CapturedFrameEquivalenceOutcome,
) -> PreparedScreenFrameExport {
    let captured_at_unix_ms = now_unix_ms();
    let frame_index = runtime.next_frame_index;
    runtime.next_frame_index = runtime.next_frame_index.saturating_add(1);

    let (width, height) = sample_buf
        .image_buf()
        .map(|image_buf| {
            let pixel_buf: &cidre::cv::PixelBuf = image_buf;
            (pixel_buf.width() as u32, pixel_buf.height() as u32)
        })
        .map(|(width, height)| (Some(width), Some(height)))
        .unwrap_or((None, None));

    PreparedScreenFrameExport {
        frame_index,
        file_path: screen_frame_artifact_path(
            &runtime.artifact_dir,
            frame_index,
            captured_at_unix_ms,
        ),
        captured_at_unix_ms,
        width,
        height,
        captured_frame_equivalence,
    }
}

#[cfg(target_os = "macos")]
fn save_screen_sample_as_jpeg(
    sample_buf: &cidre::cm::SampleBuf,
    output_path: &Path,
) -> Result<(), CaptureErrorResponse> {
    let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
    {
        use cidre::{cg, ut, vt};

        let image_buf = sample_buf.image_buf().ok_or_else(|| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Screen frame sample did not contain an image buffer".to_string(),
        })?;
        let pixel_buf: &cidre::cv::PixelBuf = image_buf;
        let cg_image = vt::cg_image_from_cv_pixel_buf(pixel_buf, None).map_err(|status| {
            CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to create CGImage from screen frame sample (status: {:?})",
                    status
                ),
            }
        })?;

        let output_url =
            cidre::cf::Url::with_file_path(output_path).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to create output URL for screen frame artifact: {}",
                    output_path.display()
                ),
            })?;

        let jpeg_type_id = ut::Type::jpeg().id();

        let mut destination = cg::ImageDst::with_url(output_url.as_ref(), jpeg_type_id.as_cf(), 1)
            .ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: image_destination_creation_failure_message(output_path, "JPEG"),
            })?;
        destination.add_image(cg_image.as_ref(), None);

        if destination.finalize() {
            Ok(())
        } else {
            Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: format!(
                    "Failed to finalize JPEG screen frame artifact: {}",
                    output_path.display()
                ),
            })
        }
    }
}

#[cfg(target_os = "macos")]
fn image_destination_creation_failure_message(output_path: &Path, format_name: &str) -> String {
    let parent_dir = output_path.parent();
    let parent_exists = parent_dir.is_some_and(|parent| parent.exists());
    let file_exists = output_path.exists();

    format!(
        "Failed to create {format_name} destination for screen frame artifact: {} (parent: {}; parent_exists: {}; file_exists: {})",
        output_path.display(),
        parent_dir
            .map(|parent| parent.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        parent_exists,
        file_exists
    )
}

#[cfg(target_os = "macos")]
fn export_screen_frame_artifact(
    runtime: &mut ScreenFrameExportRuntime,
    sample_buf: cidre::arc::R<cidre::cm::SampleBuf>,
    captured_frame_equivalence: CapturedFrameEquivalenceOutcome,
) -> Result<(), CaptureErrorResponse> {
    let prepared =
        prepare_screen_frame_export(runtime, sample_buf.as_ref(), captured_frame_equivalence);
    let callback_queue = runtime.callback_queue.retained();
    let on_frame_exported = runtime.on_frame_exported.clone();
    let first_error = runtime.first_error.clone();
    let segment_frame_index = runtime.segment_frame_index.clone();
    let file_path = prepared.file_path.clone();
    let pending_index_entry = Some(ScreenSegmentFrameIndexEntry {
        captured_at_unix_ms: prepared.captured_at_unix_ms,
        frame_index: prepared.frame_index,
        video_offset_ms: 0,
    });

    callback_queue.async_once(move || {
        if let Err(error) = save_screen_sample_as_jpeg(sample_buf.as_ref(), &file_path) {
            store_first_frame_export_error(&first_error, error.clone());
            capture_runtime::debug_log!(
                "[capture-screen] failed to export screen frame artifact {}: [{}] {}",
                file_path.display(),
                error.code,
                error.message
            );
            return;
        }

        if let Some(entry) = pending_index_entry {
            push_screen_segment_frame_index_entry(&segment_frame_index, entry);
        }

        let captured_frame_equivalence = match prepared.captured_frame_equivalence {
            CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
                CapturedFrameEquivalenceOutcome::Ready(equivalence)
            }
            CapturedFrameEquivalenceOutcome::Quarantined(error) => {
                match captured_frame_equivalence_from_image_path(&file_path) {
                    CapturedFrameEquivalenceOutcome::Ready(equivalence) => {
                        capture_runtime::debug_log!(
                            "[capture-screen] recovered captured frame equivalence from exported artifact {} after live sample failure: {}",
                            file_path.display(),
                            error
                        );
                        CapturedFrameEquivalenceOutcome::Ready(equivalence)
                    }
                    CapturedFrameEquivalenceOutcome::Quarantined(fallback_error) => {
                        CapturedFrameEquivalenceOutcome::Quarantined(format!(
                            "{error}; fallback from exported artifact failed: {fallback_error}"
                        ))
                    }
                }
            }
        };

        (on_frame_exported)(ScreenFrameArtifact {
            file_path: file_path.to_string_lossy().to_string(),
            captured_at_unix_ms: prepared.captured_at_unix_ms,
            width: prepared.width,
            height: prepared.height,
            captured_frame_equivalence,
        });
    });

    Ok(())
}

#[cfg(target_os = "macos")]
fn screen_frame_export_runtime(
    session_dir: &Path,
    config: Option<ScreenFrameExportConfig>,
) -> Result<Option<ScreenFrameExportRuntime>, CaptureErrorResponse> {
    let Some(config) = config else {
        return Ok(None);
    };

    let artifact_dir = session_dir.join("frames");
    std::fs::create_dir_all(&artifact_dir).map_err(|error| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create screen frame artifact directory: {error}"),
    })?;

    Ok(Some(ScreenFrameExportRuntime {
        artifact_dir,
        callback_queue: dispatch::Queue::serial_with_ar_pool(),
        on_frame_exported: config.on_frame_exported,
        first_error: Arc::new(Mutex::new(None)),
        segment_frame_index: Arc::new(Mutex::new(ScreenSegmentFrameIndexState::default())),
        next_frame_index: 0,
    }))
}

#[cfg(target_os = "macos")]
mod stream_output_delegate {
    #![allow(clippy::useless_transmute)]

    use super::{
        append_audio_sample_to_writer, append_video_sample_to_writer,
        create_video_asset_writer_for_sample_buf, export_screen_frame_artifact, objc,
        store_first_stream_output_error, stream_output_callback_objc_exception_error,
        stream_output_callback_panic_error, StreamOutputContext,
    };
    use cidre::ns;
    use cidre::sc::StreamOutput;

    cidre::define_obj_type!(
        pub(super) ScStreamOutputDelegate + cidre::sc::StreamOutputImpl,
        StreamOutputContext,
        ZScStreamOutputDelegate
    );

    impl cidre::sc::StreamOutput for ScStreamOutputDelegate {}

    fn handle_stream_did_output_sample_buf(
        ctx: &mut StreamOutputContext,
        sample_buf: &mut cidre::cm::SampleBuf,
        kind: cidre::sc::OutputType,
    ) {
        let append_result = match kind {
            cidre::sc::OutputType::Screen => {
                if !super::should_append_screen_sample(sample_buf) {
                    return;
                }
                let screen_activity_fingerprint =
                    super::screen_activity_sample_fingerprint(sample_buf);
                let _ =
                    super::maybe_mark_screen_activity_for_fingerprint(screen_activity_fingerprint);
                let captured_frame_equivalence =
                    super::screen_activity_sample_captured_frame_equivalence(sample_buf)
                        .unwrap_or_else(|| {
                            super::CapturedFrameEquivalenceOutcome::quarantined(
                                "failed to derive captured frame equivalence from screen sample",
                            )
                        });

                if ctx.screen_video_writer.is_none() {
                    let Some(output_file) = ctx.screen_video_output_file.as_deref() else {
                        return;
                    };
                    let output_url = ns::Url::with_fs_path_str(output_file, false);
                    match create_video_asset_writer_for_sample_buf(
                        &output_url,
                        "screen",
                        sample_buf,
                        ctx.video_bitrate_bps,
                    ) {
                        Ok(writer) => ctx.screen_video_writer = Some(writer),
                        Err(error) => {
                            store_first_stream_output_error(&mut ctx.first_error, error);
                            return;
                        }
                    }
                }

                match ctx.screen_video_writer.as_mut() {
                    Some(writer) => match append_video_sample_to_writer(writer, sample_buf) {
                        Ok(appended) => {
                            if appended {
                                if let Some(frame_export) = ctx.frame_export.as_mut() {
                                    if let Err(error) = export_screen_frame_artifact(
                                        frame_export,
                                        sample_buf.retained(),
                                        captured_frame_equivalence,
                                    ) {
                                        store_first_stream_output_error(
                                            &mut ctx.first_error,
                                            error,
                                        );
                                    }
                                }
                            }
                            None
                        }
                        Err(error) => Some(Err(error)),
                    },
                    None => None,
                }
            }
            cidre::sc::OutputType::Audio => {
                super::maybe_mark_system_audio_activity_for_sample(sample_buf);
                ctx.system_audio_writer
                    .as_mut()
                    .map(|writer| append_audio_sample_to_writer(writer, sample_buf))
            }
            cidre::sc::OutputType::Mic => None,
        };

        if let Some(Err(error)) = append_result {
            store_first_stream_output_error(&mut ctx.first_error, error);
        }
    }

    #[cidre::objc::add_methods]
    impl cidre::sc::StreamOutputImpl for ScStreamOutputDelegate {
        extern "C" fn impl_stream_did_output_sample_buf(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _stream: &cidre::sc::Stream,
            sample_buf: &mut cidre::cm::SampleBuf,
            kind: cidre::sc::OutputType,
        ) {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let objc_result = ns::try_catch(|| {
                    let ctx = self.inner_mut();

                    handle_stream_did_output_sample_buf(ctx, sample_buf, kind);
                });

                if let Err(exception) = objc_result {
                    store_first_stream_output_error(
                        &mut self.inner_mut().first_error,
                        stream_output_callback_objc_exception_error(exception),
                    );
                }
            }));

            if let Err(payload) = result {
                store_first_stream_output_error(
                    &mut self.inner_mut().first_error,
                    stream_output_callback_panic_error(payload),
                );
            }
        }
    }
}

#[cfg(target_os = "macos")]
use stream_output_delegate::ScStreamOutputDelegate;

#[cfg(target_os = "macos")]
mod stream_delegate {
    use super::{error_with_ns_error, log_capture_error, ScStreamDelegateState};
    use cidre::objc;
    use cidre::sc::StreamDelegate;

    cidre::define_obj_type!(
        pub(super) ScStreamDelegate + cidre::sc::StreamDelegateImpl,
        ScStreamDelegateState,
        ZScStreamDelegate
    );

    impl cidre::sc::StreamDelegate for ScStreamDelegate {}

    #[cidre::objc::add_methods]
    impl cidre::sc::StreamDelegateImpl for ScStreamDelegate {
        extern "C" fn impl_stream_did_stop_with_err(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _stream: &cidre::sc::Stream,
            error: &cidre::ns::Error,
        ) {
            let stop_error = error_with_ns_error(
                "capture_stream_system_stopped",
                "ScreenCaptureKit stream stopped unexpectedly",
                error,
            );
            log_capture_error(
                "ScreenCaptureKit delegate reported stopped stream",
                &stop_error,
            );
            let mut state = self
                .inner_mut()
                .lifecycle_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.stream_live = false;
            state.stop_error = Some(stop_error);
        }

        extern "C" fn impl_stream_did_become_inactive(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _stream: &cidre::sc::Stream,
        ) {
            let mut state = self
                .inner_mut()
                .lifecycle_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.stream_live = false;
        }

        extern "C" fn impl_stream_did_become_active(
            &mut self,
            _cmd: Option<&cidre::objc::Sel>,
            _stream: &cidre::sc::Stream,
        ) {
            let mut state = self
                .inner_mut()
                .lifecycle_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.stream_live = true;
            state.stop_error = None;
        }
    }
}

#[cfg(target_os = "macos")]
use stream_delegate::ScStreamDelegate;

#[cfg(target_os = "macos")]
fn should_append_screen_sample(sample_buf: &cidre::cm::SampleBuf) -> bool {
    use cidre::{cf, cm, sc};

    let mut attachment_mode = cm::AttachMode::Propagate;
    let status_key = sc::FrameInfo::status().as_type_ref().try_as_string();
    let status_value = status_key
        .and_then(|key| sample_buf.attach(key, &mut attachment_mode))
        .and_then(cf::Type::try_as_number)
        .and_then(|status| status.to_i32());

    should_append_screen_sample_with_state(
        status_value,
        sample_buf.data_is_ready(),
        sample_buf.image_buf().is_some(),
        sc::FrameStatus::Complete as i32,
    )
}

#[cfg(target_os = "macos")]
fn should_append_screen_sample_with_state(
    status_value: Option<i32>,
    data_is_ready: bool,
    has_image_buffer: bool,
    complete_status: i32,
) -> bool {
    if !data_is_ready || !has_image_buffer {
        return false;
    }

    match status_value {
        Some(value) => value == complete_status,
        None => true,
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C-unwind" {
    fn CVPixelBufferGetBaseAddress(pixel_buffer: &cidre::cv::PixelBuf) -> *const std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: &cidre::cv::PixelBuf) -> usize;
    fn CVPixelBufferGetDataSize(pixel_buffer: &cidre::cv::PixelBuf) -> usize;
}

#[cfg(target_os = "macos")]
fn maybe_remove_screen_video_file(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => capture_runtime::debug_log!(
            "[capture-screen] failed to remove invalid screen video artifact {path}: {error}"
        ),
    }
}

#[cfg(target_os = "macos")]
fn maybe_remove_system_audio_file(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => capture_runtime::debug_log!(
            "[capture-screen] failed to remove zero-sample system audio artifact {path}: {error}"
        ),
    }
}

#[cfg(target_os = "macos")]
fn validate_screen_video_file(path: &str) -> Result<(), CaptureErrorResponse> {
    use cidre::{av, ns};

    let metadata = std::fs::metadata(path).map_err(|error| CaptureErrorResponse {
        code: "capture_output_processing_failed".to_string(),
        message: format!("{FINALIZED_SCREEN_RECORDING_INSPECTION_ERROR_PREFIX}{error}"),
    })?;
    if metadata.len() == 0 {
        return Err(CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: FINALIZED_SCREEN_RECORDING_EMPTY_ERROR_MESSAGE.to_string(),
        });
    }

    let result = {
        let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        let output_url = ns::Url::with_fs_path_str(path, false);
        let asset =
            av::UrlAsset::with_url(&output_url, None).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to open finalized screen recording for validation".to_string(),
            })?;

        let tracks = load_asset_tracks_with_timeout(
            asset.as_ref(),
            av::MediaType::video(),
            "capture_output_processing_failed",
            "Timed out while validating finalized screen recording video track",
        )?;

        if tracks.is_empty() {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE.to_string(),
            });
        }

        Ok(())
    };

    result
}

#[cfg(target_os = "macos")]
fn is_recoverable_screen_recording_validation_error(message: &str) -> bool {
    matches!(
        message,
        FINALIZED_SCREEN_RECORDING_EMPTY_ERROR_MESSAGE
            | FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE
    ) || (message.starts_with(FINALIZED_SCREEN_RECORDING_INSPECTION_ERROR_PREFIX)
        && FINALIZED_SCREEN_RECORDING_MISSING_FILE_MARKERS
            .iter()
            .any(|marker| message.contains(marker)))
}

#[cfg(target_os = "macos")]
fn is_recoverable_screen_segment_finalize_failure(message: &str) -> bool {
    capture_writers::is_no_video_samples_error_message("screen", message)
        || is_recoverable_screen_recording_validation_error(message)
}

#[cfg(target_os = "macos")]
fn is_recoverable_screen_segment_finalize_failure_detail(detail: &str) -> bool {
    is_recoverable_screen_segment_finalize_failure(detail)
        || detail
            .strip_prefix(SCREEN_VIDEO_WRITER_FAILURE_PREFIX)
            .is_some_and(|writer_failure| {
                capture_writers::is_no_video_samples_error_message("screen", writer_failure)
                    || is_avfoundation_11800_screen_video_failure(
                        writer_failure,
                        SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX,
                    )
            })
}

#[cfg(target_os = "macos")]
fn contains_additional_screen_segment_finalize_failures(detail: &str) -> bool {
    SCREEN_SEGMENT_FINALIZE_FAILURE_PREFIXES
        .iter()
        .any(|prefix| detail.contains(&format!("; {prefix}")))
}

#[cfg(target_os = "macos")]
fn is_avfoundation_11800_screen_video_failure(message: &str, prefix: &str) -> bool {
    message
        .strip_prefix(prefix)
        .is_some_and(|failure| failure.ends_with(AVFOUNDATION_FAILURE_CODE_11800_SUFFIX))
}

#[cfg(target_os = "macos")]
fn is_recoverable_screen_video_writer_avfoundation_11800_failure_pair(message: &str) -> bool {
    let Some(detail) = capture_writers::strip_output_processing_failure_prefix(message) else {
        return false;
    };
    let Some((stream_output_failure, writer_failure)) =
        detail.split_once(&format!("; {SCREEN_VIDEO_WRITER_FAILURE_PREFIX}"))
    else {
        return false;
    };

    if contains_additional_screen_segment_finalize_failures(stream_output_failure)
        || contains_additional_screen_segment_finalize_failures(writer_failure)
    {
        return false;
    }

    let Some(stream_output_failure) =
        stream_output_failure.strip_prefix(SCREEN_STREAM_OUTPUT_PROCESSING_FAILURE_PREFIX)
    else {
        return false;
    };

    is_avfoundation_11800_screen_video_failure(
        stream_output_failure,
        SCREEN_VIDEO_APPEND_SAMPLE_FAILURE_PREFIX,
    ) && is_avfoundation_11800_screen_video_failure(
        writer_failure,
        SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX,
    )
}

#[cfg(target_os = "macos")]
pub fn should_recover_from_segment_finalize_error(error: &CaptureErrorResponse) -> bool {
    error.code == "capture_output_processing_failed"
        && (is_recoverable_screen_segment_finalize_failure(&error.message)
            || capture_writers::single_output_processing_failure_detail(
                &error.message,
                &SCREEN_SEGMENT_FINALIZE_FAILURE_PREFIXES,
            )
            .is_some_and(is_recoverable_screen_segment_finalize_failure_detail)
            || is_recoverable_screen_video_writer_avfoundation_11800_failure_pair(&error.message))
}

#[cfg(target_os = "macos")]
pub struct StartedCaptureSession {
    pub session: ActiveCaptureSession,
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(target_os = "macos")]
pub struct RotatedCaptureOutputs {
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(target_os = "macos")]
type StartCallbackMap = HashMap<usize, mpsc::Sender<()>>;
#[cfg(target_os = "macos")]
type FinishCallbackMap = HashMap<usize, mpsc::Sender<Result<(), CaptureErrorResponse>>>;

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct AvFoundationCaptureSession {
    capture_session: objc2::rc::Retained<objc2_av_foundation::AVCaptureSession>,
    movie_output: objc2::rc::Retained<objc2_av_foundation::AVCaptureMovieFileOutput>,
    _delegate: objc2::rc::Retained<objc2_foundation::NSObject>,
    delegate_key: usize,
    finish_rx: mpsc::Receiver<Result<(), CaptureErrorResponse>>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct ScreenCaptureKitCaptureSession {
    stream: cidre::arc::R<cidre::sc::Stream>,
    _stream_delegate: cidre::arc::R<ScStreamDelegate>,
    stream_output_delegate: cidre::arc::R<ScStreamOutputDelegate>,
    stream_output_queue: cidre::arc::R<dispatch::Queue>,
    sources: ScreenCaptureSources,
    video_bitrate_bps: Option<u32>,
    frame_export: Option<ScreenFrameExportConfig>,
    system_audio_inactivity_tail_buffer_seconds: u64,
    lifecycle_state: Arc<Mutex<ScreenCaptureKitLifecycleState>>,
    privacy_filter_state: Option<PrivacyFilterApplicationState>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct ScreenCaptureKitLifecycleState {
    stream_live: bool,
    stop_error: Option<CaptureErrorResponse>,
}

#[cfg(target_os = "macos")]
impl Default for ScreenCaptureKitLifecycleState {
    fn default() -> Self {
        Self {
            stream_live: true,
            stop_error: None,
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct ScStreamDelegateState {
    lifecycle_state: Arc<Mutex<ScreenCaptureKitLifecycleState>>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
enum CaptureBackendSession {
    AvFoundation(AvFoundationCaptureSession),
    ScreenCaptureKit(ScreenCaptureKitCaptureSession),
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ActiveCaptureSession {
    backend: CaptureBackendSession,
}

#[cfg(target_os = "macos")]
unsafe impl Send for ActiveCaptureSession {}

#[cfg(target_os = "macos")]
impl ActiveCaptureSession {
    fn is_screen_stream_live(&self) -> bool {
        match &self.backend {
            CaptureBackendSession::AvFoundation(_) => true,
            CaptureBackendSession::ScreenCaptureKit(session) => session.is_stream_live(),
        }
    }

    fn take_screen_stop_error(&mut self) -> Option<CaptureErrorResponse> {
        match &mut self.backend {
            CaptureBackendSession::AvFoundation(_) => None,
            CaptureBackendSession::ScreenCaptureKit(session) => session.take_stop_error(),
        }
    }

    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        match &mut self.backend {
            CaptureBackendSession::AvFoundation(session) => session.stop(),
            CaptureBackendSession::ScreenCaptureKit(session) => session.stop(),
        }
    }

    fn stop_for_inactivity(&mut self, tail_trim_seconds: u64) -> Result<(), CaptureErrorResponse> {
        match &mut self.backend {
            CaptureBackendSession::AvFoundation(session) => session.stop(),
            CaptureBackendSession::ScreenCaptureKit(session) => {
                session.stop_for_inactivity(tail_trim_seconds)
            }
        }
    }

    fn update_privacy_filter(
        &mut self,
        filter: PrivacyContentFilter,
    ) -> Result<(), PrivacyFilterApplyError> {
        match &mut self.backend {
            CaptureBackendSession::ScreenCaptureKit(session) => {
                session.update_privacy_filter(filter)
            }
            CaptureBackendSession::AvFoundation(_) => Err(PrivacyFilterApplyError {
                kind: PrivacyFilterApplyErrorKind::UnsupportedPlatform,
                message: "Privacy content filters require ScreenCaptureKit".to_string(),
            }),
        }
    }
}

#[cfg(target_os = "macos")]
impl AvFoundationCaptureSession {
    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        let was_recording = unsafe { self.movie_output.isRecording() };
        let mut finalize_error: Option<CaptureErrorResponse> = None;

        if was_recording {
            unsafe { self.movie_output.stopRecording() };

            match self.finish_rx.recv_timeout(Duration::from_secs(15)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    log_capture_error(
                        "AVFoundation capture finalization failed during stop",
                        &error,
                    );
                    finalize_error = Some(error);
                }
                Err(_) => {
                    let mut callbacks = delegate_finish_callbacks()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    callbacks.remove(&self.delegate_key);
                    let error = CaptureErrorResponse {
                        code: "capture_stop_incomplete".to_string(),
                        message: "Timed out waiting for native capture file finalization"
                            .to_string(),
                    };
                    log_capture_error(
                        "AVFoundation capture stop timed out waiting for finalization",
                        &error,
                    );
                    finalize_error = Some(error);
                }
            }
        }

        unsafe { self.capture_session.stopRunning() };

        if let Some(error) = finalize_error {
            return Err(error);
        }

        Ok(())
    }
}

#[cfg(target_os = "macos")]
impl ScreenCaptureKitCaptureSession {
    fn update_privacy_filter(
        &mut self,
        filter: PrivacyContentFilter,
    ) -> Result<(), PrivacyFilterApplyError> {
        let next_key = filter.key();
        if self
            .privacy_filter_state
            .as_ref()
            .is_some_and(|state| state.satisfies_request(&next_key))
        {
            return Ok(());
        }
        let built_filter = build_screen_capture_kit_content_filter(&filter)?;
        let should_log_diagnostics = should_log_privacy_filter_diagnostics(
            self.privacy_filter_state.as_ref(),
            &built_filter.state,
        );
        if self
            .privacy_filter_state
            .as_ref()
            .is_some_and(|state| state.applied_filter_matches(&built_filter.state))
        {
            if should_log_diagnostics {
                log_privacy_filter_diagnostics(&built_filter.state);
            }
            self.privacy_filter_state = Some(built_filter.state);
            return Ok(());
        }
        if should_log_diagnostics {
            log_privacy_filter_diagnostics(&built_filter.state);
        }
        let (tx, rx) = mpsc::channel();
        self.stream
            .update_content_filter_ch(&built_filter.filter, move |error| {
                let result = error
                    .map(|error| PrivacyFilterApplyError {
                        kind: PrivacyFilterApplyErrorKind::FilterUpdateFailed,
                        message: format!(
                            "Failed to update ScreenCaptureKit content filter: {error}"
                        ),
                    })
                    .map_or(Ok(()), Err);
                let _ = tx.send(result);
            });
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {
                self.privacy_filter_state = Some(built_filter.state);
                Ok(())
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(PrivacyFilterApplyError {
                kind: PrivacyFilterApplyErrorKind::FilterUpdateFailed,
                message: "Timed out updating ScreenCaptureKit content filter".to_string(),
            }),
        }
    }

    fn is_stream_live(&self) -> bool {
        self.lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stream_live
    }

    fn take_stop_error(&mut self) -> Option<CaptureErrorResponse> {
        self.lifecycle_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop_error
            .take()
    }

    fn is_stop_timeout_code(code: &str) -> bool {
        matches!(
            code,
            "capture_stop_incomplete" | "capture_start_rollback_incomplete"
        )
    }

    fn stop_stream(
        stream: &cidre::sc::Stream,
        timeout_code: &str,
    ) -> Result<(), CaptureErrorResponse> {
        let (tx, rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
        let mut completion = cidre::blocks::ErrCh::new1(move |error| {
            let _ = tx.send(match error {
                Some(error) => Err(error_with_ns_error(
                    "capture_stop_failed",
                    "Failed to stop ScreenCaptureKit stream",
                    error,
                )),
                None => Ok(()),
            });
        });

        stream.stop_with_ch_block(Some(&mut completion));

        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result,
            Err(_) => Err(CaptureErrorResponse {
                code: timeout_code.to_string(),
                message: "Timed out waiting for ScreenCaptureKit stream stop".to_string(),
            }),
        }
    }

    fn stop(&mut self) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(0)
    }

    fn stop_for_inactivity(&mut self, tail_trim_seconds: u64) -> Result<(), CaptureErrorResponse> {
        self.stop_with_inactivity_tail_trim_seconds(tail_trim_seconds)
    }

    fn stop_with_inactivity_tail_trim_seconds(
        &mut self,
        tail_trim_seconds: u64,
    ) -> Result<(), CaptureErrorResponse> {
        let mut stop_error: Option<CaptureErrorResponse> = None;

        let stream_stopped = match Self::stop_stream(&self.stream, "capture_stop_incomplete") {
            Ok(()) => true,
            Err(error) => {
                if Self::is_stop_timeout_code(error.code.as_str()) {
                    log_capture_error("ScreenCaptureKit stream stop timed out", &error);
                    return Err(error);
                }

                log_capture_error("ScreenCaptureKit stream stop failed", &error);
                if stop_error.is_none() {
                    stop_error = Some(error);
                }

                false
            }
        };

        if stream_stopped {
            synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
            self.stream_output_delegate
                .inner_mut()
                .system_audio_tail_trim_seconds = tail_trim_seconds;
            if let Err(error) =
                finalize_stream_output_context(self.stream_output_delegate.inner_mut())
            {
                log_capture_error(
                    "ScreenCaptureKit output finalization failed during stop",
                    &error,
                );
                if stop_error.is_none() {
                    stop_error = Some(error);
                }
            }
        }

        if let Some(error) = stop_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn pause_system_audio_writer_for_inactivity(
        &mut self,
        tail_trim_seconds: u64,
    ) -> Result<(), CaptureErrorResponse> {
        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let ctx = self.stream_output_delegate.inner_mut();
        if let Some(mut writer) = ctx.system_audio_writer.take() {
            set_audio_writer_inactivity_tail_trim_seconds(&mut writer, tail_trim_seconds);
            if let Err(error) = finish_audio_asset_writer_discarding_inactivity_tail(&mut writer) {
                if recover_zero_sample_system_audio_soft_pause(
                    ctx.system_audio_output_file.as_deref(),
                    &error,
                ) {
                    return Ok(());
                }

                log_capture_error(
                    "failed to finalize system audio writer during soft-pause",
                    &error,
                );
                return Err(error);
            }
        }
        Ok(())
    }

    fn pause_screen_outputs_for_inactivity(&mut self) -> Result<(), CaptureErrorResponse> {
        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let ctx = self.stream_output_delegate.inner_mut();

        if ctx.screen_video_output_file.is_none() && ctx.frame_export.is_none() {
            return Ok(());
        }

        let screen_video_output_file = ctx.screen_video_output_file.take();
        let mut frame_export = ctx.frame_export.take();
        let mut failures = Vec::new();

        if let Some(mut writer) = ctx.screen_video_writer.take() {
            if let Err(error) = finish_video_asset_writer(&mut writer) {
                if capture_writers::is_no_video_samples_error_message("screen", &error.message) {
                    if let Some(path) = screen_video_output_file.as_deref() {
                        maybe_remove_screen_video_file(path);
                    }
                }
                failures.push(format!("screen video writer failed: {}", error.message));
            }
        }

        if let Err(error) =
            finalize_screen_frame_export(screen_video_output_file.as_deref(), frame_export.as_mut())
        {
            failures.push(format!("screen frame export failed: {}", error.message));
        }

        capture_writers::aggregate_output_processing_failures(failures)
    }

    fn resume_system_audio_writer(
        &mut self,
        output_path: &str,
    ) -> Result<(), CaptureErrorResponse> {
        use cidre::ns;

        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let ctx = self.stream_output_delegate.inner_mut();
        if ctx.system_audio_writer.is_some() {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "System audio writer is already active; pause before resuming".to_string(),
            });
        }
        let output_url = ns::Url::with_fs_path_str(output_path, false);
        let mut writer = create_audio_asset_writer(&output_url, "system audio")?;
        set_audio_writer_inactivity_tail_trim_seconds(
            &mut writer,
            ctx.system_audio_inactivity_tail_buffer_seconds,
        );
        ctx.system_audio_writer = Some(writer);
        ctx.system_audio_tail_trim_seconds = 0;
        Ok(())
    }

    fn resume_screen_outputs(
        &mut self,
        segment_dir: &Path,
        recording_file: &str,
    ) -> Result<(), CaptureErrorResponse> {
        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let ctx = self.stream_output_delegate.inner_mut();
        if ctx.screen_video_output_file.is_some() || ctx.frame_export.is_some() {
            return Err(CaptureErrorResponse {
                code: "invalid_runtime_state".to_string(),
                message: "Screen outputs are already active; pause before resuming".to_string(),
            });
        }

        ctx.screen_video_output_file = self.sources.screen.then(|| recording_file.to_string());
        ctx.screen_video_writer = None;
        ctx.video_bitrate_bps = self.video_bitrate_bps;
        ctx.frame_export = if self.sources.screen {
            screen_frame_export_runtime(segment_dir, self.frame_export.clone())?
        } else {
            None
        };
        ctx.first_error = None;
        Ok(())
    }

    fn rotate_output_files(
        &mut self,
        segment_dir: &Path,
        screen_output_file: Option<&Path>,
        system_audio_output_path: Option<&Path>,
    ) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
        let recording_file = screen_output_file
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| segment_dir.join("screen.mov").to_string_lossy().to_string());
        let system_audio_recording_file = self
            .sources
            .system_audio
            .then(|| system_audio_output_path.map(|p| p.to_string_lossy().to_string()))
            .flatten();

        let mut output_files =
            output_files_for_session(segment_dir, system_audio_output_path, &self.sources);
        if self.sources.screen && screen_output_file.is_some() {
            output_files.screen_file = Some(recording_file.clone());
            output_files.screen_files = vec![recording_file.clone()];
        }

        std::fs::create_dir_all(segment_dir)
            .map_err(|e| CaptureErrorResponse {
                code: "io_error".to_string(),
                message: format!("Failed to create capture session directory: {e}"),
            })
            .map_err(|error| {
                log_capture_error(
                    "failed to create ScreenCaptureKit segment directory during rotation",
                    &error,
                );
                error
            })?;

        let next_context = stream_output_context_for_segment(
            segment_dir,
            &recording_file,
            system_audio_recording_file.as_deref(),
            &self.sources,
            self.sources.system_audio && system_audio_recording_file.is_some(),
            self.video_bitrate_bps,
            self.frame_export.clone(),
            self.system_audio_inactivity_tail_buffer_seconds,
        )
        .map_err(|error| {
            log_capture_error(
                "failed to prepare ScreenCaptureKit segment outputs during rotation",
                &error,
            );
            error
        })?;

        synchronize_stream_output_queue(Some(self.stream_output_queue.as_ref()));
        let mut previous_context =
            std::mem::replace(self.stream_output_delegate.inner_mut(), next_context);
        finalize_rotated_segment_context(&mut previous_context)?;
        drop(previous_context);

        Ok(RotatedCaptureOutputs {
            recording_file,
            system_audio_recording_file,
            output_files,
        })
    }
}

#[cfg(target_os = "macos")]
struct BuiltScreenCaptureKitContentFilter {
    filter: cidre::arc::R<cidre::sc::ContentFilter>,
    state: PrivacyFilterApplicationState,
}

#[cfg(target_os = "macos")]
fn build_screen_capture_kit_content_filter(
    privacy_filter: &PrivacyContentFilter,
) -> Result<BuiltScreenCaptureKitContentFilter, PrivacyFilterApplyError> {
    let (content_tx, content_rx) = mpsc::channel();
    cidre::sc::ShareableContent::current_with_ch(move |content, error| {
        let result = match (content, error) {
            (Some(content), None) => Ok(content.retained()),
            (_, Some(error)) => Err(PrivacyFilterApplyError {
                kind: PrivacyFilterApplyErrorKind::DisplayUnavailable,
                message: format!("Failed to query ScreenCaptureKit shareable content: {error}"),
            }),
            _ => Err(PrivacyFilterApplyError {
                kind: PrivacyFilterApplyErrorKind::DisplayUnavailable,
                message: "No ScreenCaptureKit shareable content available".to_string(),
            }),
        };
        let _ = content_tx.send(result);
    });
    let content = content_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| PrivacyFilterApplyError {
            kind: PrivacyFilterApplyErrorKind::DisplayUnavailable,
            message: "Timed out querying ScreenCaptureKit shareable content".to_string(),
        })??;

    let displays = content.displays();
    let display = if privacy_filter.display_id == 0 {
        displays.first()
    } else {
        displays
            .iter()
            .find(|display| display.display_id().0 == privacy_filter.display_id)
    }
    .ok_or_else(|| PrivacyFilterApplyError {
        kind: PrivacyFilterApplyErrorKind::DisplayUnavailable,
        message: format!(
            "No ScreenCaptureKit display available for privacy filter display {}",
            privacy_filter.display_id
        ),
    })?;

    let windows = content.windows();
    let apps = content.apps();
    let requested_bundle_ids: std::collections::BTreeSet<_> =
        privacy_filter.excluded_bundle_ids.iter().cloned().collect();
    let requested_window_ids: std::collections::BTreeSet<_> =
        privacy_filter.excluded_window_ids.iter().copied().collect();
    let available_apps = apps
        .iter()
        .map(|app| PrivacyFilterAvailableApp {
            bundle_id: app.bundle_id().to_string(),
        })
        .collect::<Vec<_>>();
    let available_windows = windows
        .iter()
        .map(|window| PrivacyFilterAvailableWindow {
            id: window.id(),
            owning_bundle_id: window.owning_app().map(|app| app.bundle_id().to_string()),
        })
        .collect::<Vec<_>>();
    let plan = plan_privacy_content_filter(
        &requested_bundle_ids,
        &requested_window_ids,
        &available_apps,
        &available_windows,
    );

    let state = PrivacyFilterApplicationState::from_plan(privacy_filter, &plan);
    let requested_excluded_windows: Vec<_> = windows
        .iter()
        .filter(|window| plan.excluded_window_ids.contains(&window.id()))
        .map(|window| window.retained())
        .collect();

    let filter = if requested_excluded_windows.is_empty() && plan.excluded_bundle_ids.is_empty() {
        screen_capture_kit_no_exclusion_filter(display, &apps)
    } else {
        match plan.strategy {
            PrivacyContentFilterStrategy::ExcludingApps => {
                let mut excluded_apps = Vec::new();
                let mut pushed_apps = std::collections::BTreeSet::new();
                for app in apps.iter() {
                    let bundle_id = app.bundle_id().to_string();
                    if plan.excluded_bundle_ids.contains(&bundle_id)
                        && pushed_apps.insert((app.process_id(), bundle_id))
                    {
                        excluded_apps.push(app.retained());
                    }
                }
                for window in windows.iter() {
                    let Some(app) = window.owning_app() else {
                        continue;
                    };
                    let bundle_id = app.bundle_id().to_string();
                    if plan.excluded_bundle_ids.contains(&bundle_id)
                        && pushed_apps.insert((app.process_id(), bundle_id))
                    {
                        excluded_apps.push(app.retained());
                    }
                }
                let app_array = cidre::ns::Array::from_slice_retained(&excluded_apps);
                cidre::sc::ContentFilter::with_display_excluding_apps_excepting_windows(
                    display,
                    &app_array,
                    &cidre::ns::Array::new(),
                )
            }
            PrivacyContentFilterStrategy::ExcludingWindows
            | PrivacyContentFilterStrategy::ExcludingAppWindowsAndWindows => {
                let window_array =
                    cidre::ns::Array::from_slice_retained(&requested_excluded_windows);
                cidre::sc::ContentFilter::with_display_excluding_windows(display, &window_array)
            }
        }
    };

    Ok(BuiltScreenCaptureKitContentFilter { filter, state })
}

#[cfg(target_os = "macos")]
fn screen_capture_kit_no_exclusion_filter(
    display: &cidre::sc::Display,
    _apps: &cidre::ns::Array<cidre::sc::RunningApp>,
) -> cidre::arc::R<cidre::sc::ContentFilter> {
    cidre::sc::ContentFilter::with_display_excluding_windows(display, &cidre::ns::Array::new())
}

#[cfg(target_os = "macos")]
fn recover_zero_sample_system_audio_soft_pause(
    system_audio_output_file: Option<&str>,
    error: &CaptureErrorResponse,
) -> bool {
    if !capture_writers::is_no_audio_samples_error_message("system audio", &error.message) {
        return false;
    }

    if let Some(path) = system_audio_output_file {
        maybe_remove_system_audio_file(path);
    }

    true
}

#[cfg(target_os = "macos")]
fn fmt_ns<T: Display + ?Sized>(value: &T) -> String {
    format!("{value}")
}

#[cfg(target_os = "macos")]
fn error_with_ns_error(code: &str, prefix: &str, error: &cidre::ns::Error) -> CaptureErrorResponse {
    CaptureErrorResponse {
        code: code.to_string(),
        message: format!(
            "{prefix}: {} (code: {})",
            fmt_ns(error.localized_desc().as_ref()),
            error.code(),
        ),
    }
}

#[cfg(target_os = "macos")]
pub fn new_session_id() -> Result<String, CaptureErrorResponse> {
    use cidre::ns;

    let uuid = ns::Uuid::new();
    let uuid_str = uuid.string();
    Ok(format!(
        "native-session-{}",
        fmt_ns(uuid_str.as_ref()).to_lowercase()
    ))
}

#[cfg(target_os = "macos")]
fn create_session_dir(session_dir: &Path) -> Result<(), CaptureErrorResponse> {
    std::fs::create_dir_all(session_dir).map_err(|e| CaptureErrorResponse {
        code: "io_error".to_string(),
        message: format!("Failed to create capture session directory: {e}"),
    })
}

#[cfg(target_os = "macos")]
fn remove_session_dir(session_dir: &std::path::Path) -> Result<(), CaptureErrorResponse> {
    match std::fs::remove_dir_all(session_dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CaptureErrorResponse {
            code: "io_error".to_string(),
            message: format!(
                "Failed to remove capture session directory after startup rollback: {error}"
            ),
        }),
    }
}

#[cfg(target_os = "macos")]
fn finalize_startup_result<T>(
    start_result: Result<T, CaptureErrorResponse>,
    session_dir: &std::path::Path,
) -> Result<T, CaptureErrorResponse> {
    match start_result {
        Ok(started) => Ok(started),
        Err(start_error) => {
            capture_runtime::debug_log!(
                "[capture-screen] capture startup failed for {}: [{}] {}",
                session_dir.display(),
                start_error.code,
                start_error.message
            );

            if should_preserve_runtime_on_startup_error(&start_error) {
                return Err(start_error);
            }

            if let Err(cleanup_error) = remove_session_dir(session_dir) {
                capture_runtime::debug_log!(
                    "[capture-screen] startup cleanup failed for {}: [{}] {}",
                    session_dir.display(),
                    cleanup_error.code,
                    cleanup_error.message
                );
                return Err(CaptureErrorResponse {
                    code: start_error.code,
                    message: format!(
                        "{}; additionally failed startup cleanup: [{}] {}",
                        start_error.message, cleanup_error.code, cleanup_error.message
                    ),
                });
            }

            Err(start_error)
        }
    }
}

#[cfg(target_os = "macos")]
fn should_preserve_runtime_on_startup_error(error: &CaptureErrorResponse) -> bool {
    error.code == "capture_start_rollback_incomplete"
}

#[cfg(target_os = "macos")]
fn recording_delegate_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel};
    use objc2::{sel, ClassType};
    use objc2_foundation::NSObject;

    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

    CLASS.get_or_init(|| {
        extern "C-unwind" fn did_start_recording(
            this: *mut AnyObject,
            _cmd: Sel,
            _output: *mut AnyObject,
            _url: *mut AnyObject,
            _connections: *mut AnyObject,
        ) {
            let key = this as usize;
            if let Some(tx) = delegate_start_callbacks()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key)
            {
                let _ = tx.send(());
            }
        }

        extern "C-unwind" fn did_finish_recording(
            this: *mut AnyObject,
            _cmd: Sel,
            _output: *mut AnyObject,
            _url: *mut AnyObject,
            _connections: *mut AnyObject,
            error: *mut AnyObject,
        ) {
            let key = this as usize;
            if let Some(tx) = delegate_finish_callbacks()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&key)
            {
                let result = if error.is_null() {
                    Ok(())
                } else {
                    Err(CaptureErrorResponse {
                        code: "capture_finalize_failed".to_string(),
                        message: "Native capture finalization failed".to_string(),
                    })
                };
                let _ = tx.send(result);
            }
        }

        let mut decl = ClassBuilder::new(
            &CString::new("ZNativeCaptureRecorderDelegate").expect("valid class name"),
            NSObject::class(),
        )
        .expect("failed to create recorder delegate class");

        if let Some(protocol) = AnyProtocol::get(
            &CString::new("AVCaptureFileOutputRecordingDelegate").expect("valid protocol name"),
        ) {
            decl.add_protocol(protocol);
        }

        unsafe {
            let did_start: extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
            ) = did_start_recording;
            decl.add_method(
                sel!(fileOutput:didStartRecordingToOutputFileAtURL:fromConnections:),
                did_start,
            );

            let did_finish: extern "C-unwind" fn(
                *mut AnyObject,
                Sel,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
                *mut AnyObject,
            ) = did_finish_recording;
            decl.add_method(
                sel!(fileOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:),
                did_finish,
            );
        }

        decl.register()
    })
}

#[cfg(target_os = "macos")]
fn delegate_finish_callbacks() -> &'static Mutex<FinishCallbackMap> {
    static CALLBACKS: OnceLock<Mutex<FinishCallbackMap>> = OnceLock::new();
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_os = "macos")]
fn delegate_start_callbacks() -> &'static Mutex<StartCallbackMap> {
    static CALLBACKS: OnceLock<Mutex<StartCallbackMap>> = OnceLock::new();
    CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_os = "macos")]
pub fn start_capture_session(
    session_dir: &Path,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    start_capture_session_with_options(
        session_dir,
        None,
        None,
        sources,
        screen_frame_rate,
        screen_resolution,
        video_bitrate_bps,
        ScreenCaptureSessionOptions::default(),
    )
}

#[cfg(target_os = "macos")]
pub fn start_capture_session_with_options(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    system_audio_output_path: Option<&Path>,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
    options: ScreenCaptureSessionOptions,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    let system_audio_writer_active = system_audio_writer_active_for_options(sources, &options);
    let backend = if sources.screen && supports_screen_capture_kit_backend() {
        "ScreenCaptureKit"
    } else {
        "AVFoundation"
    };
    capture_runtime::debug_log!(
        "[capture-screen] starting {backend} capture session at {} (sources: screen={}, system_audio={}, system_audio_writer_active={}, frame_rate={}, resolution={:?}, video_bitrate_bps={:?})",
        session_dir.display(),
        sources.screen,
        sources.system_audio,
        system_audio_writer_active,
        screen_frame_rate,
        screen_resolution,
        video_bitrate_bps
    );

    if sources.screen && supports_screen_capture_kit_backend() {
        return start_screen_capture_kit_session(
            session_dir,
            screen_output_file,
            system_audio_output_path,
            sources,
            screen_frame_rate,
            screen_resolution,
            video_bitrate_bps,
            options,
        );
    }

    start_avfoundation_capture_session(
        session_dir,
        screen_output_file,
        sources,
        screen_resolution,
        video_bitrate_bps,
        options,
    )
}

#[cfg(target_os = "macos")]
fn start_avfoundation_capture_session(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    sources: &ScreenCaptureSources,
    screen_resolution: &ScreenResolution,
    _video_bitrate_bps: Option<u32>,
    options: ScreenCaptureSessionOptions,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    use objc2_av_foundation::{
        AVCaptureInput, AVCaptureMovieFileOutput, AVCaptureOutput, AVCaptureScreenInput,
        AVCaptureSession,
    };
    use objc2_foundation::{NSObject, NSURL};

    create_session_dir(session_dir).map_err(|error| {
        log_capture_error(
            "failed to create AVFoundation capture session directory during startup",
            &error,
        );
        error
    })?;

    if options.frame_export.is_some() {
        let error = CaptureErrorResponse {
            code: "capture_frame_export_unsupported".to_string(),
            message: "Frame export requires the ScreenCaptureKit backend (macOS 15+)".to_string(),
        };
        log_capture_error("AVFoundation capture startup rejected frame export", &error);
        return Err(error);
    }

    if sources.screen
        && *screen_resolution
            != (ScreenResolution::Preset {
                preset: ScreenResolutionPreset::Original,
            })
    {
        let error = CaptureErrorResponse {
            code: "screen_resolution_unsupported".to_string(),
            message: "Selected screen resolution requires the ScreenCaptureKit backend (macOS 15+). On this backend, only the original display resolution is supported.".to_string(),
        };
        log_capture_error(
            "AVFoundation capture startup rejected unsupported screen resolution",
            &error,
        );
        return Err(error);
    }

    if sources.screen
        && options
            .initial_privacy_filter
            .as_ref()
            .is_some_and(|filter| {
                !filter.excluded_bundle_ids.is_empty() || !filter.excluded_window_ids.is_empty()
            })
    {
        let error = CaptureErrorResponse {
            code: "privacy_filter_unsupported".to_string(),
            message: "Privacy content filters require ScreenCaptureKit".to_string(),
        };
        log_capture_error(
            "AVFoundation capture startup rejected active privacy filter",
            &error,
        );
        return Err(error);
    }

    let start_result = (|| {
        let output_file = screen_output_file
            .map(Path::to_path_buf)
            .unwrap_or_else(|| session_dir.join("screen.mov"));
        let output_file_str = output_file.to_string_lossy().to_string();

        let mut output_files = output_files_for_session(&session_dir, None, sources);
        if sources.screen && screen_output_file.is_some() {
            output_files.screen_file = Some(output_file_str.clone());
            output_files.screen_files = vec![output_file_str.clone()];
        }

        let capture_session = unsafe { AVCaptureSession::new() };

        if sources.screen {
            let screen_input = unsafe { AVCaptureScreenInput::new() };
            let screen_input_ref: &AVCaptureInput =
                unsafe { &*(&*screen_input as *const _ as *const AVCaptureInput) };

            let can_add = unsafe { capture_session.canAddInput(screen_input_ref) };
            if can_add {
                unsafe { capture_session.addInput(screen_input_ref) };
            }

            if !can_add {
                return Err(CaptureErrorResponse {
                    code: "screen_input_unavailable".to_string(),
                    message: "Failed to add screen input".to_string(),
                });
            }
        }

        let movie_output = unsafe { AVCaptureMovieFileOutput::new() };

        let movie_output_ref: &AVCaptureOutput =
            unsafe { &*(&*movie_output as *const _ as *const AVCaptureOutput) };
        let can_add_output = unsafe { capture_session.canAddOutput(movie_output_ref) };
        if !can_add_output {
            return Err(CaptureErrorResponse {
                code: "capture_output_unavailable".to_string(),
                message: "Failed to add movie output".to_string(),
            });
        }

        unsafe { capture_session.addOutput(movie_output_ref) };

        let output_url =
            NSURL::from_file_path(&output_file).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_url_failed".to_string(),
                message: "Failed to create output URL for AVFoundation capture".to_string(),
            })?;

        let delegate_object: objc2::rc::Retained<NSObject> =
            unsafe { objc2::msg_send![recording_delegate_class(), new] };
        let delegate_key = (&*delegate_object as *const NSObject) as usize;
        let (start_tx, start_rx) = mpsc::channel::<()>();
        let (finish_tx, finish_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
        delegate_start_callbacks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(delegate_key, start_tx);
        delegate_finish_callbacks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(delegate_key, finish_tx);

        unsafe { capture_session.startRunning() };
        unsafe {
            let _: () = objc2::msg_send![
                &*movie_output,
                startRecordingToOutputFileURL: &*output_url,
                recordingDelegate: &*delegate_object
            ];
        }

        if start_rx.recv_timeout(Duration::from_secs(3)).is_err() {
            unsafe { movie_output.stopRecording() };
            unsafe { capture_session.stopRunning() };
            delegate_start_callbacks()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&delegate_key);
            delegate_finish_callbacks()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&delegate_key);
            return Err(CaptureErrorResponse {
                code: "capture_start_timeout".to_string(),
                message: "AVFoundation movie output did not transition to recording in time"
                    .to_string(),
            });
        }

        Ok(StartedCaptureSession {
            session: ActiveCaptureSession {
                backend: CaptureBackendSession::AvFoundation(AvFoundationCaptureSession {
                    capture_session,
                    movie_output,
                    _delegate: delegate_object,
                    delegate_key,
                    finish_rx,
                }),
            },
            recording_file: output_file_str,
            system_audio_recording_file: None,
            output_files,
        })
    })();

    finalize_startup_result(start_result, &session_dir)
}

#[cfg(target_os = "macos")]
fn start_screen_capture_kit_session(
    session_dir: &Path,
    screen_output_file: Option<&Path>,
    system_audio_output_path: Option<&Path>,
    sources: &ScreenCaptureSources,
    screen_frame_rate: u32,
    screen_resolution: &ScreenResolution,
    video_bitrate_bps: Option<u32>,
    options: ScreenCaptureSessionOptions,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    use cidre::{api, sc};

    let system_audio_writer_active = system_audio_writer_active_for_options(sources, &options);

    if !api::version!(macos = 15.0) {
        let error = CaptureErrorResponse {
            code: "screen_capture_kit_unsupported".to_string(),
            message: "ScreenCaptureKit recording requires macOS 15.0 or newer".to_string(),
        };
        log_capture_error(
            "ScreenCaptureKit backend is unavailable during startup",
            &error,
        );
        return Err(error);
    }

    create_session_dir(session_dir).map_err(|error| {
        log_capture_error(
            "failed to create ScreenCaptureKit capture session directory during startup",
            &error,
        );
        error
    })?;

    let start_result = (|| {
        let output_file = screen_output_file
            .map(Path::to_path_buf)
            .unwrap_or_else(|| session_dir.join("screen.mov"));
        let output_file_str = output_file.to_string_lossy().to_string();
        let system_audio_output_file = system_audio_writer_active
            .then(|| system_audio_output_path.map(|p| p.to_string_lossy().to_string()))
            .flatten();

        let active_system_audio_output_path = system_audio_writer_active
            .then_some(system_audio_output_path)
            .flatten();
        let mut output_files =
            output_files_for_session(&session_dir, active_system_audio_output_path, sources);
        if sources.screen && screen_output_file.is_some() {
            output_files.screen_file = Some(output_file_str.clone());
            output_files.screen_files = vec![output_file_str.clone()];
        }

        let (content_tx, content_rx) =
            mpsc::channel::<Result<cidre::arc::R<sc::ShareableContent>, CaptureErrorResponse>>();
        sc::ShareableContent::current_with_ch(move |content, error| {
            let result = if let Some(content) = content {
                Ok(content.retained())
            } else if let Some(error) = error {
                Err(error_with_ns_error(
                    "capture_shareable_content_failed",
                    "Failed to query ScreenCaptureKit shareable content",
                    error,
                ))
            } else {
                Err(CaptureErrorResponse {
                    code: "capture_shareable_content_unavailable".to_string(),
                    message: "No ScreenCaptureKit shareable content available".to_string(),
                })
            };
            let _ = content_tx.send(result);
        });

        let content = match content_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result?,
            Err(_) => {
                return Err(CaptureErrorResponse {
                    code: "capture_shareable_content_timeout".to_string(),
                    message: "Timed out while querying ScreenCaptureKit shareable content"
                        .to_string(),
                });
            }
        };

        let displays = content.displays();
        let display = displays.first().ok_or_else(|| CaptureErrorResponse {
            code: "capture_display_unavailable".to_string(),
            message: "No display available for ScreenCaptureKit capture".to_string(),
        })?;

        let initial_privacy_filter = options.initial_privacy_filter.clone();
        let (filter, privacy_filter_state) =
            if let Some(initial_privacy_filter) = &initial_privacy_filter {
                let built_filter = build_screen_capture_kit_content_filter(initial_privacy_filter)
                    .map_err(|error| CaptureErrorResponse {
                        code: "privacy_filter_apply_failed".to_string(),
                        message: error.message,
                    })?;
                log_privacy_filter_diagnostics(&built_filter.state);
                (built_filter.filter, Some(built_filter.state))
            } else {
                (
                    screen_capture_kit_no_exclusion_filter(display, &content.apps()),
                    None,
                )
            };

        let stream_resolution = resolve_stream_resolution(
            screen_resolution,
            display.width().max(1) as u32,
            display.height().max(1) as u32,
        );

        let screen_stream_cfg = configured_screen_capture_kit_stream_cfg(
            &stream_resolution,
            screen_frame_rate,
            sources,
        );

        let lifecycle_state = Arc::new(Mutex::new(ScreenCaptureKitLifecycleState::default()));
        let stream_delegate = ScStreamDelegate::with(ScStreamDelegateState {
            lifecycle_state: lifecycle_state.clone(),
        });
        let stream =
            cidre::sc::Stream::with_delegate(&filter, &screen_stream_cfg, stream_delegate.as_ref());
        let stream_output_queue = dispatch::Queue::serial_with_ar_pool();
        let stream_output_delegate =
            ScStreamOutputDelegate::with(stream_output_context_for_segment(
                session_dir,
                &output_file_str,
                system_audio_output_file.as_deref(),
                sources,
                system_audio_writer_active,
                video_bitrate_bps,
                options.frame_export.clone(),
                options.system_audio_inactivity_tail_trim_seconds,
            )?);

        if sources.screen {
            stream
                .add_stream_output(
                    stream_output_delegate.as_ref(),
                    sc::OutputType::Screen,
                    Some(&stream_output_queue),
                )
                .map_err(|error| {
                    error_with_ns_error(
                        "capture_stream_output_attach_failed",
                        "Failed to attach ScreenCaptureKit screen output",
                        error,
                    )
                })?;
        }

        if sources.system_audio {
            stream
                .add_stream_output(
                    stream_output_delegate.as_ref(),
                    sc::OutputType::Audio,
                    Some(&stream_output_queue),
                )
                .map_err(|error| {
                    error_with_ns_error(
                        "capture_stream_output_attach_failed",
                        "Failed to attach ScreenCaptureKit system audio output",
                        error,
                    )
                })?;
        }

        start_screen_capture_kit_stream(&stream)?;

        Ok(StartedCaptureSession {
            session: ActiveCaptureSession {
                backend: CaptureBackendSession::ScreenCaptureKit(ScreenCaptureKitCaptureSession {
                    stream,
                    _stream_delegate: stream_delegate,
                    stream_output_delegate,
                    stream_output_queue,
                    sources: *sources,
                    video_bitrate_bps,
                    frame_export: options.frame_export,
                    system_audio_inactivity_tail_buffer_seconds: options
                        .system_audio_inactivity_tail_trim_seconds,
                    lifecycle_state,
                    privacy_filter_state,
                }),
            },
            recording_file: output_file_str,
            system_audio_recording_file: system_audio_output_file,
            output_files,
        })
    })();

    finalize_startup_result(start_result, &session_dir)
}

#[cfg(target_os = "macos")]
fn configured_screen_capture_kit_stream_cfg(
    stream_resolution: &ScreenCaptureResolution,
    screen_frame_rate: u32,
    sources: &ScreenCaptureSources,
) -> cidre::arc::R<cidre::sc::StreamCfg> {
    use cidre::{cm, sc};

    let mut screen_stream_cfg = sc::StreamCfg::new();
    screen_stream_cfg.set_width(stream_resolution.width as usize);
    screen_stream_cfg.set_height(stream_resolution.height as usize);
    screen_stream_cfg.set_minimum_frame_interval(cm::Time::new(1, screen_frame_rate as i32));
    // Request a packed 32-bit buffer so live captured-frame equivalence can read
    // interleaved pixels directly instead of falling back to the exported JPEG.
    screen_stream_cfg.set_pixel_format(cidre::cv::PixelFormat::_32_BGRA);
    screen_stream_cfg.set_shows_cursor(sources.screen);
    screen_stream_cfg.set_captures_audio(sources.system_audio);
    screen_stream_cfg.set_capture_mic(false);
    if sources.system_audio {
        screen_stream_cfg.set_sample_rate(48_000);
        screen_stream_cfg.set_channel_count(2);
    }

    screen_stream_cfg
}

#[cfg(target_os = "macos")]
fn stream_output_context_for_segment(
    session_dir: &Path,
    recording_file: &str,
    system_audio_recording_file: Option<&str>,
    sources: &ScreenCaptureSources,
    system_audio_writer_active: bool,
    video_bitrate_bps: Option<u32>,
    frame_export: Option<ScreenFrameExportConfig>,
    system_audio_inactivity_tail_buffer_seconds: u64,
) -> Result<StreamOutputContext, CaptureErrorResponse> {
    use cidre::ns;

    let screen_video_output_file = if sources.screen {
        Some(recording_file.to_string())
    } else {
        None
    };
    let screen_video_writer = None;

    let system_audio_writer = if system_audio_writer_active {
        let output_file = system_audio_recording_file.ok_or_else(|| CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Missing system audio output file while creating segment writer".to_string(),
        })?;
        let output_url = ns::Url::with_fs_path_str(output_file, false);
        let mut writer = create_audio_asset_writer(&output_url, "system audio")?;
        set_audio_writer_inactivity_tail_trim_seconds(
            &mut writer,
            system_audio_inactivity_tail_buffer_seconds,
        );
        Some(writer)
    } else {
        None
    };

    Ok(StreamOutputContext {
        screen_video_output_file,
        screen_video_writer,
        video_bitrate_bps,
        system_audio_output_file: system_audio_recording_file.map(str::to_owned),
        system_audio_writer,
        system_audio_tail_trim_seconds: 0,
        system_audio_inactivity_tail_buffer_seconds,
        frame_export: if sources.screen {
            screen_frame_export_runtime(session_dir, frame_export)?
        } else {
            None
        },
        first_error: None,
    })
}

#[cfg(target_os = "macos")]
fn start_screen_capture_kit_stream(stream: &cidre::sc::Stream) -> Result<(), CaptureErrorResponse> {
    let (start_tx, start_rx) = mpsc::channel::<Result<(), CaptureErrorResponse>>();
    let mut completion = cidre::blocks::ErrCh::new1(move |error| {
        let _ = start_tx.send(match error {
            Some(error) => Err(error_with_ns_error(
                "capture_stream_start_failed",
                "Failed to start ScreenCaptureKit capture",
                error,
            )),
            None => Ok(()),
        });
    });

    stream.start_with_ch_block(Some(&mut completion));

    match start_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(_) => Err(CaptureErrorResponse {
            code: "capture_stream_start_timeout".to_string(),
            message: "Timed out while starting ScreenCaptureKit stream capture".to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
fn synchronize_stream_output_queue(queue: Option<&dispatch::Queue>) {
    if let Some(queue) = queue {
        queue.sync(|| ());
    }
}

#[cfg(target_os = "macos")]
fn finalize_screen_frame_export(
    screen_video_output_file: Option<&str>,
    frame_export: Option<&mut ScreenFrameExportRuntime>,
) -> Result<(), CaptureErrorResponse> {
    let Some(frame_export) = frame_export else {
        return Ok(());
    };

    synchronize_stream_output_queue(Some(frame_export.callback_queue.as_ref()));

    if let Some(screen_video_output_file) = screen_video_output_file {
        if let Err(error) =
            persist_screen_segment_frame_index(screen_video_output_file, frame_export)
        {
            log_capture_error("ScreenCaptureKit frame index finalization failed", &error);
        }
    }

    if let Some(error) = take_frame_export_error(&frame_export.first_error) {
        log_capture_error("ScreenCaptureKit frame export finalization failed", &error);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn finalize_secondary_stream_outputs(
    screen_video_output_file: Option<&str>,
    system_audio_output_file: Option<&str>,
    system_audio_writer: Option<&mut AudioAssetWriterState>,
    system_audio_tail_trim_seconds: u64,
    frame_export: Option<&mut ScreenFrameExportRuntime>,
) -> Result<(), CaptureErrorResponse> {
    let mut failures = Vec::new();

    if let Some(writer) = system_audio_writer {
        set_audio_writer_inactivity_tail_trim_seconds(writer, system_audio_tail_trim_seconds);
        let result = if system_audio_tail_trim_seconds > 0 {
            finish_audio_asset_writer_discarding_inactivity_tail(writer)
        } else {
            finish_audio_asset_writer(writer)
        };
        if let Err(error) = result {
            if capture_writers::is_no_audio_samples_error_message("system audio", &error.message) {
                if let Some(path) = system_audio_output_file {
                    maybe_remove_system_audio_file(path);
                }
            }
            failures.push(format!("system audio writer failed: {}", error.message));
        }
    }

    finalize_screen_frame_export(screen_video_output_file, frame_export)?;

    capture_writers::aggregate_output_processing_failures(failures)
}

#[cfg(target_os = "macos")]
fn finalize_stream_output_context_impl<
    FinalizeScreen,
    ValidateScreen,
    FinalizeSecondary,
    RemoveScreen,
    LogSecondary,
>(
    screen_video_output_file: Option<&str>,
    screen_video_writer_present: bool,
    first_error: Option<CaptureErrorResponse>,
    finalize_screen_video: FinalizeScreen,
    validate_screen_video: ValidateScreen,
    finalize_secondary_outputs: FinalizeSecondary,
    mut remove_screen_video: RemoveScreen,
    log_secondary_failure: LogSecondary,
) -> Result<(), CaptureErrorResponse>
where
    FinalizeScreen: FnOnce(Option<CaptureErrorResponse>) -> Result<(), CaptureErrorResponse>,
    ValidateScreen: FnOnce(&str) -> Result<(), CaptureErrorResponse>,
    FinalizeSecondary: FnOnce() -> Result<(), CaptureErrorResponse>,
    RemoveScreen: FnMut(&str),
    LogSecondary: FnOnce(&CaptureErrorResponse),
{
    if screen_video_output_file.is_some() && !screen_video_writer_present {
        if let Some(path) = screen_video_output_file {
            remove_screen_video(path);
        }

        if let Some(error) = first_error {
            return Err(error);
        }
        return Err(capture_writers::no_video_samples_error("screen"));
    }

    if let Err(error) = finalize_screen_video(first_error) {
        if let Some(path) = screen_video_output_file {
            remove_screen_video(path);
        }
        return Err(error);
    }

    if let Some(path) = screen_video_output_file {
        if let Err(error) = validate_screen_video(path) {
            remove_screen_video(path);
            return Err(error);
        }
    }

    if let Err(error) = finalize_secondary_outputs() {
        log_secondary_failure(&error);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn finalize_stream_output_context(
    context: &mut StreamOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let mut screen_video_writer = context.screen_video_writer.take();
    let mut system_audio_writer = context.system_audio_writer.take();
    let result = finalize_stream_output_context_impl(
        context.screen_video_output_file.as_deref(),
        screen_video_writer.is_some(),
        context.first_error.take(),
        |first_error| {
            writers_finalize_screen_video_output_context(screen_video_writer.as_mut(), first_error)
        },
        validate_screen_video_file,
        || {
            finalize_secondary_stream_outputs(
                context.screen_video_output_file.as_deref(),
                context.system_audio_output_file.as_deref(),
                system_audio_writer.as_mut(),
                context.system_audio_tail_trim_seconds,
                context.frame_export.as_mut(),
            )
        },
        maybe_remove_screen_video_file,
        |error| {
            log_capture_error(
                "ScreenCaptureKit secondary output finalization failed after preserving screen recording",
                error,
            )
        },
    );

    drop(screen_video_writer);
    drop(system_audio_writer);

    result
}

#[cfg(target_os = "macos")]
fn finalize_rotated_segment_context(
    context: &mut StreamOutputContext,
) -> Result<(), CaptureErrorResponse> {
    let screen_video_output_file = context.screen_video_output_file.clone();
    finalize_rotated_segment_context_impl(
        screen_video_output_file.as_deref(),
        || finalize_stream_output_context(context),
        |path| {
            maybe_remove_screen_video_file(path);
            !Path::new(path).exists()
        },
    )
}

#[cfg(target_os = "macos")]
fn finalize_rotated_segment_context_impl<Finalize, Remove>(
    screen_video_output_file: Option<&str>,
    finalize: Finalize,
    mut remove_screen_video: Remove,
) -> Result<(), CaptureErrorResponse>
where
    Finalize: FnOnce() -> Result<(), CaptureErrorResponse>,
    Remove: FnMut(&str) -> bool,
{
    match finalize() {
        Err(error) if should_recover_from_segment_finalize_error(&error) => {
            if let Some(path) = screen_video_output_file {
                if !remove_screen_video(path) {
                    let error = CaptureErrorResponse {
                        code: "capture_output_processing_failed".to_string(),
                        message: format!(
                            "Recovered rotated segment finalization failure but failed to remove invalid screen artifact at {path}"
                        ),
                    };
                    log_capture_error(
                        "failed to remove invalid screen artifact after rotated segment recovery",
                        &error,
                    );
                    return Err(error);
                }
            }
            log_capture_error(
                "recovered from ScreenCaptureKit rotated segment finalization failure",
                &error,
            );
            Ok(())
        }
        Err(error) => {
            log_capture_error(
                "failed to finalize rotated ScreenCaptureKit segment",
                &error,
            );
            Err(error)
        }
        Ok(()) => Ok(()),
    }
}

#[cfg(target_os = "macos")]
pub struct RotateScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub segment_dir: &'a Path,
    /// Visible dated output path for the screen recording.
    /// When `Some`, the video file is written here instead of `segment_dir/screen.mov`.
    pub screen_output_file: Option<&'a Path>,
    /// Full output path for the system-audio file in the new segment, or `None`
    /// when system audio is not being captured.
    pub system_audio_output_path: Option<&'a Path>,
}

#[cfg(target_os = "macos")]
pub fn rotate_screen_capture_session(
    args: RotateScreenCaptureSessionArgs<'_>,
) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
    let Some(session) = args.active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "Missing active screen capture session for segment rotation".to_string(),
        });
    };

    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(session) => session.rotate_output_files(
            args.segment_dir,
            args.screen_output_file,
            args.system_audio_output_path,
        ),
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "capture_rotation_requires_restart".to_string(),
            message: "This capture backend requires full restart for segment rotation".to_string(),
        }),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn update_active_privacy_filter(
    _active_session: &mut Option<ActiveCaptureSession>,
    _filter: PrivacyContentFilter,
) -> Result<(), PrivacyFilterApplyError> {
    Err(PrivacyFilterApplyError {
        kind: PrivacyFilterApplyErrorKind::UnsupportedPlatform,
        message: "Privacy content filters require ScreenCaptureKit".to_string(),
    })
}

#[cfg(target_os = "macos")]
pub fn update_active_privacy_filter(
    active_session: &mut Option<ActiveCaptureSession>,
    filter: PrivacyContentFilter,
) -> Result<(), PrivacyFilterApplyError> {
    let Some(session) = active_session.as_mut() else {
        return Err(PrivacyFilterApplyError {
            kind: PrivacyFilterApplyErrorKind::FilterUpdateFailed,
            message: "Missing active screen capture session for privacy filter update".to_string(),
        });
    };
    session.update_privacy_filter(filter)
}

/// Finalize and disable the system-audio writer for an active ScreenCaptureKit
/// session without stopping the screen capture stream.  Returns `Ok(())` if
/// there was no writer to pause (idempotent).
#[cfg(target_os = "macos")]
pub fn pause_system_audio_writer(
    active_session: &mut Option<ActiveCaptureSession>,
) -> Result<(), CaptureErrorResponse> {
    pause_system_audio_writer_for_inactivity(active_session, 0)
}

#[cfg(target_os = "macos")]
pub fn pause_system_audio_writer_for_inactivity(
    active_session: &mut Option<ActiveCaptureSession>,
    tail_trim_seconds: u64,
) -> Result<(), CaptureErrorResponse> {
    let Some(session) = active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "No active screen capture session for system audio pause".to_string(),
        });
    };
    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(sck) => {
            sck.pause_system_audio_writer_for_inactivity(tail_trim_seconds)
        }
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "system_audio_pause_unsupported".to_string(),
            message: "System audio soft-pause is only supported on the ScreenCaptureKit backend"
                .to_string(),
        }),
    }
}

/// Create and attach a new system-audio writer to an active ScreenCaptureKit
/// session that was previously paused.  The caller supplies the new output path.
#[cfg(target_os = "macos")]
pub fn resume_system_audio_writer(
    active_session: &mut Option<ActiveCaptureSession>,
    output_path: &str,
) -> Result<(), CaptureErrorResponse> {
    let Some(session) = active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "No active screen capture session for system audio resume".to_string(),
        });
    };
    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(sck) => sck.resume_system_audio_writer(output_path),
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "system_audio_resume_unsupported".to_string(),
            message: "System audio soft-resume is only supported on the ScreenCaptureKit backend"
                .to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub fn pause_screen_outputs_for_inactivity(
    active_session: &mut Option<ActiveCaptureSession>,
) -> Result<(), CaptureErrorResponse> {
    let Some(session) = active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "No active screen capture session for screen output pause".to_string(),
        });
    };
    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(sck) => sck.pause_screen_outputs_for_inactivity(),
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "screen_pause_unsupported".to_string(),
            message: "Screen soft-pause is only supported on the ScreenCaptureKit backend"
                .to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub fn resume_screen_outputs(
    active_session: &mut Option<ActiveCaptureSession>,
    segment_dir: &Path,
    recording_file: &str,
) -> Result<(), CaptureErrorResponse> {
    let Some(session) = active_session.as_mut() else {
        return Err(CaptureErrorResponse {
            code: "invalid_runtime_state".to_string(),
            message: "No active screen capture session for screen output resume".to_string(),
        });
    };
    match &mut session.backend {
        CaptureBackendSession::ScreenCaptureKit(sck) => {
            sck.resume_screen_outputs(segment_dir, recording_file)
        }
        CaptureBackendSession::AvFoundation(_) => Err(CaptureErrorResponse {
            code: "screen_resume_unsupported".to_string(),
            message: "Screen soft-resume is only supported on the ScreenCaptureKit backend"
                .to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub struct StopScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub inactivity_tail_trim_seconds: u64,
}

#[cfg(target_os = "macos")]
pub fn stop_screen_capture_session(
    args: StopScreenCaptureSessionArgs<'_>,
) -> Result<(), CaptureErrorResponse> {
    let mut stop_error: Option<CaptureErrorResponse> = None;

    if let Some(session) = args.active_session.as_mut() {
        match if args.inactivity_tail_trim_seconds > 0 {
            session.stop_for_inactivity(args.inactivity_tail_trim_seconds)
        } else {
            session.stop()
        } {
            Ok(()) => {
                *args.active_session = None;
            }
            Err(error)
                if ScreenCaptureKitCaptureSession::is_stop_timeout_code(error.code.as_str()) =>
            {
                return Err(error);
            }
            Err(error) => {
                stop_error = Some(error);
                *args.active_session = None;
            }
        }
    }

    if let Some(error) = stop_error {
        Err(error)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
pub fn screen_permission_state() -> CapturePermissionState {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        CapturePermissionState::Granted
    } else if SCREEN_PERMISSION_REQUESTED.load(Ordering::SeqCst) {
        CapturePermissionState::Denied
    } else {
        CapturePermissionState::Unknown
    }
}

#[cfg(target_os = "macos")]
pub fn ensure_screen_permission() -> bool {
    SCREEN_PERMISSION_REQUESTED.store(true, Ordering::SeqCst);
    unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
pub fn system_audio_permission_state() -> CapturePermissionState {
    if supports_system_audio_capture() {
        screen_permission_state()
    } else {
        CapturePermissionState::Unsupported
    }
}

#[cfg(target_os = "macos")]
pub fn supports_system_audio_capture() -> bool {
    cidre::api::version!(macos = 15.0)
}

#[cfg(target_os = "macos")]
pub fn should_preserve_runtime_on_stop_error(error: &CaptureErrorResponse) -> bool {
    ScreenCaptureKitCaptureSession::is_stop_timeout_code(error.code.as_str())
}

#[cfg(target_os = "macos")]
pub fn screen_capture_session_is_live(active_session: Option<&ActiveCaptureSession>) -> bool {
    active_session.is_some_and(ActiveCaptureSession::is_screen_stream_live)
}

#[cfg(target_os = "macos")]
pub fn take_screen_capture_session_stop_error(
    active_session: Option<&mut ActiveCaptureSession>,
) -> Option<CaptureErrorResponse> {
    active_session.and_then(ActiveCaptureSession::take_screen_stop_error)
}

#[cfg(target_os = "macos")]
fn supports_screen_capture_kit_backend() -> bool {
    cidre::api::version!(macos = 15.0)
}

pub fn supports_frame_export() -> bool {
    #[cfg(target_os = "macos")]
    {
        supports_screen_capture_kit_backend()
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(not(target_os = "macos"))]
pub struct ActiveCaptureSession;

#[cfg(not(target_os = "macos"))]
pub struct StartedCaptureSession {
    pub session: ActiveCaptureSession,
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(not(target_os = "macos"))]
pub struct RotatedCaptureOutputs {
    pub recording_file: String,
    pub system_audio_recording_file: Option<String>,
    pub output_files: CaptureOutputFiles,
}

#[cfg(not(target_os = "macos"))]
pub struct StopScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub inactivity_tail_trim_seconds: u64,
}

#[cfg(not(target_os = "macos"))]
pub struct RotateScreenCaptureSessionArgs<'a> {
    pub active_session: &'a mut Option<ActiveCaptureSession>,
    pub segment_dir: &'a Path,
    pub screen_output_file: Option<&'a Path>,
    pub system_audio_output_path: Option<&'a Path>,
}

#[cfg(not(target_os = "macos"))]
pub fn new_session_id() -> Result<String, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn start_capture_session(
    _session_dir: &Path,
    _sources: &ScreenCaptureSources,
    _screen_frame_rate: u32,
    _screen_resolution: &ScreenResolution,
    _video_bitrate_bps: Option<u32>,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn start_capture_session_with_options(
    _session_dir: &Path,
    _screen_output_file: Option<&Path>,
    _system_audio_output_path: Option<&Path>,
    _sources: &ScreenCaptureSources,
    _screen_frame_rate: u32,
    _screen_resolution: &ScreenResolution,
    _video_bitrate_bps: Option<u32>,
    _options: ScreenCaptureSessionOptions,
) -> Result<StartedCaptureSession, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn pause_system_audio_writer(
    _active_session: &mut Option<ActiveCaptureSession>,
) -> Result<(), CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "System audio soft-pause is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn resume_system_audio_writer(
    _active_session: &mut Option<ActiveCaptureSession>,
    _output_path: &str,
) -> Result<(), CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "System audio soft-resume is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn pause_screen_outputs_for_inactivity(
    _active_session: &mut Option<ActiveCaptureSession>,
) -> Result<(), CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Screen soft-pause is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn resume_screen_outputs(
    _active_session: &mut Option<ActiveCaptureSession>,
    _segment_dir: &Path,
    _recording_file: &str,
) -> Result<(), CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Screen soft-resume is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn stop_screen_capture_session(
    _args: StopScreenCaptureSessionArgs<'_>,
) -> Result<(), CaptureErrorResponse> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn rotate_screen_capture_session(
    _args: RotateScreenCaptureSessionArgs<'_>,
) -> Result<RotatedCaptureOutputs, CaptureErrorResponse> {
    Err(CaptureErrorResponse {
        code: "unsupported_platform".to_string(),
        message: "Native capture is currently supported only on macOS".to_string(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn screen_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_screen_permission() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn system_audio_permission_state() -> CapturePermissionState {
    CapturePermissionState::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub fn supports_system_audio_capture() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn should_preserve_runtime_on_stop_error(_error: &CaptureErrorResponse) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn should_recover_from_segment_finalize_error(_error: &CaptureErrorResponse) -> bool {
    false
}

pub fn support_for_current_platform() -> ScreenCaptureSupport {
    #[cfg(target_os = "macos")]
    {
        ScreenCaptureSupport {
            platform: "macos".to_string(),
            native_capture_supported: true,
            screen: true,
            system_audio: supports_system_audio_capture(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        ScreenCaptureSupport {
            platform: std::env::consts::OS.to_string(),
            native_capture_supported: false,
            screen: false,
            system_audio: false,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn last_screen_activity_unix_ms() -> Option<u64> {
    let ts = LAST_SCREEN_ACTIVITY_UNIX_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(ts)
}

#[cfg(target_os = "macos")]
pub fn screen_activity_idle_ms() -> Option<u64> {
    let ts = LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(now_monotonic_marker_ms().saturating_sub(ts))
}

#[cfg(target_os = "macos")]
pub fn last_system_audio_activity_unix_ms() -> Option<u64> {
    let ts = LAST_SYSTEM_AUDIO_ACTIVITY_UNIX_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(ts)
}

#[cfg(target_os = "macos")]
pub fn system_audio_activity_idle_ms() -> Option<u64> {
    let ts = LAST_SYSTEM_AUDIO_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);
    (ts > 0).then_some(now_monotonic_marker_ms().saturating_sub(ts))
}

#[cfg(target_os = "macos")]
pub fn system_audio_activity_level() -> Option<f32> {
    last_system_audio_activity_unix_ms()
        .map(|_| f32::from_bits(LAST_SYSTEM_AUDIO_ACTIVITY_LEVEL_BITS.load(Ordering::Relaxed)))
}

#[cfg(target_os = "macos")]
pub fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    let sample_count = LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_SAMPLE_COUNT.swap(0, Ordering::Relaxed);
    let level_bits = LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.swap(0, Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

#[cfg(target_os = "macos")]
pub fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    let sample_count = LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_SAMPLE_COUNT.load(Ordering::Relaxed);
    let level_bits = LAST_SYSTEM_AUDIO_ACTIVITY_WINDOW_PEAK_LEVEL_BITS.load(Ordering::Relaxed);
    (sample_count > 0).then_some(f32::from_bits(level_bits))
}

#[cfg(target_os = "macos")]
pub fn record_system_audio_activity_for_tests(level: f32, now_monotonic_ms: u64, now_unix_ms: u64) {
    store_system_audio_activity(level, now_monotonic_ms, now_unix_ms);
}

#[cfg(not(target_os = "macos"))]
pub fn take_system_audio_activity_window_peak_level() -> Option<f32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn peek_system_audio_activity_window_peak_level() -> Option<f32> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn record_system_audio_activity_for_tests(
    _level: f32,
    _now_monotonic_ms: u64,
    _now_unix_ms: u64,
) {
}

#[cfg(not(target_os = "macos"))]
pub fn last_screen_activity_unix_ms() -> Option<u64> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn screen_activity_idle_ms() -> Option<u64> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn last_system_audio_activity_unix_ms() -> Option<u64> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn system_audio_activity_idle_ms() -> Option<u64> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn system_audio_activity_level() -> Option<f32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::process::Command;

    #[test]
    fn mixed_app_and_window_privacy_plan_expands_excluded_app_windows() {
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let requested_window_ids = std::collections::BTreeSet::from([42]);
        let available_apps = vec![
            PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
            },
            PrivacyFilterAvailableApp {
                bundle_id: "com.google.Chrome".to_string(),
            },
        ];
        let available_windows = vec![
            PrivacyFilterAvailableWindow {
                id: 7,
                owning_bundle_id: Some("com.secret".to_string()),
            },
            PrivacyFilterAvailableWindow {
                id: 42,
                owning_bundle_id: Some("com.google.Chrome".to_string()),
            },
            PrivacyFilterAvailableWindow {
                id: 99,
                owning_bundle_id: Some("com.google.Chrome".to_string()),
            },
        ];

        let plan = plan_privacy_content_filter(
            &requested_bundle_ids,
            &requested_window_ids,
            &available_apps,
            &available_windows,
        );

        assert_eq!(
            plan.strategy,
            PrivacyContentFilterStrategy::ExcludingAppWindowsAndWindows
        );
        assert_eq!(
            plan.excluded_bundle_ids,
            std::collections::BTreeSet::from(["com.secret".to_string()])
        );
        assert_eq!(
            plan.excluded_window_ids,
            std::collections::BTreeSet::from([7, 42])
        );
    }

    #[test]
    fn unresolved_privacy_filter_targets_do_not_satisfy_cached_request() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
            excluded_window_ids: vec![42],
        };
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let requested_window_ids = std::collections::BTreeSet::from([42]);

        let unresolved_plan =
            plan_privacy_content_filter(&requested_bundle_ids, &requested_window_ids, &[], &[]);
        let unresolved_state =
            PrivacyFilterApplicationState::from_plan(&requested, &unresolved_plan);

        assert_eq!(
            unresolved_state.unresolved_bundle_ids,
            vec!["com.secret".to_string()]
        );
        assert_eq!(unresolved_state.unresolved_window_ids, vec![42]);
        assert!(!unresolved_state.satisfies_request(&requested.key()));

        let resolved_plan = plan_privacy_content_filter(
            &requested_bundle_ids,
            &requested_window_ids,
            &[PrivacyFilterAvailableApp {
                bundle_id: "com.secret".to_string(),
            }],
            &[PrivacyFilterAvailableWindow {
                id: 42,
                owning_bundle_id: Some("com.example.visible".to_string()),
            }],
        );
        let resolved_state = PrivacyFilterApplicationState::from_plan(&requested, &resolved_plan);

        assert!(!resolved_state.satisfies_request(&requested.key()));
        assert!(!unresolved_state.applied_filter_matches(&resolved_state));
    }

    #[test]
    fn repeated_unresolved_privacy_filter_plan_suppresses_duplicate_diagnostics() {
        let requested = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
            excluded_window_ids: vec![42],
        };
        let requested_bundle_ids = std::collections::BTreeSet::from(["com.secret".to_string()]);
        let requested_window_ids = std::collections::BTreeSet::from([42]);
        let plan = plan_privacy_content_filter(
            &requested_bundle_ids,
            &requested_window_ids,
            &[],
            &[PrivacyFilterAvailableWindow {
                id: 42,
                owning_bundle_id: Some("com.example.visible".to_string()),
            }],
        );
        let previous = PrivacyFilterApplicationState::from_plan(&requested, &plan);
        let next = PrivacyFilterApplicationState::from_plan(&requested, &plan);

        assert!(should_log_privacy_filter_diagnostics(None, &next));
        assert!(!should_log_privacy_filter_diagnostics(
            Some(&previous),
            &next
        ));
    }

    #[test]
    fn privacy_filter_diagnostics_log_when_unresolved_targets_change() {
        let previous_request = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.secret".to_string()],
            excluded_window_ids: vec![42],
        };
        let next_request = PrivacyContentFilter {
            display_id: 0,
            excluded_bundle_ids: vec!["com.other-secret".to_string()],
            excluded_window_ids: vec![42],
        };
        let requested_window_ids = std::collections::BTreeSet::from([42]);
        let available_windows = vec![PrivacyFilterAvailableWindow {
            id: 42,
            owning_bundle_id: Some("com.example.visible".to_string()),
        }];
        let previous_plan = plan_privacy_content_filter(
            &std::collections::BTreeSet::from(["com.secret".to_string()]),
            &requested_window_ids,
            &[],
            &available_windows,
        );
        let next_plan = plan_privacy_content_filter(
            &std::collections::BTreeSet::from(["com.other-secret".to_string()]),
            &requested_window_ids,
            &[],
            &available_windows,
        );
        let previous = PrivacyFilterApplicationState::from_plan(&previous_request, &previous_plan);
        let next = PrivacyFilterApplicationState::from_plan(&next_request, &next_plan);

        assert!(should_log_privacy_filter_diagnostics(
            Some(&previous),
            &next
        ));
    }

    #[test]
    fn screen_segment_frame_index_binary_round_trips() {
        let index = ScreenSegmentFrameIndex {
            version: SCREEN_SEGMENT_FRAME_INDEX_VERSION,
            entries: vec![
                ScreenSegmentFrameIndexEntry {
                    captured_at_unix_ms: 1,
                    frame_index: 2,
                    video_offset_ms: 3,
                },
                ScreenSegmentFrameIndexEntry {
                    captured_at_unix_ms: 4,
                    frame_index: 5,
                    video_offset_ms: 6,
                },
            ],
        };

        let bytes = encode_screen_segment_frame_index(&index);
        let decoded =
            decode_screen_segment_frame_index(&bytes).expect("binary frame index should decode");

        assert_eq!(decoded, index);
    }

    #[test]
    fn screen_segment_frame_index_offsets_monotonic_check_rejects_swapped_adjacent_offsets() {
        let entries = vec![
            ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 1,
                frame_index: 57,
                video_offset_ms: 57_067,
            },
            ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 2,
                frame_index: 58,
                video_offset_ms: 56_067,
            },
        ];

        assert!(!screen_segment_frame_index_offsets_are_monotonic(&entries));
    }

    #[test]
    fn screen_segment_frame_index_offsets_monotonic_check_accepts_increasing_offsets() {
        let entries = vec![
            ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 1,
                frame_index: 57,
                video_offset_ms: 56_067,
            },
            ScreenSegmentFrameIndexEntry {
                captured_at_unix_ms: 2,
                frame_index: 58,
                video_offset_ms: 57_067,
            },
        ];

        assert!(screen_segment_frame_index_offsets_are_monotonic(&entries));
    }

    #[cfg(target_os = "macos")]
    fn screen_activity_state_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(target_os = "macos")]
    fn stream_output_callback_error_from_panic<F>(panic_fn: F) -> CaptureErrorResponse
    where
        F: FnOnce(),
    {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(panic_fn))
            .expect_err("panic should be caught");

        stream_output_callback_panic_error(payload)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nested_objc_exception_inside_catch_unwind_does_not_abort_subprocess() {
        const ENV_NAME: &str = "CAPTURE_SCREEN_NESTED_OBJC_EXCEPTION_CHILD";

        if std::env::var_os(ENV_NAME).is_some() {
            run_nested_objc_exception_and_panic_boundary_for_test();
            return;
        }

        let output =
            Command::new(std::env::current_exe().expect("current test binary should exist"))
                .env(ENV_NAME, "1")
                .arg("--exact")
                .arg("tests::nested_objc_exception_inside_catch_unwind_does_not_abort_subprocess")
                .arg("--nocapture")
                .output()
                .expect("child process should run nested exception harness");

        assert!(
            output.status.success(),
            "nested ObjC exception should be contained without aborting child process\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_scales_preset_with_display_aspect_ratio() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P720,
            },
            2560,
            1440,
        );

        assert_eq!(resolved.width, 1280);
        assert_eq!(resolved.height, 720);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_clamps_preset_to_display_size() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Preset {
                preset: ScreenResolutionPreset::P1080,
            },
            1366,
            768,
        );

        assert_eq!(resolved.width, 1366);
        assert_eq!(resolved.height, 768);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_stream_resolution_keeps_custom_dimensions() {
        let resolved = resolve_stream_resolution(
            &ScreenResolution::Custom {
                width: 1001,
                height: 777,
            },
            1920,
            1080,
        );

        assert_eq!(resolved.width, 1002);
        assert_eq!(resolved.height, 778);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn configured_screen_capture_kit_stream_cfg_requests_bgra_pixel_format() {
        let cfg = configured_screen_capture_kit_stream_cfg(
            &ScreenCaptureResolution {
                width: 1280,
                height: 720,
            },
            1,
            &ScreenCaptureSources {
                screen: true,
                system_audio: false,
            },
        );

        assert_eq!(cfg.width(), 1280);
        assert_eq!(cfg.height(), 720);
        assert_eq!(cfg.pixel_format(), cidre::cv::PixelFormat::_32_BGRA);
    }

    // --- output_files_for_session path-layout regression ---

    #[test]
    fn output_files_screen_only_uses_session_dir() {
        let session_dir = Path::new("/recordings/2026/04/19/.session-abc-segment-0001");
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: false,
        };

        let files = output_files_for_session(session_dir, None, &sources);

        let screen_file = files.screen_file.expect("screen_file should be Some");
        assert!(
            screen_file.contains("session-abc-segment-0001"),
            "screen output should be inside the hidden segment workspace: {screen_file}"
        );
        assert!(
            !screen_file.contains("/audio/"),
            "screen output must not be inside the audio directory: {screen_file}"
        );
        assert!(
            files.system_audio_file.is_none(),
            "system_audio_file should be None when system_audio is disabled"
        );
    }

    #[test]
    fn output_files_system_audio_uses_flat_audio_dir() {
        let session_dir = Path::new("/recordings/2026/04/19/.session-abc-segment-0001");
        let system_audio_path =
            Path::new("/recordings/2026/04/19/audio/system-audio-session-abc-segment-0001.m4a");
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: true,
        };

        let files = output_files_for_session(session_dir, Some(system_audio_path), &sources);

        let audio_file = files
            .system_audio_file
            .expect("system_audio_file should be Some when system_audio is enabled");
        assert_eq!(
            audio_file, "/recordings/2026/04/19/audio/system-audio-session-abc-segment-0001.m4a",
            "system-audio output should match the provided path exactly"
        );
        assert!(
            audio_file.ends_with("system-audio-session-abc-segment-0001.m4a"),
            "system-audio filename should contain segment qualifier: {audio_file}"
        );

        let screen_file = files.screen_file.expect("screen_file should be Some");
        assert!(
            screen_file.contains("session-abc-segment-0001"),
            "screen output should remain in the hidden segment workspace: {screen_file}"
        );
        assert!(
            !screen_file.contains("/audio/"),
            "screen output must not bleed into the audio directory: {screen_file}"
        );
    }

    #[test]
    fn output_files_system_audio_path_is_separate_from_screen_workspace() {
        // The two directory roots must share no prefix relationship - the audio
        // file lives flat under dated audio/ while the segment workspace is
        // a dot-hidden sibling of the date directory.
        let session_dir = Path::new("/save/2026/04/19/.mysession-segment-0003");
        let system_audio_path =
            Path::new("/save/2026/04/19/audio/system-audio-mysession-segment-0003.m4a");
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: true,
        };

        let files = output_files_for_session(session_dir, Some(system_audio_path), &sources);

        let audio_file = files.system_audio_file.unwrap();
        let screen_file = files.screen_file.unwrap();

        // They must be in entirely different parent directories.
        let audio_path = std::path::Path::new(&audio_file);
        let screen_path = std::path::Path::new(&screen_file);
        assert_ne!(
            audio_path.parent(),
            screen_path.parent(),
            "system-audio and screen outputs must live in different directories"
        );
        assert!(
            !audio_path.starts_with(session_dir),
            "audio output must not be inside the hidden segment workspace"
        );
        assert!(
            !screen_path.starts_with("/save/2026/04/19/audio/"),
            "screen output must not be inside the audio directory"
        );
    }

    #[test]
    fn screen_frame_artifact_path_uses_timestamp_and_sequence() {
        let path =
            screen_frame_artifact_path(Path::new("/tmp/session/frames"), 42, 1_717_000_123_456);

        assert_eq!(
            path,
            Path::new("/tmp/session/frames/frame-1717000123456-000042.jpg")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_capture_kit_lifecycle_state_defaults_to_live() {
        let state = ScreenCaptureKitLifecycleState::default();

        assert!(state.stream_live);
        assert!(state.stop_error.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_capture_kit_lifecycle_state_tracks_stop_error() {
        let mut state = ScreenCaptureKitLifecycleState::default();
        state.stream_live = false;
        state.stop_error = Some(CaptureErrorResponse {
            code: "capture_stream_system_stopped".to_string(),
            message:
                "ScreenCaptureKit stream stopped unexpectedly: stopped by system (code: -3821)"
                    .to_string(),
        });

        assert!(!state.stream_live);
        let stop_error = state.stop_error.expect("stop error should be recorded");
        assert_eq!(stop_error.code, "capture_stream_system_stopped");
        assert!(stop_error.message.contains("-3821"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_callback_panic_error_formats_static_str_payload() {
        let error = stream_output_callback_error_from_panic(|| panic!("boom"));

        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "ScreenCaptureKit output callback panicked: boom"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_callback_panic_error_formats_string_payload() {
        let error = stream_output_callback_error_from_panic(|| {
            std::panic::panic_any(String::from("owned boom"));
        });

        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "ScreenCaptureKit output callback panicked: owned boom"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_callback_panic_error_handles_non_string_payloads() {
        let error = stream_output_callback_error_from_panic(|| {
            std::panic::panic_any(42_u8);
        });

        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            "ScreenCaptureKit output callback panicked with a non-string payload"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_callback_objc_exception_error_contains_expected_wording() {
        let reason = cidre::ns::str!(c"test reason");
        let exception =
            cidre::ns::try_catch(|| cidre::ns::Exception::raise(reason)).expect_err("should catch");
        let error = stream_output_callback_objc_exception_error(exception);

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(
            error
                .message
                .contains("ScreenCaptureKit output callback ObjC exception"),
            "message should contain ObjC exception wording: {}",
            error.message
        );
        assert!(
            error.message.contains("test reason"),
            "message should contain exception reason: {}",
            error.message
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_callback_objc_exception_error_includes_exception_name() {
        let reason = cidre::ns::str!(c"some reason");
        let exception =
            cidre::ns::try_catch(|| cidre::ns::Exception::raise(reason)).expect_err("should catch");
        let error = stream_output_callback_objc_exception_error(exception);

        // Exception::raise uses NSGenericException by default
        assert!(
            error.message.contains("NSGenericException"),
            "message should contain exception name: {}",
            error.message
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_append_screen_sample_with_state_accepts_complete_ready_image_samples() {
        assert!(should_append_screen_sample_with_state(
            Some(1),
            true,
            true,
            1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_append_screen_sample_with_state_preserves_missing_status_for_ready_image_samples() {
        assert!(should_append_screen_sample_with_state(None, true, true, 1));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_append_screen_sample_with_state_rejects_non_ready_samples() {
        assert!(!should_append_screen_sample_with_state(
            Some(1),
            false,
            true,
            1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_append_screen_sample_with_state_rejects_samples_without_image_buffers() {
        assert!(!should_append_screen_sample_with_state(
            Some(1),
            true,
            false,
            1
        ));
        assert!(!should_append_screen_sample_with_state(
            None, true, false, 1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_append_screen_sample_with_state_rejects_non_complete_status_samples() {
        assert!(!should_append_screen_sample_with_state(
            Some(2),
            true,
            true,
            1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn image_destination_creation_failure_message_includes_parent_context() {
        let unique = format!(
            "capture-screen-png-dst-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let base_dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&base_dir).expect("base dir should be created");
        let output_path = base_dir.join("frame.jpg");

        let message = image_destination_creation_failure_message(&output_path, "JPEG");

        assert!(message.contains(&output_path.display().to_string()));
        assert!(message.contains(&format!("parent: {}", base_dir.display())));
        assert!(message.contains("parent_exists: true"));
        assert!(message.contains("file_exists: false"));

        std::fs::remove_dir_all(&base_dir).expect("base dir should be removed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_accepts_empty_or_missing_screen_video_failures() {
        let no_samples = capture_writers::no_video_samples_error("screen");
        let aggregated_no_samples =
            capture_writers::aggregate_output_processing_failures(vec![format!(
                "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{}",
                no_samples.message
            )])
            .expect_err("single screen writer failure should aggregate");
        let empty_video = CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: FINALIZED_SCREEN_RECORDING_EMPTY_ERROR_MESSAGE.to_string(),
        };
        let aggregated_empty_video = capture_writers::aggregate_output_processing_failures(vec![
            FINALIZED_SCREEN_RECORDING_EMPTY_ERROR_MESSAGE.to_string(),
        ])
        .expect_err("single validation failure should aggregate");
        let missing_track = CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE.to_string(),
        };
        let missing_file = CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!(
                "{FINALIZED_SCREEN_RECORDING_INSPECTION_ERROR_PREFIX}No such file or directory (os error 2)"
            ),
        };
        let aggregated_missing_file =
            capture_writers::aggregate_output_processing_failures(vec![missing_file
                .message
                .clone()])
            .expect_err("single missing-file validation failure should aggregate");

        assert!(should_recover_from_segment_finalize_error(&no_samples));
        assert!(should_recover_from_segment_finalize_error(
            &aggregated_no_samples
        ));
        assert!(should_recover_from_segment_finalize_error(&empty_video));
        assert!(should_recover_from_segment_finalize_error(
            &aggregated_empty_video
        ));
        assert!(should_recover_from_segment_finalize_error(&missing_track));
        assert!(should_recover_from_segment_finalize_error(&missing_file));
        assert!(should_recover_from_segment_finalize_error(
            &aggregated_missing_file
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_accepts_single_screen_video_writer_avfoundation_11800_failure(
    ) {
        let error = capture_writers::aggregate_output_processing_failures(vec![format!(
            "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
        )])
        .expect_err("single screen-video writer AVFoundation failure should aggregate");

        assert!(should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_accepts_single_screen_video_avfoundation_11800_failure_pair(
    ) {
        let error = capture_writers::aggregate_output_processing_failures(vec![
            format!(
                "{SCREEN_STREAM_OUTPUT_PROCESSING_FAILURE_PREFIX}{SCREEN_VIDEO_APPEND_SAMPLE_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
            ),
            format!(
                "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
            ),
        ])
        .expect_err("single screen-video AVFoundation failure pair should aggregate");

        assert!(should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_rejects_single_screen_video_writer_avfoundation_11800_failure_with_extra_failures(
    ) {
        let error = capture_writers::aggregate_output_processing_failures(vec![
            format!(
                "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
            ),
            "system audio writer failed: boom".to_string(),
        ])
        .expect_err("multiple failures should aggregate");

        assert!(!should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_rejects_screen_video_avfoundation_11800_failure_pair_with_extra_failures(
    ) {
        let error = capture_writers::aggregate_output_processing_failures(vec![
            format!(
                "{SCREEN_STREAM_OUTPUT_PROCESSING_FAILURE_PREFIX}{SCREEN_VIDEO_APPEND_SAMPLE_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
            ),
            format!(
                "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
            ),
            "system audio writer failed: boom".to_string(),
        ])
        .expect_err("multiple failures should aggregate");

        assert!(!should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_rejects_other_output_failures() {
        let error = CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to finalize capture outputs: system audio writer failed: boom"
                .to_string(),
        };

        assert!(!should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_recover_from_segment_finalize_error_rejects_recoverable_screen_failure_with_other_failures(
    ) {
        let no_samples = capture_writers::no_video_samples_error("screen");
        let error = capture_writers::aggregate_output_processing_failures(vec![
            format!("{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{}", no_samples.message),
            "system audio writer failed: boom".to_string(),
        ])
        .expect_err("multiple failures should aggregate");

        assert!(!should_recover_from_segment_finalize_error(&error));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_rotated_segment_context_recovers_from_single_screen_video_writer_avfoundation_11800_failure(
    ) {
        let mut context = StreamOutputContext {
            screen_video_output_file: Some("/tmp/missing-screen-writer.mov".to_string()),
            screen_video_writer: None,
            video_bitrate_bps: None,
            system_audio_output_file: None,
            system_audio_writer: None,
            system_audio_tail_trim_seconds: 0,
            system_audio_inactivity_tail_buffer_seconds: 0,
            frame_export: None,
            first_error: Some(
                capture_writers::aggregate_output_processing_failures(vec![format!(
                    "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
                )])
                .expect_err("single screen-video writer AVFoundation failure should aggregate"),
            ),
        };

        assert!(finalize_rotated_segment_context(&mut context).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_rotated_segment_context_rejects_single_screen_video_writer_avfoundation_11800_failure_with_extra_failures(
    ) {
        let mut context = StreamOutputContext {
            screen_video_output_file: Some("/tmp/missing-screen-writer.mov".to_string()),
            screen_video_writer: None,
            video_bitrate_bps: None,
            system_audio_output_file: None,
            system_audio_writer: None,
            system_audio_tail_trim_seconds: 0,
            system_audio_inactivity_tail_buffer_seconds: 0,
            frame_export: None,
            first_error: Some(
                capture_writers::aggregate_output_processing_failures(vec![
                    format!(
                        "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
                    ),
                    "system audio writer failed: boom".to_string(),
                ])
                .expect_err("multiple failures should aggregate"),
            ),
        };

        assert!(finalize_rotated_segment_context(&mut context).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_rotated_segment_context_recovers_from_missing_screen_video() {
        let mut context = StreamOutputContext {
            screen_video_output_file: Some("/tmp/missing-screen.mov".to_string()),
            screen_video_writer: None,
            video_bitrate_bps: None,
            system_audio_output_file: None,
            system_audio_writer: None,
            system_audio_tail_trim_seconds: 0,
            system_audio_inactivity_tail_buffer_seconds: 0,
            frame_export: None,
            first_error: None,
        };

        assert!(finalize_rotated_segment_context(&mut context).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_rotated_segment_context_keeps_recoverable_failures_fatal_when_invalid_screen_artifact_survives_cleanup(
    ) {
        let error = finalize_rotated_segment_context_impl(
            Some("/tmp/stale-invalid-screen.mov"),
            || {
                Err(capture_writers::aggregate_output_processing_failures(vec![
                    format!(
                        "{SCREEN_VIDEO_WRITER_FAILURE_PREFIX}{SCREEN_VIDEO_FINALIZE_ASSET_WRITER_FAILURE_PREFIX}The operation could not be completed {AVFOUNDATION_FAILURE_CODE_11800_SUFFIX}"
                    ),
                ])
                .expect_err("single recoverable failure should aggregate"))
            },
            |_path| false,
        )
        .expect_err("rotated recovery must fail when invalid artifact cleanup fails");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert!(error
            .message
            .contains("failed to remove invalid screen artifact at /tmp/stale-invalid-screen.mov"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_rotated_segment_context_keeps_nonrecoverable_failures_fatal() {
        let mut context = StreamOutputContext {
            screen_video_output_file: Some("/tmp/missing-screen.mov".to_string()),
            screen_video_writer: None,
            video_bitrate_bps: None,
            system_audio_output_file: None,
            system_audio_writer: None,
            system_audio_tail_trim_seconds: 0,
            system_audio_inactivity_tail_buffer_seconds: 0,
            frame_export: None,
            first_error: Some(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "boom".to_string(),
            }),
        };

        let error = finalize_rotated_segment_context(&mut context)
            .expect_err("unexpected finalization failures must remain fatal");
        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(error.message, "boom");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_stream_output_context_keeps_valid_screen_video_when_system_audio_has_zero_samples()
    {
        let mut removed_paths = Vec::new();
        let mut logged_errors = Vec::new();

        let result = finalize_stream_output_context_impl(
            Some("/tmp/valid-screen.mov"),
            true,
            None,
            |_| Ok(()),
            |_| Ok(()),
            || {
                capture_writers::aggregate_output_processing_failures(vec![format!(
                    "system audio writer failed: {}",
                    capture_writers::no_audio_samples_error("system audio").message
                )])
            },
            |path| removed_paths.push(path.to_string()),
            |error| logged_errors.push(error.message.clone()),
        );

        assert!(result.is_ok());
        assert!(removed_paths.is_empty());
        assert_eq!(logged_errors.len(), 1);
        assert!(logged_errors[0].contains("system audio writer failed:"));
        assert!(logged_errors[0].contains("No system audio audio samples were received"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn zero_sample_system_audio_finalize_removes_output_artifact() {
        let temp_path = std::env::temp_dir().join(format!(
            "zero-sample-system-audio-{}.m4a",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        std::fs::write(&temp_path, b"placeholder")
            .expect("placeholder audio artifact should exist");
        let temp_path = temp_path.to_string_lossy().to_string();

        let error = capture_writers::no_audio_samples_error("system audio");
        if capture_writers::is_no_audio_samples_error_message("system audio", &error.message) {
            maybe_remove_system_audio_file(&temp_path);
        }

        assert!(!Path::new(&temp_path).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn zero_sample_system_audio_soft_pause_recovers_and_removes_output_artifact() {
        let temp_path = std::env::temp_dir().join(format!(
            "zero-sample-system-audio-soft-pause-{}.m4a",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ));
        std::fs::write(&temp_path, b"placeholder")
            .expect("placeholder audio artifact should exist");
        let temp_path = temp_path.to_string_lossy().to_string();
        let error = capture_writers::no_audio_samples_error("system audio");

        assert!(recover_zero_sample_system_audio_soft_pause(
            Some(&temp_path),
            &error
        ));
        assert!(!Path::new(&temp_path).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_audio_soft_pause_keeps_unexpected_finalize_errors_fatal() {
        let error = CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "writer exploded".to_string(),
        };

        assert!(!recover_zero_sample_system_audio_soft_pause(
            Some("/tmp/system-audio.m4a"),
            &error
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_context_allows_paused_system_audio_without_output_file() {
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: true,
        };

        let context = stream_output_context_for_segment(
            Path::new("/tmp/paused-system-audio-segment"),
            "/tmp/paused-system-audio-segment/screen.mov",
            None,
            &sources,
            false,
            None,
            None,
            3,
        )
        .expect("paused system audio should not require a new writer path");

        assert_eq!(
            context.screen_video_output_file.as_deref(),
            Some("/tmp/paused-system-audio-segment/screen.mov")
        );
        assert!(context.system_audio_output_file.is_none());
        assert!(context.system_audio_writer.is_none());
        assert_eq!(context.system_audio_inactivity_tail_buffer_seconds, 3);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_audio_writer_active_can_be_disabled_while_stream_source_stays_enabled() {
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: true,
        };

        assert!(system_audio_writer_active_for_options(
            &sources,
            &ScreenCaptureSessionOptions::default()
        ));
        assert!(!system_audio_writer_active_for_options(
            &sources,
            &ScreenCaptureSessionOptions {
                system_audio_writer_active: Some(false),
                ..Default::default()
            }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stream_output_context_requires_output_file_for_active_system_audio_writer() {
        let sources = ScreenCaptureSources {
            screen: true,
            system_audio: true,
        };

        let error = stream_output_context_for_segment(
            Path::new("/tmp/active-system-audio-segment"),
            "/tmp/active-system-audio-segment/screen.mov",
            None,
            &sources,
            true,
            None,
            None,
            0,
        )
        .expect_err("active system audio must still require a writer path");

        assert_eq!(error.code, "invalid_runtime_state");
        assert_eq!(
            error.message,
            "Missing system audio output file while creating segment writer"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_screen_frame_export_returns_ok_when_error_exists() {
        let mut runtime = ScreenFrameExportRuntime {
            artifact_dir: PathBuf::from("/tmp/frames"),
            callback_queue: dispatch::Queue::serial_with_ar_pool(),
            on_frame_exported: std::sync::Arc::new(|_| {}),
            first_error: Arc::new(Mutex::new(Some(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to finalize JPEG screen frame artifact: boom".to_string(),
            }))),
            segment_frame_index: Arc::new(Mutex::new(ScreenSegmentFrameIndexState::default())),
            next_frame_index: 0,
        };

        let result = finalize_screen_frame_export(None, Some(&mut runtime));

        assert!(result.is_ok());
        assert!(take_frame_export_error(&runtime.first_error).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_screen_frame_export_synchronizes_callback_queue_before_returning() {
        let completed = Arc::new(AtomicBool::new(false));
        let mut runtime = ScreenFrameExportRuntime {
            artifact_dir: PathBuf::from("/tmp/frames"),
            callback_queue: dispatch::Queue::serial_with_ar_pool(),
            on_frame_exported: std::sync::Arc::new(|_| {}),
            first_error: Arc::new(Mutex::new(Some(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to finalize JPEG screen frame artifact: boom".to_string(),
            }))),
            segment_frame_index: Arc::new(Mutex::new(ScreenSegmentFrameIndexState::default())),
            next_frame_index: 0,
        };

        let completed_for_queue = completed.clone();
        runtime.callback_queue.async_once(move || {
            completed_for_queue.store(true, Ordering::SeqCst);
        });

        let result = finalize_screen_frame_export(None, Some(&mut runtime));

        assert!(result.is_ok());
        assert!(completed.load(Ordering::SeqCst));
        assert!(take_frame_export_error(&runtime.first_error).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_segment_frame_index_path_uses_video_stem() {
        let path = screen_segment_frame_index_path(Path::new(
            "/tmp/2026/04/12/session-preview-segment-0001.mov",
        ));

        assert_eq!(
            path,
            PathBuf::from("/tmp/2026/04/12/session-preview-segment-0001.frame-index.bin")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finalize_stream_output_context_keeps_true_screen_video_failures_fatal() {
        let mut removed_paths = Vec::new();
        let mut logged_secondary_failure = false;

        let error = finalize_stream_output_context_impl(
            Some("/tmp/invalid-screen.mov"),
            true,
            None,
            |_| Ok(()),
            |_| {
                Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE.to_string(),
                })
            },
            || Ok(()),
            |path| removed_paths.push(path.to_string()),
            |_| logged_secondary_failure = true,
        )
        .expect_err("screen validation failures must remain fatal");

        assert_eq!(error.code, "capture_output_processing_failed");
        assert_eq!(
            error.message,
            FINALIZED_SCREEN_RECORDING_NO_VIDEO_TRACK_ERROR_MESSAGE
        );
        assert_eq!(removed_paths, vec!["/tmp/invalid-screen.mov".to_string()]);
        assert!(!logged_secondary_failure);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_marks_initial_sample_without_delay() {
        assert!(should_mark_screen_activity(0, 10_000));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_debounce_window_stays_below_minimum_idle_timeout() {
        assert!(SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS < 1_000);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_debounces_samples_inside_window() {
        assert!(!should_mark_screen_activity(
            10_000,
            10_000 + SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS - 1,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_marks_samples_once_window_elapses() {
        assert!(should_mark_screen_activity(
            10_000,
            10_000 + SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_content_gate_marks_initial_fingerprint() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(should_mark_screen_activity_for_fingerprint(0, Some(11)));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_content_gate_skips_repeated_fingerprints_after_activity() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(should_mark_screen_activity_for_fingerprint(0, Some(11)));
        assert!(mark_screen_activity(10_000, 20_000));
        LAST_SCREEN_ACTIVITY_FINGERPRINT.store(11, Ordering::Relaxed);

        assert!(!should_mark_screen_activity_for_fingerprint(11, Some(11)));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_content_gate_marks_changed_fingerprints_after_activity() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(should_mark_screen_activity_for_fingerprint(0, Some(11)));
        assert!(mark_screen_activity(10_000, 20_000));
        LAST_SCREEN_ACTIVITY_FINGERPRINT.store(11, Ordering::Relaxed);

        assert!(should_mark_screen_activity_for_fingerprint(11, Some(12)));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn maybe_mark_screen_activity_updates_state_for_changed_fingerprint() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(maybe_mark_screen_activity_for_fingerprint(Some(11)));
        let first_timestamp = LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed);

        assert!(!maybe_mark_screen_activity_for_fingerprint(Some(11)));
        assert_eq!(
            LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed),
            first_timestamp
        );

        std::thread::sleep(Duration::from_millis(
            SCREEN_ACTIVITY_DEBOUNCE_WINDOW_MS + 10,
        ));

        assert!(maybe_mark_screen_activity_for_fingerprint(Some(12)));
        assert!(LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed) > first_timestamp);

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_content_gate_skips_unknown_fingerprints_after_activity() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();
        assert!(mark_screen_activity(10_000, 20_000));
        LAST_SCREEN_ACTIVITY_FINGERPRINT.store(11, Ordering::Relaxed);

        assert!(!should_mark_screen_activity_for_fingerprint(11, None));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_content_gate_skips_unknown_initial_fingerprint() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(!should_mark_screen_activity_for_fingerprint(0, None));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reset_last_screen_activity_clears_monotonic_and_unix_samples() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        assert!(should_mark_screen_activity_for_fingerprint(0, Some(11)));
        LAST_SCREEN_ACTIVITY_FINGERPRINT.store(11, Ordering::Relaxed);
        assert!(mark_screen_activity(10_000, 20_000));

        assert_eq!(last_screen_activity_unix_ms(), Some(20_000));
        assert_eq!(
            LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed),
            10_000
        );
        assert_ne!(LAST_SCREEN_ACTIVITY_FINGERPRINT.load(Ordering::Relaxed), 0);

        reset_last_screen_activity_unix_ms();

        assert_eq!(last_screen_activity_unix_ms(), None);
        assert_eq!(LAST_SCREEN_ACTIVITY_MONOTONIC_MS.load(Ordering::Relaxed), 0);
        assert_eq!(LAST_SCREEN_ACTIVITY_FINGERPRINT.load(Ordering::Relaxed), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reset_last_screen_activity_clears_system_audio_samples() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        store_system_audio_activity(0.6, 10_000, 20_000);

        assert_eq!(last_system_audio_activity_unix_ms(), Some(20_000));
        assert_eq!(system_audio_activity_level(), Some(0.6));
        assert_eq!(system_audio_activity_idle_ms(), Some(0));

        reset_last_screen_activity_unix_ms();

        assert_eq!(last_system_audio_activity_unix_ms(), None);
        assert_eq!(system_audio_activity_level(), None);
        assert_eq!(system_audio_activity_idle_ms(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_audio_activity_window_peak_tracks_max_until_taken() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        store_system_audio_activity(0.02, 10_000, 20_000);
        store_system_audio_activity(0.60, 10_010, 20_010);
        store_system_audio_activity(0.08, 10_020, 20_020);

        assert_eq!(take_system_audio_activity_window_peak_level(), Some(0.60));
        assert_eq!(take_system_audio_activity_window_peak_level(), None);
        assert_eq!(system_audio_activity_level(), Some(0.08));

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_audio_activity_window_peak_peek_preserves_value_until_taken() {
        let _guard = screen_activity_state_test_guard();
        reset_last_screen_activity_unix_ms();

        store_system_audio_activity(0.15, 10_000, 20_000);
        store_system_audio_activity(0.70, 10_010, 20_010);

        assert_eq!(peek_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(peek_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(take_system_audio_activity_window_peak_level(), Some(0.70));
        assert_eq!(peek_system_audio_activity_window_peak_level(), None);

        reset_last_screen_activity_unix_ms();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mix_screen_activity_pixel_probe_bytes_skips_invalid_bounds() {
        let bytes = [1_u8, 2, 3];
        let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;

        assert!(!mix_screen_activity_pixel_probe_bytes(
            &mut hash,
            bytes.as_ptr(),
            4,
            4,
            2,
            bytes.len(),
        ));
        assert_eq!(hash, SCREEN_ACTIVITY_FINGERPRINT_SEED);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mix_screen_activity_pixel_probe_bytes_samples_valid_bounds() {
        let bytes = [7_u8; 32];
        let mut hash = SCREEN_ACTIVITY_FINGERPRINT_SEED;

        assert!(mix_screen_activity_pixel_probe_bytes(
            &mut hash,
            bytes.as_ptr(),
            8,
            8,
            4,
            bytes.len(),
        ));
        assert_ne!(hash, SCREEN_ACTIVITY_FINGERPRINT_SEED);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_planar_screen_activity_equivalence_uses_pixel_width_not_row_byte_width() {
        let pixel_width = 32;
        let height = 32;
        let bytes_per_row = pixel_width * 4;
        let bytes = vec![128_u8; bytes_per_row * height];

        let live_equivalence = non_planar_screen_activity_captured_frame_equivalence(
            &bytes,
            bytes_per_row,
            pixel_width,
            height,
            cidre::cv::PixelFormat::_32_BGRA,
        );

        assert!(
            matches!(
                live_equivalence,
                Some(CapturedFrameEquivalenceOutcome::Ready(_))
            ),
            "live non-planar screen samples should derive equivalence from pixel dimensions"
        );

        let byte_width_bug_shape = non_planar_screen_activity_captured_frame_equivalence(
            &bytes,
            bytes_per_row,
            bytes_per_row,
            height,
            cidre::cv::PixelFormat::_32_BGRA,
        );

        assert!(
            byte_width_bug_shape.is_none(),
            "row byte width must not be accepted where pixel width is required"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_planar_screen_activity_equivalence_accepts_bgra_bytes_without_fallback_shape() {
        let pixel_width = 8;
        let height = 8;
        let bytes_per_row = pixel_width * 4;
        let bytes = vec![96_u8; bytes_per_row * height];

        let equivalence = non_planar_screen_activity_captured_frame_equivalence(
            &bytes,
            bytes_per_row,
            pixel_width,
            height,
            cidre::cv::PixelFormat::_32_BGRA,
        );

        assert!(matches!(
            equivalence,
            Some(CapturedFrameEquivalenceOutcome::Ready(_))
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_activity_bitmap_fingerprint_detects_localized_center_change() {
        let width = 64;
        let height = 64;
        let bytes_per_row = width * 4;
        let baseline = vec![128_u8; bytes_per_row * height];
        let mut changed = baseline.clone();

        for changed_row in 32..40 {
            for changed_col in 32..40 {
                let changed_offset = changed_row * bytes_per_row + changed_col * 4;
                changed[changed_offset..changed_offset + 4].copy_from_slice(&[32, 32, 32, 255]);
            }
        }

        let baseline_fingerprint =
            screen_activity_bitmap_fingerprint(&baseline, bytes_per_row, width, height);
        let changed_fingerprint =
            screen_activity_bitmap_fingerprint(&changed, bytes_per_row, width, height);

        assert_ne!(baseline_fingerprint, changed_fingerprint);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_frame_content_bitmap_fingerprint_ignores_cursor_sized_change() {
        let width = 64;
        let height = 64;
        let bytes_per_row = width * 4;
        let baseline = vec![128_u8; bytes_per_row * height];
        let mut cursor_changed = baseline.clone();

        for row in 35..37 {
            for col in 35..37 {
                let offset = row * bytes_per_row + col * 4;
                cursor_changed[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }

        let baseline_fingerprint =
            screen_frame_content_bitmap_fingerprint(&baseline, bytes_per_row, width, height);
        let cursor_changed_fingerprint =
            screen_frame_content_bitmap_fingerprint(&cursor_changed, bytes_per_row, width, height);

        assert_eq!(baseline_fingerprint, cursor_changed_fingerprint);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_frame_content_bitmap_fingerprint_detects_larger_localized_change() {
        let width = 64;
        let height = 64;
        let bytes_per_row = width * 4;
        let baseline = vec![128_u8; bytes_per_row * height];
        let mut changed = baseline.clone();

        for row in 32..40 {
            for col in 32..40 {
                let offset = row * bytes_per_row + col * 4;
                changed[offset..offset + 4].copy_from_slice(&[32, 32, 32, 255]);
            }
        }

        let baseline_fingerprint =
            screen_frame_content_bitmap_fingerprint(&baseline, bytes_per_row, width, height);
        let changed_fingerprint =
            screen_frame_content_bitmap_fingerprint(&changed, bytes_per_row, width, height);

        assert_ne!(baseline_fingerprint, changed_fingerprint);
    }
}

#[cfg(target_os = "macos")]
fn load_asset_tracks_with_timeout(
    asset: &cidre::av::UrlAsset,
    media_type: &cidre::av::MediaType,
    timeout_code: &str,
    timeout_message: &str,
) -> Result<cidre::arc::R<cidre::ns::Array<cidre::av::asset::Track>>, CaptureErrorResponse> {
    let (tx, rx) = mpsc::channel::<
        Result<cidre::arc::R<cidre::ns::Array<cidre::av::asset::Track>>, CaptureErrorResponse>,
    >();

    asset.load_tracks_with_media_type_block(media_type, move |tracks, error| {
        let result = if let Some(tracks) = tracks {
            Ok(tracks.retained())
        } else if let Some(error) = error {
            Err(error_with_ns_error(
                "capture_output_processing_failed",
                "Failed to query recording tracks",
                error,
            ))
        } else {
            Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to query recording tracks".to_string(),
            })
        };

        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(_) => Err(CaptureErrorResponse {
            code: timeout_code.to_string(),
            message: timeout_message.to_string(),
        }),
    }
}

#[cfg(target_os = "macos")]
pub fn strip_audio_from_recording_file(recording_file: &str) -> Result<(), CaptureErrorResponse> {
    use cidre::{av, ns};

    let input_path = std::path::Path::new(recording_file);
    let temp_path = input_path.with_extension("video-only.mov");
    let _ = std::fs::remove_file(&temp_path);

    let input_url = ns::Url::with_fs_path_str(recording_file, false);
    let temp_url = ns::Url::with_fs_path_str(temp_path.to_string_lossy().as_ref(), false);

    let result = {
        let _autorelease_pool = cidre::objc::autorelease_pool::AutoreleasePoolPage::push();
        let asset =
            av::UrlAsset::with_url(&input_url, None).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to open recording for video-only conversion".to_string(),
            })?;

        let video_tracks = load_asset_tracks_with_timeout(
            asset.as_ref(),
            av::MediaType::video(),
            "capture_output_processing_failed",
            "Timed out while loading recording video track",
        )?;
        let video_track = video_tracks.first().ok_or_else(|| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Recording has no video track to preserve".to_string(),
        })?;

        let mut reader =
            av::AssetReader::with_asset(asset.as_ref()).map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to create asset reader for video-only conversion".to_string(),
            })?;

        let mut reader_output =
            av::AssetReaderTrackOutput::with_track(video_track, None).map_err(|_| {
                CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Failed to create video track reader output".to_string(),
                }
            })?;
        reader_output.set_always_copies_sample_data(false);

        let reader_output_ref: &av::AssetReaderOutput =
            unsafe { &*(&*reader_output as *const _ as *const av::AssetReaderOutput) };
        if !reader.can_add_output(reader_output_ref) {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to add video track reader output".to_string(),
            });
        }
        reader
            .add_output(reader_output_ref)
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to attach video track reader output".to_string(),
            })?;

        let mut writer = av::AssetWriter::with_url_and_file_type(&temp_url, av::FileType::qt())
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to create video-only asset writer".to_string(),
            })?;
        let mut writer_input =
            av::AssetWriterInput::with_media_type_and_output_settings(av::MediaType::video(), None)
                .map_err(|_| CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Failed to create video-only writer input".to_string(),
                })?;
        writer_input.set_expects_media_data_in_real_time(false);

        if !writer.can_add_input(&writer_input) {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to add video-only writer input".to_string(),
            });
        }
        writer
            .add_input(&writer_input)
            .map_err(|_| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to attach video-only writer input".to_string(),
            })?;

        let started_reading = reader.start_reading().map_err(|_| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: "Failed to start reading recording for video-only conversion".to_string(),
        })?;
        if !started_reading {
            if let Some(error) = reader.error() {
                return Err(error_with_ns_error(
                    "capture_output_processing_failed",
                    "Failed to start reading recording for video-only conversion",
                    error.as_ref(),
                ));
            }

            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to start reading recording for video-only conversion".to_string(),
            });
        }

        if !writer.start_writing() {
            return Err(capture_writers::writer_error_response(
                &writer,
                "capture_output_processing_failed",
                "Failed to start writing video-only recording",
            ));
        }
        writer.start_session_at_src_time(cidre::cm::Time::zero());

        loop {
            let sample_buf = reader_output
                .next_sample_buf()
                .map_err(|_| CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Failed to read video sample during video-only conversion".to_string(),
                })?;

            let Some(sample_buf) = sample_buf else {
                break;
            };

            while !writer_input.is_ready_for_more_media_data() {
                std::thread::sleep(Duration::from_millis(1));
            }

            let appended = writer_input
                .append_sample_buf(sample_buf.as_ref())
                .map_err(|_| CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Failed to append video sample during video-only conversion"
                        .to_string(),
                })?;

            if !appended {
                return Err(capture_writers::writer_error_response(
                    &writer,
                    "capture_output_processing_failed",
                    "Failed to append video sample during video-only conversion",
                ));
            }
        }

        writer_input.mark_as_finished();
        writer.finish_writing();

        let wait_deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match writer.status() {
                cidre::av::asset::WriterStatus::Completed => break,
                cidre::av::asset::WriterStatus::Failed => {
                    return Err(capture_writers::writer_error_response(
                        &writer,
                        "capture_output_processing_failed",
                        "Failed to finalize video-only recording",
                    ));
                }
                status if std::time::Instant::now() >= wait_deadline => {
                    return Err(CaptureErrorResponse {
                        code: "capture_output_processing_failed".to_string(),
                        message: format!(
                            "Timed out while finalizing video-only recording (status: {:?})",
                            status
                        ),
                    });
                }
                _ => std::thread::sleep(Duration::from_millis(10)),
            }
        }

        match reader.status() {
            cidre::av::asset::ReaderStatus::Completed => {}
            cidre::av::asset::ReaderStatus::Failed => {
                if let Some(error) = reader.error() {
                    return Err(error_with_ns_error(
                        "capture_output_processing_failed",
                        "Video-only conversion reader failed",
                        error.as_ref(),
                    ));
                }

                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: "Video-only conversion reader failed".to_string(),
                });
            }
            status => {
                return Err(CaptureErrorResponse {
                    code: "capture_output_processing_failed".to_string(),
                    message: format!(
                        "Video-only conversion reader ended unexpectedly (status: {:?})",
                        status
                    ),
                });
            }
        }

        reader.cancel_reading();

        let video_only_asset =
            av::UrlAsset::with_url(&temp_url, None).ok_or_else(|| CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Failed to verify video-only recording".to_string(),
            })?;
        let audio_tracks = load_asset_tracks_with_timeout(
            video_only_asset.as_ref(),
            av::MediaType::audio(),
            "capture_output_processing_failed",
            "Timed out while validating video-only recording audio tracks",
        )?;
        if !audio_tracks.is_empty() {
            return Err(CaptureErrorResponse {
                code: "capture_output_processing_failed".to_string(),
                message: "Video-only conversion produced an unexpected audio track".to_string(),
            });
        }

        std::fs::rename(&temp_path, input_path).map_err(|error| CaptureErrorResponse {
            code: "capture_output_processing_failed".to_string(),
            message: format!("Failed to replace recording with video-only mov: {error}"),
        })
    };

    result
}

#[cfg(not(target_os = "macos"))]
pub fn strip_audio_from_recording_file(_recording_file: &str) -> Result<(), CaptureErrorResponse> {
    Ok(())
}
