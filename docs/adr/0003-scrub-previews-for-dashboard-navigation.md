# Scrub previews for dashboard navigation

Mnema will use **Scrub Previews** as temporary, low-cost, display-sized visual representations while a user navigates the dashboard timeline. Exact frame previews remain authoritative for parked frames, OCR, copy, download, and persisted **Captured Frame** truth.

This deliberately introduces a second preview quality tier because fast timeline scrubbing should prioritize continuity and low CPU use, while inspection and content actions require exactness. We chose batched, indexed, reduced-size Scrub Preview generation over per-frame exact extraction during scrubbing, direct video rendering, or raw pixel-buffer delivery because it keeps the existing Tauri asset-file frontend model while avoiding repeated full-size AVFoundation extraction work.
