//! Frame and scrub previews for the timeline: the original frame file, a
//! persisted segment frame, or a still pulled from the segment video, plus the
//! low-res scrub-preview cache and its background generation. Every file a
//! preview hands out is granted to the host's asset scope through
//! `WindowSignal::AllowAssetPath`.

use std::{
    collections::{HashMap, HashSet},
    fs,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use std::sync::Condvar;

#[cfg(test)]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::oneshot;

use crate::{AppCore, AppInfraState, CoreEvent, WindowSignal};

mod cache_files;
mod commands;
mod dto;
mod extract;
mod preview;
mod scrub_cache;
mod scrub_jobs;
mod segment;
mod state;
#[cfg(test)]
mod tests;

use cache_files::*;
pub use commands::*;
pub use dto::*;
use extract::*;
use preview::*;
pub use scrub_cache::*;
pub use scrub_jobs::*;
use segment::*;
pub use state::*;

pub type FramePreviewCacheState = Mutex<FramePreviewState>;

const FRAME_PREVIEW_CACHE_MAX_ENTRIES: usize = 256;
const FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL: Duration = Duration::from_secs(15);
const FRAME_PREVIEW_EXACT_MISS_LOG_THRESHOLD_MS: f64 = 5.0;
const FRAME_PREVIEW_EXACT_VIDEO_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_GENERATION_CANCELLED: &str = "preview generation cancelled";
const PREVIEW_GENERATION_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const SCRUB_PREVIEW_DEFAULT_MAX_PIXEL_SIZE: u32 = 200;
const SCRUB_PREVIEW_MIN_MAX_PIXEL_SIZE: u32 = 200;
const SCRUB_PREVIEW_MAX_MAX_PIXEL_SIZE: u32 = 1280;
const SCRUB_PREVIEW_PERF_LOG_THRESHOLD_MS: u128 = 25;
const SCRUB_PREVIEW_VIDEO_BATCH_TIMEOUT: Duration = Duration::from_secs(2);
const SCRUB_PREVIEW_JPEG_QUALITY: u8 = 72;
const SCRUB_PREVIEW_RENDITION: &str = "v1-jpeg-q72-max360-1fps";
const SCRUB_PREVIEW_MAX_PIXEL_SIZE: u32 = 360;
const SCRUB_PREVIEW_INTERVAL_MS: u64 = 1000;
const SCRUB_PREVIEW_CHUNK_SIZE: usize = 30;
pub const SCRUB_PREVIEW_CACHE_CHANGED_EVENT: &str = "scrub_preview_cache_changed";
const GENERATED_FRAME_PREVIEW_CACHE_DIR: &str = "frame-previews";
const GENERATED_SCRUB_PREVIEW_CACHE_DIR: &str = "scrub-previews";
const GENERATED_FRAME_SCRUB_PREVIEW_CACHE_DIR: &str = "frame-v1-jpeg-q72";
const GENERATED_FRAME_PREVIEW_CACHE_MAX_FILES: usize = 512;
const GENERATED_FRAME_PREVIEW_CACHE_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);
const GENERATED_SCRUB_PREVIEW_CACHE_MAX_FILES: usize = 4096;
const GENERATED_SCRUB_PREVIEW_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const GENERATED_SCRUB_PREVIEW_CACHE_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24);
