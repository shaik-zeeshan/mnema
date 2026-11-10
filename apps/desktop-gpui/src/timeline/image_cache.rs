//! The stage's [`ImageCache`], shaped like the Svelte stage's (192 compressed
//! blob URLs, decoded only when shown):
//!
//! - an LRU of the **encoded** file bytes at 192 entries (`PREVIEW_CACHE_MAX_ENTRIES`),
//!   failures remembered 5 s (`PREVIEW_FAILURE_CACHE_TTL_MS`) before a retry;
//! - a small LRU of **decoded** pixels, [`DECODED_MAX`] = the displayed frame plus
//!   the scrub's widest prefetch window, each decoded no larger than the stage's
//!   pixel size. An evicted image goes back to the GPU atlas with `drop_image` and
//!   its pixels are freed off the main thread.
//!
//! Reads and decodes run on the background executor (RGBA swapped to BGRA there),
//! so the main thread never decodes. 192 decoded Retina frames were ~3 GB; 192
//! JPEGs are a few tens of MB.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    hash, App, AppContext as _, Entity, EntityId, ImageCache, ImageCacheError, Pixels, RenderImage,
    Resource, Size, Task, WeakEntity, Window,
};

pub const MAX_ENTRIES: usize = super::scrub_engine::CACHE_MAX;
pub const FAILURE_TTL: Duration = super::scrub_engine::FAILURE_TTL;
/// Decoded frames kept: the scrub's largest "max uncached" window (9).
pub const DECODED_MAX: usize = 9;

/// A small LRU (most recent last) the cache keeps its items in; pure, so the
/// eviction order is testable without a window.
pub struct Lru<V> {
    cap: usize,
    entries: Vec<(u64, V)>,
}

impl<V> Lru<V> {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Move `key` to the most-recent end and return it.
    pub fn touch(&mut self, key: u64) -> Option<&mut V> {
        let pos = self.entries.iter().position(|(k, _)| *k == key)?;
        let entry = self.entries.remove(pos);
        self.entries.push(entry);
        self.entries.last_mut().map(|(_, v)| v)
    }

    pub fn peek(&self, key: u64) -> Option<&V> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn peek_mut(&mut self, key: u64) -> Option<&mut V> {
        self.entries
            .iter_mut()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    pub fn remove(&mut self, key: u64) -> Option<V> {
        let pos = self.entries.iter().position(|(k, _)| *k == key)?;
        Some(self.entries.remove(pos).1)
    }

    /// Insert as most recent; returns what fell off the old end.
    pub fn insert(&mut self, key: u64, value: V) -> Vec<V> {
        self.remove(key);
        self.entries.push((key, value));
        let over = self.entries.len().saturating_sub(self.cap);
        self.entries.drain(..over).map(|(_, v)| v).collect()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = V> + '_ {
        self.entries.drain(..).map(|(_, v)| v)
    }
}

enum Bytes {
    Loading,
    Ready(Arc<[u8]>),
    /// Unreadable or undecodable; retried `FAILURE_TTL` after `at`.
    Failed(ImageCacheError, Instant),
}

enum Pix {
    Loading,
    Ready {
        image: Arc<RenderImage>,
        /// The pixel box it was decoded for, and whether that shrank it.
        limit: (u32, u32),
        shrunk: bool,
    },
}

pub struct FrameImages {
    this: WeakEntity<Self>,
    encoded: Lru<Bytes>,
    decoded: Lru<Pix>,
    /// The stage's laid-out size (logical px); 0 until the first layout.
    stage: Rc<Cell<Size<Pixels>>>,
    /// Views to repaint when a load lands.
    waiting: HashSet<EntityId>,
    tasks: HashMap<u64, Task<()>>,
    /// Loads started (first sights, retries and re-decodes); tests read it.
    pub loads: u32,
}

type Loaded = Option<Result<Arc<RenderImage>, ImageCacheError>>;

/// Decode `bytes` (BGRA, as GPUI wants) to fit inside `limit` px, never upscaling.
/// `(0, 0)` = no limit (the stage hasn't laid out yet).
pub fn decode_fit(bytes: &[u8], limit: (u32, u32)) -> Result<(RenderImage, bool), ImageCacheError> {
    let err = |e: image::ImageError| ImageCacheError::Other(Arc::new(e.into()));
    let mut img = image::load_from_memory(bytes).map_err(err)?;
    let (w, h) = (img.width(), img.height());
    let shrunk = limit.0 > 0 && limit.1 > 0 && (w > limit.0 || h > limit.1);
    if shrunk {
        // Area-averaging downscale: sharp for screenshots, and fast.
        img = img.thumbnail(limit.0, limit.1);
    }
    let mut rgba = img.into_rgba8();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Ok((RenderImage::new([image::Frame::new(rgba)]), shrunk))
}

impl FrameImages {
    pub fn new(stage: Rc<Cell<Size<Pixels>>>, cx: &mut App) -> Entity<Self> {
        Self::with_decoded(stage, DECODED_MAX, cx)
    }

