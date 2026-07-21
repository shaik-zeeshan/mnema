# Mnema

**Private, searchable memory for your Mac.** Mnema continuously records your screen, microphone, and system audio, turns it into a searchable, scrubbable, AI-queryable record of what you did — and keeps every byte on your device.

**[mnema.day](https://mnema.day)** · [Download for macOS](https://mnema.day)

## What it does

- **Continuous capture** — screen, microphone, and system audio recorded quietly in the background, segmented and indexed as you work.
- **Recall** — full-text and semantic search over everything you've seen (OCR) and heard (transcription), plus a scrubbable timeline of your day.
- **Ask AI** — chat with your own history, or summon Quick Recall from anywhere like Spotlight. Works with cloud providers (Anthropic, OpenAI) or fully local models (Ollama, Llamafile) — your choice.
- **Speaker analysis** — on-device diarization tells you who said what in meetings.
- **Agent access** — a brokered `mnema` CLI lets AI agents (Claude Code, MCP clients) search your activity with your permission. See [mnema.day/agents](https://mnema.day/agents).

## Private by design

- Everything is stored in an **SQLCipher-encrypted database on your Mac**. Nothing leaves your device unless you explicitly opt in (e.g. a cloud AI provider or Deepgram transcription, each behind its own consent gate — with local alternatives for both).
- **Privacy exclusion list**: apps you exclude are filtered out of both screen capture and system audio.
- API keys and secrets live in an encrypted vault backed by the macOS keychain, never in config files.
- Retention windows let you cap how long raw recordings are kept; delete-recent removes everything from a time range.

## Requirements

- macOS on Apple Silicon (system-audio capture requires macOS 15+).
- Screen Recording, Microphone, and (optionally) System Audio permissions.

## Tech

Tauri 2 + Svelte 5 frontend, Rust backend. Capture runs on native macOS frameworks (ScreenCaptureKit, AVFoundation, Core Audio process taps); OCR via Apple Vision; transcription via Apple Speech, local Whisper/Parakeet, or Deepgram; on-device embeddings via candle on the Apple GPU. Platform support details: [SUPPORTS.md](SUPPORTS.md).

```
apps/desktop     Tauri + Svelte desktop app
apps/web         mnema.day marketing site
crates/          capture, storage, AI runtime, and processing crates
docs/adr         architecture decision records
```

## Building from source

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites and build steps.

## License

[AGPL-3.0](LICENSE). Mnema is open source; the packaged app is sold with a one-time purchase license at [mnema.day](https://mnema.day).
