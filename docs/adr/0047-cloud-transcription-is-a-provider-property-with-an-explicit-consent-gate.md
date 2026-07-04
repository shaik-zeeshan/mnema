# Cloud transcription is a provider property with an explicit consent gate

## Status

Accepted.

## Context

Audio transcription providers were deliberately local-only for v1 (`local_whisper`, `apple_speech_on_device`, `parakeet` — recorded as a resolved ambiguity in `crates/audio-transcription/CONTEXT.md`). Adding Deepgram reopens that decision, and it is the first feature that ships raw captured audio off-device — the AI features (ADR 0028/0033) send selected context to cloud models, never raw capture output. The AI settings had already gone provider-centric with per-instance identities (ADR 0034/0035), which was the obvious template to copy.

## Decision

Cloud-ness is a property of an **Audio Transcription Provider**, not a new category:

- **`deepgram` is a fourth provider ID** in the existing picker — same `AudioTranscriptionSettings` shape (`provider` + `modelId`), same admission and freeze-at-admission job semantics as the local providers.
- **Availability = API key present.** No key → segments remain eligible and wait for backfill, exactly like a segment waiting on a missing local model download. No new availability concept.
- **Models are vendor-hosted identifiers**: `nova-3` (default) and `nova-2`, exposed because `nova-2` covers a broader language list — a language-coverage choice, not a quality tier. The installed/missing/downloading/failed model lifecycle applies only to app-managed local models.
- **Consent is a blocking dialog on every switch to Deepgram**, in Settings only (`@tauri-apps/plugin-dialog`). Cancelling reverts the selection. No persisted consent flag — the dialog re-fires on each switch, which is rare enough that re-asking is a feature. Deepgram does not appear in onboarding.
- **Provider selection stays global across recording sources.** Microphone and system-audio segments always use the same provider; per-source routing (e.g. "mic local, system audio cloud") is explicitly out of scope.
- **The key lives in the existing keychain store** (`crates/app-infra/src/ai_provider_key_store.rs`, same keychain service) under the account **`transcription.deepgram`**. Thin dedicated Tauri commands apply the `transcription.` prefix server-side; the frontend never passes account strings.

## Considered Options

- **An ADR 0034-style cloud category with per-instance provider ids.** Rejected: AI settings needed instance identity because multiple keyed providers coexist with pins; transcription has exactly one active provider, frozen per job at admission — machinery that already exists. A category split is architecture for a second cloud vendor that doesn't exist.
- **A persisted "user consented" flag.** Rejected: zero-state re-asking on every switch is simpler and safer; switching providers is rare.
- **Deepgram in the onboarding provider selector.** Rejected: drags key entry, consent, and a network dependency into first-run; cloud users find it in Settings.
- **Unprefixed `deepgram` keychain account.** Rejected: AI-provider accounts in the same keychain service are user-created instance ids (ADR 0035); an unprefixed name is one user-named instance away from a collision.

## Consequences

- The consent dialog copy is the privacy boundary: it must state that microphone **and** system-audio recordings upload to Deepgram under the user's own account and data policies, and that only future segments are affected.
- The system-audio speech-activity gate now has a cost dimension, not just a compute one: it keeps silent segments from being uploaded and billed.
- Vendor-side data handling is governed by the user's own Deepgram account (BYO-key); Mnema stores nothing new off-device.
- `SUPPORTS.md` gains its first transcription provider whose availability is not platform-bound (plain HTTPS upload of the segment file — no local decode path required).