    /// Keep `decoded_max` decoded images: a strip of small thumbnails all on
    /// screen at once needs more than the stage's 9, or it re-decodes forever.
    pub fn with_decoded(
        stage: Rc<Cell<Size<Pixels>>>,
        decoded_max: usize,
        cx: &mut App,
    ) -> Entity<Self> {
        let e = cx.new(|cx| Self {
            this: cx.entity().downgrade(),
            encoded: Lru::new(MAX_ENTRIES),
            decoded: Lru::new(decoded_max),
            stage,
            waiting: HashSet::new(),
            tasks: HashMap::new(),
            loads: 0,
        });
        cx.observe_release(&e, |cache, cx| {
            for pix in cache.decoded.drain() {
                if let Pix::Ready { image, .. } = pix {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        e
    }

    pub fn len(&self) -> usize {
        self.encoded.len()
    }

    pub fn is_empty(&self) -> bool {
        self.encoded.is_empty()
    }

    /// Decoded frames held right now (≤ [`DECODED_MAX`]).
    pub fn decoded_len(&self) -> usize {
        self.decoded.len()
    }

    /// Already decoded (the stage swaps to it without a blank frame).
    pub fn ready(&self, resource: &Resource) -> bool {
        matches!(self.decoded.peek(hash(resource)), Some(Pix::Ready { .. }))
    }

    /// Reading or decoding failed (and the failure is still cached).
    pub fn failed(&self, resource: &Resource) -> bool {
        matches!(self.encoded.peek(hash(resource)), Some(Bytes::Failed(..)))
    }

    fn limit(&self, window: &Window) -> (u32, u32) {
        let s = self.stage.get();
        let scale = window.scale_factor();
        let px = |v: Pixels| (f32::from(v) * scale).ceil().max(0.) as u32;
        (px(s.width), px(s.height))
    }

    fn evict(evicted: Vec<Pix>, window: Option<&mut Window>, cx: &mut App) {
        let mut window = window;
        for pix in evicted {
            if let Pix::Ready { image, .. } = pix {
                // The last reference frees ~10 MB of pixels: do that off the main thread.
                let last = image.clone();
                cx.drop_image(image, window.as_deref_mut());
                cx.background_spawn(async move { drop(last) }).detach();
            }
        }
    }

    /// Read (unless `bytes` are cached) and decode on the background executor.
    fn start(
        &mut self,
        key: u64,
        path: Option<Arc<Path>>,
        bytes: Option<Arc<[u8]>>,
        limit: (u32, u32),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.loads += 1;
        if bytes.is_none() {
            self.encoded.insert(key, Bytes::Loading);
        }
        if !matches!(self.decoded.peek(key), Some(Pix::Ready { .. })) {
            let evicted = self.decoded.insert(key, Pix::Loading);
            Self::evict(evicted, Some(window), cx);
        }
        let work = cx.background_spawn(async move {
            let bytes: Arc<[u8]> = match (bytes, path) {
                (Some(b), _) => b,
                (None, Some(p)) => std::fs::read(&*p)
                    .map_err(|e| ImageCacheError::Io(Arc::new(e)))?
                    .into(),
                (None, None) => unreachable!("a load has a path or bytes"),
            };
            let (image, shrunk) = decode_fit(&bytes, limit)?;
            Ok::<_, ImageCacheError>((bytes, Arc::new(image), shrunk))
        });
        let this = self.this.clone();
        let task = cx.spawn(async move |cx| {
            let res = work.await;
            let now = cx.background_executor().now();
            this.update(cx, |c, cx| {
                c.tasks.remove(&key);
                match res {
                    Ok((bytes, image, shrunk)) => {
                        c.encoded.insert(key, Bytes::Ready(bytes));
                        match c.decoded.peek_mut(key) {
                            Some(slot) => {
                                let old = std::mem::replace(
                                    slot,
                                    Pix::Ready {
                                        image,
                                        limit,
                                        shrunk,
                                    },
                                );
                                Self::evict(vec![old], None, cx);
                            }
                            // Evicted while decoding: keep only the bytes.
                            None => Self::evict(
                                vec![Pix::Ready {
                                    image,
                                    limit,
                                    shrunk,
                                }],
                                None,
                                cx,
                            ),
                        }
                    }
                    Err(e) => {
                        c.encoded.insert(key, Bytes::Failed(e, now));
                        if matches!(c.decoded.peek(key), Some(Pix::Loading)) {
                            c.decoded.remove(key);
                        }
                    }
                }
                for view in c.waiting.drain() {
                    App::notify(cx, view);
                }
            })
            .ok();
        });
        self.tasks.insert(key, task);
    }
}

impl ImageCache for FrameImages {
    fn load(&mut self, resource: &Resource, window: &mut Window, cx: &mut App) -> Loaded {
        let Resource::Path(path) = resource else {
            return Some(Err(ImageCacheError::Asset(
                "the stage loads files only".into(),
            )));
        };
        let now = cx.background_executor().now();
        let key = hash(resource);
        let limit = self.limit(window);
        let busy = self.tasks.contains_key(&key);
        if let Some(Pix::Ready {
            image,
            limit: had,
            shrunk,
        }) = self.decoded.touch(key)
        {
            let image = image.clone();
            // The stage grew past a shrunk decode: redo it, showing this one meanwhile.
            let regrow = *shrunk && (limit.0 > had.0 || limit.1 > had.1);
            if let (true, false, Some(Bytes::Ready(b))) = (regrow, busy, self.encoded.touch(key)) {
                let b = b.clone();
                self.start(key, None, Some(b), limit, window, cx);
            } else {
                self.encoded.touch(key);
            }
            return Some(Ok(image));
        }
        self.waiting.insert(window.current_view());
        if busy {
            return None;
        }
        match self.encoded.touch(key) {
            Some(Bytes::Failed(e, at)) if now.duration_since(*at) < FAILURE_TTL => {
                return Some(Err(e.clone()));
            }
            Some(Bytes::Ready(b)) => {
                let b = b.clone();
                self.start(key, None, Some(b), limit, window, cx);
                return None;
            }
            _ => {}
        }
        self.start(key, Some(path.clone()), None, limit, window, cx);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lru_evicts_the_least_recently_used_past_192() {
        let mut lru = Lru::new(MAX_ENTRIES);
        for k in 0..MAX_ENTRIES as u64 {
            assert!(lru.insert(k, k).is_empty());
        }
        assert_eq!(lru.len(), 192);
        lru.touch(0); // 0 is now the most recent; 1 is the oldest
        assert_eq!(lru.insert(999, 999), vec![1]);
        assert!(lru.peek(0).is_some() && lru.peek(1).is_none());
        assert_eq!(lru.len(), 192);
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_frame_decodes_to_the_stage_size_and_never_up() {
        let bytes = png(400, 200);
        let (img, shrunk) = decode_fit(&bytes, (100, 100)).unwrap();
        assert!(shrunk);
        assert_eq!(img.size(0).width.0, 100, "fits the box, keeps the aspect");
        assert_eq!(img.size(0).height.0, 50);
        let (img, shrunk) = decode_fit(&bytes, (800, 800)).unwrap();
        assert!(!shrunk);
        assert_eq!(img.size(0).width.0, 400);
        let (img, _) = decode_fit(&bytes, (0, 0)).unwrap();
        assert_eq!(img.size(0).width.0, 400, "no layout yet: full size");
        // BGRA: the blue channel comes first.
        assert_eq!(&img.as_bytes(0).unwrap()[..4], &[30, 20, 10, 255]);
    }
}
