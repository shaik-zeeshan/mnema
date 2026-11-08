//! The stage's [`ImageCache`]: decoded frames, LRU at 192 entries
//! (`PREVIEW_CACHE_MAX_ENTRIES`), failures remembered 5 s
//! (`PREVIEW_FAILURE_CACHE_TTL_MS`) before a retry, and every evicted image
//! handed back to the GPU atlas with `drop_image` (a full-size frame is ~20 MB
//! of atlas; WebKit's equivalent leak is why the Svelte stage paints blob URLs).
//!
//! Loads go through GPUI's `ImageCacheItem`, which reads and decodes on the
//! background executor and swaps RGBA to BGRA, so the main thread never decodes;
//! an evicted image's pixels are freed there too. The GPU upload happens when the
//! stage first paints an image: one per stage swap, which the scrub engine's
//! display throttle keeps ≥ 80 ms apart while scrubbing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    hash, App, AppContext as _, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage,
    Resource, Window,
};

pub const MAX_ENTRIES: usize = super::scrub_engine::CACHE_MAX;
pub const FAILURE_TTL: Duration = super::scrub_engine::FAILURE_TTL;

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

struct Slot {
    item: ImageCacheItem,
    /// When a load was first seen failed; a retry is due `FAILURE_TTL` later.
    failed_at: Option<Instant>,
}

pub struct FrameImages {
    slots: Lru<Slot>,
    /// Loads started (first sights and retries); tests read it.
    pub loads: u32,
}

type Loaded = Option<Result<Arc<RenderImage>, ImageCacheError>>;

impl FrameImages {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let e = cx.new(|_| Self {
            slots: Lru::new(MAX_ENTRIES),
            loads: 0,
        });
        cx.observe_release(&e, |cache, cx| {
            for slot in cache.slots.drain() {
                if let Some(Ok(image)) = slot.item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        e
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Already decoded (the stage swaps to it without a blank frame).
    pub fn ready(&self, resource: &Resource) -> bool {
        matches!(
            self.slots.peek(hash(resource)).map(|s| s.item.get()),
            Some(Some(Ok(_)))
        )
    }

    /// Decoding failed (and the failure is still cached).
    pub fn failed(&self, resource: &Resource) -> bool {
        matches!(
            self.slots.peek(hash(resource)).map(|s| s.item.get()),
            Some(Some(Err(_)))
        )
    }
}

impl ImageCache for FrameImages {
    fn load(&mut self, resource: &Resource, window: &mut Window, cx: &mut App) -> Loaded {
        let now = cx.background_executor().now();
        let key = hash(resource);
        if let Some(slot) = self.slots.touch(key) {
            let result = slot.item.get();
            match (&result, slot.failed_at) {
                (Some(Err(_)), Some(at)) if now.duration_since(at) >= FAILURE_TTL => {
                    self.slots.remove(key); // retry below
                }
                (Some(Err(_)), None) => {
                    slot.failed_at = Some(now);
                    return result;
                }
                _ => return slot.item.use_image(window),
            }
        }
        self.loads += 1;
        let item = ImageCacheItem::new(resource, cx);
        let result = item.use_image(window);
        for evicted in self.slots.insert(
            key,
            Slot {
                item,
                failed_at: None,
            },
        ) {
            if let Some(Ok(image)) = evicted.item.get() {
                // The last reference frees ~10 MB of pixels: do that off the main thread.
                let last = image.clone();
                cx.drop_image(image, Some(window));
                drop(evicted);
                cx.background_spawn(async move { drop(last) }).detach();
            }
        }
        result
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
}
