# Segment-based data retention policy

Mnema will model retention around logical `capture_sessions` and concrete `capture_segments`. A capture session represents one user recording, while capture segments are the actual deletion units for produced screen, microphone, and system-audio artifacts.

Retention policies are `never`, `days_7`, `days_14`, and `days_30`, with `never` as the default. Calendar policies use local-device calendar semantics: `days_7` keeps today plus the previous 6 local calendar days, and cleanup deletes segments whose `ended_at` is before the computed local-midnight cutoff.

Retention cleanup is scoped to the current `saveDirectory` and active SQLite database. It deletes capture segments and derived frame/audio/processing/speaker data, including segment-derived voice embeddings and recognition rejections, but preserves user-authored `person_profiles`. Cleanup must skip active capture output and running processing/finalize work; queued, failed, and completed jobs can be deleted with their subject.

The Tauri layer exposes dry-run, manual cleanup, and status commands and emits `timeline_data_changed` with reason `retention` after timeline-visible deletion. The dashboard should prune loaded stale rows and keep the active retained item when possible instead of forcing a full reset.
