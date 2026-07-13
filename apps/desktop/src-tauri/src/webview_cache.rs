// Purge WKWebView's decoded-image memory cache when a window loses focus.
//
// The dashboard timeline and Quick Recall scrub through hundreds of distinct
// frame-preview asset URLs. WebKit keeps one decoded IOSurface per URL in its
// memory cache and only releases them under system memory pressure — which
// macOS answers by swapping instead of purging, so the WebContent process
// footprint grows without bound (observed: 2.2 GB after two days, ~90% of it
// decoded frame previews). Purging `WKWebsiteDataTypeMemoryCache` drops the
// surfaces; the previews are local files that re-decode in milliseconds on
// next view.
//
// Blur-only on purpose: while a window is focused the receipt playback's
// warm-decode trick (receipt-frames.ts) relies on the cache for its instant
// frame swaps, and an active scrub should never re-decode mid-gesture.

use std::sync::Mutex;
use std::time::{Duration, Instant};

// ponytail: fixed 60s floor; make it adaptive only if blur-purge ever shows up
// in profiles.
const MIN_PURGE_INTERVAL: Duration = Duration::from_secs(60);

static LAST_PURGE: Mutex<Option<Instant>> = Mutex::new(None);

fn purge_due(last: Option<Instant>, now: Instant, min_interval: Duration) -> bool {
    last.map_or(true, |then| now.duration_since(then) >= min_interval)
}

/// Called from the shared window-event seam on every `Focused(false)`.
/// Rate-limited so cmd-tab flurries don't purge-thrash; no-op off macOS
/// (WebView2/WebKitGTK manage their own caches).
pub fn purge_webview_memory_cache_on_blur(app: &tauri::AppHandle) {
    let now = Instant::now();
    {
        let mut last = LAST_PURGE.lock().expect("webview-cache purge lock");
        if !purge_due(*last, now, MIN_PURGE_INTERVAL) {
            return;
        }
        *last = Some(now);
    }

    #[cfg(target_os = "macos")]
    {
        // WKWebsiteDataStore is main-thread-only.
        let _ = app.run_on_main_thread(|| unsafe { macos::purge_memory_cache_now() });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use block::ConcreteBlock;
    use objc::{msg_send, sel, sel_impl};

    type Id = *mut objc::runtime::Object;

    #[link(name = "WebKit", kind = "framework")]
    extern "C" {
        static WKWebsiteDataTypeMemoryCache: Id;
    }

    pub(super) unsafe fn purge_memory_cache_now() {
        let data_store: Id = msg_send![objc::class!(WKWebsiteDataStore), defaultDataStore];
        let types: Id =
            msg_send![objc::class!(NSSet), setWithObject: WKWebsiteDataTypeMemoryCache];
        let since: Id = msg_send![objc::class!(NSDate), distantPast];
        // The completion handler is non-nullable; pass an empty block.
        let done = ConcreteBlock::new(|| {}).copy();
        let _: () = msg_send![
            data_store,
            removeDataOfTypes: types
            modifiedSince: since
            completionHandler: &*done
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_blur_always_purges() {
        assert!(purge_due(None, Instant::now(), MIN_PURGE_INTERVAL));
    }

    #[test]
    fn blur_within_interval_is_skipped() {
        let now = Instant::now();
        assert!(!purge_due(Some(now), now + Duration::from_secs(5), MIN_PURGE_INTERVAL));
    }

    #[test]
    fn blur_after_interval_purges_again() {
        let now = Instant::now();
        assert!(purge_due(Some(now), now + MIN_PURGE_INTERVAL, MIN_PURGE_INTERVAL));
    }
}
