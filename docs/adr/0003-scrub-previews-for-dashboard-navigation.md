# Scrub previews for dashboard navigation

Mnema will use **Scrub Previews** as temporary, low-cost, display-sized visual representations while a user navigates the dashboard timeline. Exact frame previews remain authoritative for parked frames, OCR, copy, download, and persisted **Captured Frame** truth.

This deliberately introduces a second preview quality tier because fast timeline scrubbing should prioritize continuity and low CPU use, while inspection and content actions require exactness. We chose batched, indexed, reduced-size Scrub Preview generation over per-frame exact extraction during scrubbing, direct video rendering, or raw pixel-buffer delivery because it keeps the existing Tauri asset-file frontend model while avoiding repeated full-size AVFoundation extraction work.

Generated **Scrub Preview** files are app-owned cache artifacts under Tauri `app_cache_dir()`, not durable recording artifacts beside the source `.mov`. The source segment recording and frame index remain under the recordings tree, while missing scrub cache entries are regenerated opportunistically or treated as absent navigation previews.

**Scrub Preview Generation** runs outside the active scrub interaction path. Timeline movement can request availability, but cache misses should enqueue background generation and return cache hits or absence immediately, because synchronous `.mov` extraction during scrubbing reintroduces the latency and CPU spikes this preview tier exists to avoid.

Finalized screen **Capture Segment** values enqueue full one-second-interval **Scrub Preview Generation** because **Capture Segment Duration** is capped at 5 minutes, bounding the automatic work to at most 300 previews per segment. Dashboard visible-window demand remains higher priority so user navigation is not blocked behind background warming for recent segments.
