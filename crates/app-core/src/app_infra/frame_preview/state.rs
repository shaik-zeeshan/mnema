//! The in-memory preview state: the TTL'd preview cache, in-flight request
//! collapsing per frame and per video, and the scrub-interval work set.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CachedFramePreview {
    preview: FramePreviewDto,
    cached_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CachedVideoPreviewFailure {
    message: String,
    cached_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ScrubPreviewCacheKey {
    frame_id: i64,
    max_pixel_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ScrubIntervalWorkKey {
    pub(super) segment_cache_key: String,
    pub(super) interval_start_video_offset_ms: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FramePreviewVideoScopeDto {
    ActiveFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VideoPreviewRequestScope {
    Shared,
    ActiveFrame,
}

impl From<Option<FramePreviewVideoScopeDto>> for VideoPreviewRequestScope {
    fn from(scope: Option<FramePreviewVideoScopeDto>) -> Self {
        match scope {
            Some(FramePreviewVideoScopeDto::ActiveFrame) => Self::ActiveFrame,
            None => Self::Shared,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct VideoPreviewWorkKey {
    scope: VideoPreviewRequestScope,
    video_path: PathBuf,
}

impl VideoPreviewWorkKey {
    fn new(video_path: &Path, scope: VideoPreviewRequestScope) -> Self {
        Self {
            scope,
            video_path: video_path.to_path_buf(),
        }
    }
}

#[derive(Debug)]
pub(super) struct ActiveVideoPreviewWork {
    work_id: u64,
    cancel_requested: Arc<AtomicBool>,
    waiters: Vec<oneshot::Sender<Result<(), String>>>,
}

#[derive(Debug)]
pub(super) struct VideoPreviewRequestToken {
    pub(super) key: VideoPreviewWorkKey,
    pub(super) work_id: u64,
    pub(super) cancel_requested: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndexedFramePreviewOffset {
    pub(super) video_offset_ms: u64,
    pub(super) exact_match: bool,
}

#[derive(Debug, Default)]
pub struct FramePreviewState {
    cache: FramePreviewCache,
    scrub_cache: FrameScrubPreviewCache,
    in_flight: HashMap<i64, Vec<oneshot::Sender<Result<Option<FramePreviewDto>, String>>>>,
    video_in_flight: HashMap<VideoPreviewWorkKey, ActiveVideoPreviewWork>,
    next_video_work_id: u64,
    scrub_interval_in_flight: HashSet<ScrubIntervalWorkKey>,
}

#[derive(Debug, Default)]
pub(super) struct FramePreviewCache {
    entries: HashMap<i64, CachedFramePreview>,
    video_failures: HashMap<PathBuf, CachedVideoPreviewFailure>,
}

#[derive(Debug, Default)]
pub struct FrameScrubPreviewCache {
    entries: HashMap<ScrubPreviewCacheKey, CachedFramePreview>,
}

impl FramePreviewState {
    pub(super) fn get(
        &mut self,
        frame_id: i64,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.cache.get(frame_id, ttl, now)
    }

    pub(super) fn insert(
        &mut self,
        frame_id: i64,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.cache.insert(frame_id, preview, ttl, now);
    }

    pub(super) fn clear(&mut self) {
        self.cancel_all_video_requests();
        self.cache.clear();
        self.scrub_cache.clear();
        self.in_flight.clear();
    }

    pub(super) fn insert_scrub(
        &mut self,
        frame_id: i64,
        max_pixel_size: u32,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.scrub_cache.insert(
            ScrubPreviewCacheKey {
                frame_id,
                max_pixel_size,
            },
            preview,
            ttl,
            now,
        );
    }

    pub(super) fn get_scrub(
        &mut self,
        frame_id: i64,
        max_pixel_size: u32,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.scrub_cache.get(
            ScrubPreviewCacheKey {
                frame_id,
                max_pixel_size,
            },
            ttl,
            now,
        )
    }

    pub(super) fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.cache.get_video_failure(video_path, now)
    }

    pub(super) fn insert_video_failure(
        &mut self,
        video_path: &Path,
        message: String,
        now: Instant,
    ) {
        self.cache.insert_video_failure(video_path, message, now);
    }

    pub(super) fn begin_request(
        &mut self,
        frame_id: i64,
    ) -> Result<(), oneshot::Receiver<Result<Option<FramePreviewDto>, String>>> {
        if let Some(waiters) = self.in_flight.get_mut(&frame_id) {
            let (tx, rx) = oneshot::channel();
            waiters.push(tx);
            return Err(rx);
        }

        self.in_flight.insert(frame_id, Vec::new());
        Ok(())
    }

    pub(super) fn finish_request(
        &mut self,
        frame_id: i64,
        result: Result<Option<FramePreviewDto>, String>,
    ) {
        let waiters = self.in_flight.remove(&frame_id).unwrap_or_default();
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
    }

    pub(super) fn begin_video_request(
        &mut self,
        video_path: &Path,
        scope: VideoPreviewRequestScope,
    ) -> Result<VideoPreviewRequestToken, oneshot::Receiver<Result<(), String>>> {
        let key = VideoPreviewWorkKey::new(video_path, scope);
        if let Some(work) = self.video_in_flight.get_mut(&key) {
            let (tx, rx) = oneshot::channel();
            work.waiters.push(tx);
            return Err(rx);
        }

        let work_id = self.next_video_work_id;
        self.next_video_work_id = self.next_video_work_id.wrapping_add(1);
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let token = VideoPreviewRequestToken {
            key,
            work_id,
            cancel_requested: Arc::clone(&cancel_requested),
        };
        self.video_in_flight.insert(
            token.key.clone(),
            ActiveVideoPreviewWork {
                work_id,
                cancel_requested,
                waiters: Vec::new(),
            },
        );
        Ok(token)
    }

    pub(super) fn finish_video_request(
        &mut self,
        token: &VideoPreviewRequestToken,
        result: Result<(), String>,
    ) {
        let should_finish = self
            .video_in_flight
            .get(&token.key)
            .is_some_and(|work| work.work_id == token.work_id);
        if !should_finish {
            return;
        }

        let work = self
            .video_in_flight
            .remove(&token.key)
            .expect("matching video preview work should exist");
        for waiter in work.waiters {
            let _ = waiter.send(result.clone());
        }
    }

    pub(super) fn cancel_active_video_requests(&mut self) -> usize {
        self.cancel_video_requests(|key| key.scope == VideoPreviewRequestScope::ActiveFrame)
    }

    fn cancel_all_video_requests(&mut self) -> usize {
        self.cancel_video_requests(|_| true)
    }

    fn cancel_video_requests(
        &mut self,
        should_cancel: impl Fn(&VideoPreviewWorkKey) -> bool,
    ) -> usize {
        let active = self
            .video_in_flight
            .keys()
            .filter(|key| should_cancel(key))
            .cloned()
            .collect::<Vec<_>>();
        let cancelled = active.len();
        for key in active {
            let Some(work) = self.video_in_flight.remove(&key) else {
                continue;
            };
            work.cancel_requested.store(true, Ordering::SeqCst);
            for waiter in work.waiters {
                let _ = waiter.send(Err(PREVIEW_GENERATION_CANCELLED.to_string()));
            }
        }
        cancelled
    }

    pub(super) fn begin_scrub_interval_work(
        &mut self,
        keys: &[ScrubIntervalWorkKey],
    ) -> Vec<ScrubIntervalWorkKey> {
        let mut accepted = Vec::new();
        for key in keys {
            if self.scrub_interval_in_flight.insert(key.clone()) {
                accepted.push(key.clone());
            }
        }
        accepted
    }

    pub(super) fn finish_scrub_interval_work(&mut self, keys: &[ScrubIntervalWorkKey]) {
        for key in keys {
            self.scrub_interval_in_flight.remove(key);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub(super) fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    #[cfg(test)]
    pub(super) fn video_in_flight_len(&self) -> usize {
        self.video_in_flight.len()
    }
}

impl FramePreviewCache {
    pub(super) fn get(
        &mut self,
        frame_id: i64,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        let preview = self
            .entries
            .get(&frame_id)
            .map(|entry| entry.preview.clone())?;
        if !Path::new(&preview.file_path).is_file() {
            self.entries.remove(&frame_id);
            return None;
        }
        Some(preview)
    }

    pub(super) fn insert(
        &mut self,
        frame_id: i64,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.evict_expired(ttl, now);
        self.entries.insert(
            frame_id,
            CachedFramePreview {
                preview,
                cached_at: now,
            },
        );
        self.evict_oldest_excess_entries();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.video_failures.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    fn evict_expired(&mut self, ttl: Duration, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.cached_at) < ttl);
    }

    fn evict_oldest_excess_entries(&mut self) {
        while self.entries.len() > FRAME_PREVIEW_CACHE_MAX_ENTRIES {
            let Some(oldest_frame_id) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(frame_id, _)| *frame_id)
            else {
                break;
            };

            self.entries.remove(&oldest_frame_id);
        }
    }

    pub(super) fn get_video_failure(&mut self, video_path: &Path, now: Instant) -> Option<String> {
        self.evict_expired_video_failures(now);
        self.video_failures
            .get(video_path)
            .map(|entry| entry.message.clone())
    }

    pub(super) fn insert_video_failure(
        &mut self,
        video_path: &Path,
        message: String,
        now: Instant,
    ) {
        self.evict_expired_video_failures(now);
        self.video_failures.insert(
            video_path.to_path_buf(),
            CachedVideoPreviewFailure {
                message,
                cached_at: now,
            },
        );
    }

    fn evict_expired_video_failures(&mut self, now: Instant) {
        self.video_failures.retain(|_, entry| {
            now.duration_since(entry.cached_at) < FRAME_PREVIEW_VIDEO_FAILURE_CACHE_TTL
        });
    }
}

impl FrameScrubPreviewCache {
    pub(super) fn get(
        &mut self,
        key: ScrubPreviewCacheKey,
        ttl: Duration,
        now: Instant,
    ) -> Option<FramePreviewDto> {
        self.evict_expired(ttl, now);
        let preview = self.entries.get(&key).map(|entry| entry.preview.clone())?;
        if !Path::new(&preview.file_path).is_file() {
            self.entries.remove(&key);
            return None;
        }
        Some(preview)
    }

    pub(super) fn insert(
        &mut self,
        key: ScrubPreviewCacheKey,
        preview: FramePreviewDto,
        ttl: Duration,
        now: Instant,
    ) {
        self.evict_expired(ttl, now);
        self.entries.insert(
            key,
            CachedFramePreview {
                preview,
                cached_at: now,
            },
        );
        self.evict_oldest_excess_entries();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    fn evict_expired(&mut self, ttl: Duration, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.cached_at) < ttl);
    }

    fn evict_oldest_excess_entries(&mut self) {
        while self.entries.len() > FRAME_PREVIEW_CACHE_MAX_ENTRIES {
            let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.entries.remove(&oldest_key);
        }
    }
}
