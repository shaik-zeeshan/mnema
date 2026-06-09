# Context Map

Mnema uses this file as the context entry point.

This context captures the domain language for the desktop capture app so architecture discussions use stable project terms.

System-wide decisions stay in [docs/adr](docs/adr/), while owner-local context lives beside the app or crate it describes.

## How to Use

- Start here instead of reading every context file.
- Open the narrowest matching local `CONTEXT.md` before changing a surface.
- Put system-wide decisions in `docs/adr/`.
- Put future context-specific decisions under `<context>/docs/adr/` and link them from that context file.
- Keep local context files focused on stable language, ownership, relationships, and resolved ambiguity for that area.

## Context Files

| Area | Context | Use for |
| --- | --- | --- |
| Desktop app | [apps/desktop/CONTEXT.md](apps/desktop/CONTEXT.md) | Svelte UI, settings surfaces, prompts, status-bar-facing actions, dashboard UX, privacy recovery flows. |
| Desktop native runtime | [apps/desktop/src-tauri/CONTEXT.md](apps/desktop/src-tauri/CONTEXT.md) | Tauri wiring, Recording Lifecycle, live privacy application, native runtime services, generated preview cache, broker authorization server. |
| App infra | [crates/app-infra/CONTEXT.md](crates/app-infra/CONTEXT.md) | SQLite state, migrations, frame/OCR pipeline, retention, search projections, storage layout, broker policy. |
| Mnema CLI | [crates/cli/CONTEXT.md](crates/cli/CONTEXT.md) | `mnema` command UX, brokered access grants, client identity, output formats, command errors, exit codes. |
| Audio transcription | [crates/audio-transcription/CONTEXT.md](crates/audio-transcription/CONTEXT.md) | Audio transcription jobs, local providers, model selection, transcript spans, audio search units. |
| Speaker analysis | [crates/speaker-analysis/CONTEXT.md](crates/speaker-analysis/CONTEXT.md) | Speaker analysis jobs, diarization policy, speaker turn alignment, speaker continuity. |
| Secret redaction | [crates/secret-redaction/CONTEXT.md](crates/secret-redaction/CONTEXT.md) | Secret detection/redaction policy for searchable, copied, snippet, and broker-visible derived text. |
| User Context | [docs/user-context/CONTEXT.md](docs/user-context/CONTEXT.md) | Standing, continuously-updated understanding of the user derived from captures: Activity (evidence) and Conclusion (distilled belief) layers. Storage + deterministic Confidence Policy / Sensitive Category Guardrail live in `crates/app-infra/src/user_context`; the Reasoning Engine derivation worker + Tauri commands live in `apps/desktop/src-tauri/src/user_context`. |

## ADR Scope

Root [docs/adr](docs/adr/) remains the place for system-wide decisions. If a future decision is local to one context, create `<context>/docs/adr/` in that owner directory and link it from both the local context file and this map when it becomes important to discover globally.

## Migration Note

The previous root `CONTEXT.md` was split into these files to keep context reads bounded.
