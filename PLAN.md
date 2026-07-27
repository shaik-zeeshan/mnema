# Plan: Onboarding rework — one resolved settings path, non-blocking model setup

Derived from issue #195, a grilling session, and five design directions (chosen: Cinematic /
Rewind, `mockup-3d-cinematic.html`). Supersedes the issue where they disagree — the issue's
"Solution" section was written before the code was checked, and three of its mechanisms turned
out to be unimplementable or unnecessary as specified.

## Problem

A new user is asked to make eleven independent configuration decisions before Mnema records
anything. The "Use recommended defaults" button makes it worse: it *selects* a transcription
model without downloading it, dropping the user onto a row already flagged as needing
attention with no explanation.

Three things compound it:

1. **A running download blocks progress entirely.** A downloading model is classified
   identically to a missing one (`onboarding-attention.ts:93-131` — every predicate reads a
   single `model.available` boolean), so it counts as an unresolved attention item, so the user
   cannot leave the configure step while it downloads.
2. **Feature dependencies only cascade downward.** Turning transcription off turns speaker
   separation off, but turning the microphone on does not turn transcription on. A user who
   enables audio capture gets silent audio with no transcript.
3. **A missing model silently destroys work.** A job that runs against an absent model takes
   the terminal-failure path: `failure_count` increments, audio backoff is `[60, 300]` seconds,
   cap is 3 — so the job is dead in about six minutes. The audio file survives, the transcript
   never exists, and nothing ever retries. This is a bug **today**, independent of onboarding:
   downloading or deleting a model from Settings while recording hits it.

## Solution

One flow where every choice arrives already answered, and where nothing about downloads or
model state can block anyone.

```
Welcome → Permissions → Capture & Storage → Your settings ⇄ Change settings → Setup → Voice → Finale
```

Permissions come early because they are the resolver's input. A pure resolver turns
`(permissionIntents, installedModels, currentSettings)` into resolved settings. **Your
settings** is a read-only manifest of the result with exactly two actions. Downloads never
block. Capture starts at the Finale.

An optional **Voice** step creates a voiceprint for the account owner from a short guided
read-aloud, so Mnema labels the user's turns by name instead of "Speaker 1". It is skippable,
re-runnable from Settings, and never gates finishing.

The model-readiness fix is not a new retry policy — it is **claim-time gating**. The job claim
query already skips jobs whose model is locked; a downloading model takes a lock, so its jobs
park for free instead of burning attempts.

## User Stories

1. As a new user, I want Mnema to choose sensible settings for me, so that I do not have to
   understand eleven features before I can record anything.
2. As a new user, I want to see exactly what was chosen before I commit, so that I am not
   agreeing to something I cannot inspect.
3. As a new user, I want to know how much will be downloaded and roughly how much disk is used
   per day, so that the cost is visible before I accept it.
4. As a new user, I want the app to start capturing as soon as I finish, even if models are
   still downloading, so that I do not lose my first hour to a progress bar.
5. As a new user, I want proof that capture is working at the end, so that I trust it is
   running and did not silently fail.
6. As a new user who denies the Microphone, I want the flow to continue and simply not enable
   audio features, so that one "no" does not dead-end my setup.
7. As a new user, I want to be told honestly that System Audio permission cannot be queried, so
   that I am not shown a confident green check that is a guess.
8. As a new user who grants Screen Recording and must relaunch, I want to resume rather than
   start over, so that a relaunch does not cost me the setup.
9. As a user who enables microphone capture, I want transcription and speaker separation
   enabled automatically, so that my audio is actually searchable.
10. As a user who turns off every audio source, I want transcription and speaker separation to
    follow, so that no orphaned feature runs against nothing.
11. As a user, I want to choose a Retention window explicitly with its disk consequence shown,
    so that the app neither accumulates forever without my deciding nor deletes my data on a
    default I skimmed.
12. As a user, I want a feature whose model has not arrived to show "Preparing", so that I do
    not report a bug for a download in flight.
13. As a user whose download fails, I want the actual reason at the failed item, so that I am
    not staring at an indefinite spinner hiding a network error.
14. As a user who cancels a download, I want the dependent feature cleanly turned off, so that
    cancelling does not leave a half-configured state.
15. As a user with insufficient disk, I want to be told before the download starts.
16. As a power user, I want one "Change settings" entry point revealing every setting, so that
    I am not fighting a wizard to reach a knob I know I want.
17. As a user who re-enters onboarding, I want my provider keys, privacy-listed apps, and
    deliberate settings changes left untouched, so that re-running setup costs me nothing.
18. As a user, I want the Reasoning Engine left off unless I configure it, so that enabling
    recall does not implicitly enable an outbound AI provider.
19. As a user, I want to record a short sample of my voice during setup, so that Mnema can
    recognise me in my recordings.
20. As a user, I want enrollment to take about fifteen seconds and give me the words to read,
    so that I am not improvising.
21. As a user, I want to skip enrollment entirely and still finish, so that I am not forced to
    give a biometric sample to use the product.
22. As a user, I want to be told plainly that the voiceprint never leaves my device, so that I
    can judge the privacy trade before recording.
23. As a user recorded with someone else talking nearby, I want the sample rejected with an
    explanation and a retry, so that Mnema does not learn the wrong voice as mine.
24. As a user whose sample is too short or too quiet, I want to be told which, so that my retry
    succeeds.
25. As a user who enrolled, I want recognition against saved people switched on as part of
    enrolling, so that the act of enrolling actually does something.
26. As a user who deliberately named a voice, I want that voiceprint to survive Retention
    deleting the recording it came from, so that recognition does not silently stop working.
27. As a user, I want to delete my voiceprint and Person Profile from Settings, so that I can
    withdraw the biometric sample I gave.
28. As a user, I want to enroll later from Settings if I skipped it, so that the decision is
    reversible.
29. As a user, I want to be told honestly that recognition is imperfect and will not label
    every turn, so that my expectations match reality.
30. As a user who enrolled, I want my own turns labelled with my name automatically when the
    match is confident, so that enrolling actually changes what I see.
31. As a user, I want an automatic label to be visibly automatic and reversible, so that I can
    audit and correct what was decided for me.
32. As a user, I want to turn automatic labelling off and go back to being asked, so that a
    biometric label is never applied against my wishes.

## Implementation Decisions

### Flow shape

- **Greenfield.** The 12-row accordion (`FeatureStack` + `FeatureRow` + 12 `*Body.svelte`) is
  replaced, not preserved. The per-feature body components are the reusable part; the accordion
  chrome is not.
- **Eight screens** as above. Permissions is a dedicated early step because every downstream
  setting derives from it.
- **"Settings", never "plan".** The resolved output is *the user's settings*. Screen 4 is
  **Your settings**; screen 5 is **Change settings**.
- **Your settings is a read-only manifest.** No row expands into an editor. Two actions:
  *Change settings* and *Start*. Changing anything is a round trip to screen 5, which
  re-resolves the manifest on return.
- **Density rule, enforced:** one line per row, one sentence per screen, ≤7 content lines per
  screen. Provider names, model identifiers, and per-row byte sizes live only on *Change
  settings* — which is deliberately dense, and is what lets the other screens be light.
- **Motion is scoped to the bookends.** Welcome and Finale only; the six working screens carry
  no ambient motion, only functional feedback (progress bar, level meter, focus states).

### Gating — the flow has exactly two hard gates

| Step | Gate |
|---|---|
| Welcome | none |
| Permissions | **none.** Soft warning only. macOS never re-prompts after denial, so a hard gate would trap the user with no in-app recovery. |
| Capture & Storage | **storage path exists and is writable**; **volume has room for the download set**. Plus existing custom resolution (16–8192 px) and bitrate (1–40 Mbps) validation. |
| Your settings | none |
| Change settings | AI features cannot be left on with no credentials |
| Setup | **none, ever.** Continue is live on arrival and never disables. |
| Voice | none. Skip is first-class. |
| Finale | none. Capture starts here. |

The current "attention item" concept — where a selected-but-missing model blocks finishing —
is deleted outright. That deletion is most of this work.

### Setup resolver

- New pure module, no Svelte, no `invoke`. Interface:
  `resolveSetup(permissionIntents, installedModels, currentSettings) → ResolvedSettings`,
  where the result enumerates feature enablements, provider/model selections, and an ordered
  download work-list.
- **All capture sources default on. The permission grant is what makes each real.** A source
  whose permission is missing renders as such on its row rather than vanishing.
- Provider defaults: Apple Vision (OCR, zero bytes), local Whisper `base` (148 MB — the exact
  figure is `audio_transcription_models.rs:1258`, 147,951,465 bytes), speakrs (speaker
  analysis), `nomic-embed-text-v1.5` (semantic search). **Deepgram is never selected by the
  resolver** — cloud transcription stays Settings-only behind its consent gate.
- **Work-list order is fixed: speakrs → Whisper base → nomic.** Speakrs is first because the
  Voice step (when built) cannot run its embedder without it.
- Already-installed models are omitted from the work-list. That is what makes re-entry cheap.
- The resolver replaces the welcome-screen preset entirely. The preset's one eager side effect
  worth keeping — applying recommended privacy-listed apps — becomes resolved data, not an
  immediate mutation.

### Permissions

- Requested **one at a time**, Screen Recording first.
- **System audio keys off intent, not grant.** Its authorization cannot be read at all (ADR
  0052; `feature-model.ts:150-154` documents why it can never carry a lock). The screen shows a
  **Request** button plus **Request again** — a closed prompt is indistinguishable from a
  denial — and states plainly that macOS will not confirm it. Never a green check.
- Denied permissions deep-link to the correct System Settings pane with a re-check.
- Granting Screen Recording may require relaunching Mnema; offer the relaunch.
- Accessibility (browser URLs) is offered last, marked optional, and only when a Gecko browser
  is installed.

### Capture & Storage

- **Capture Rate is the app's existing log-spaced slider**, not preset tiers
  (`lib/components/capture-rate.ts`: stops `0.1, 0.5, 1, 2, 3, 5, 10, 15, 30, 45, 60` seconds,
  default 2s). The UI speaks in "one snapshot every X", never in fps.
- **Storage figures are measured, not invented.** Anchor: 270 MB/day total at one snapshot
  every 3s, 720p, medium bitrate, pause-on-inactivity on — measured over a complete 14-day
  window (3.00 GB recordings + 773 MB index). Storage scales linearly with frame rate
  (`compute_effective_screen_bitrate_bps` multiplies by frame rate), so the ladder derives from
  the anchor: **~400 MB/day at the 2s default.** One footnote states the measurement basis and
  the pause-on-inactivity dependency; the figures are not otherwise annotated.
- **Retention defaults to `Never` (keep everything)** and is presented as an explicit choice.
  It is the only setting whose wrong guess destroys data with no undo, so the app never
  defaults to deleting. Four options; only the selected one shows its consequence. **No chart.**
- Excluded apps: recommended list pre-applied, rendered as a summary with an Edit affordance,
  not an inline picker.

### Feature dependency rules

- `featureLockReason` (`feature-model.ts:146`) and the controller's hand-written toggle switch
  collapse into one pure module: `applyToggle(state, featureId) → state`.
- **Cascades run both directions.** Enabling an audio source enables transcription for that
  source and enables speaker separation; disabling the last audio source disables both. The
  existing downward cascades must not regress.
- Turning a feature *off* is always permitted.
- System audio carries no screen-liveness or screen-permission lock (ADR 0052).

### Model readiness — claim-time gating, not a retry policy

This replaces the issue's proposed approach and an earlier transient-liveness proposal. Both
were worse.

- `claim_next_queued_job_matching_processor` (`store.rs:1305`) already excludes jobs via
  `NOT EXISTS (SELECT 1 FROM processing_model_cleanup_locks …)`. **A model that is downloading
  or absent takes a lock, so its jobs are never claimed.** They sit `queued` — not running, not
  failing, not burning an attempt, not polling.
- Download completes → lock released → jobs claimable on the next drain, with all three
  genuine-failure attempts intact.
- Download cancelled → the dependent feature is disabled (below), so no new jobs are created.
- Download failed and never retried → jobs stay parked indefinitely, which costs nothing.
- **Rename the table** to `processing_model_locks` with a `reason` column
  (`'cleanup' | 'downloading'`). The predicate is unchanged. *Confirm the repo's no-users-yet
  premise before editing an applied migration in place.*
- **"Preparing" becomes free**: a job that is `queued` while its model is locked *is* the
  Preparing state, derivable from data the dashboard already loads (`ProcessingJobDto`,
  `get_processing_result`, `latestProcessingJobForProcessor` in `routes/+page.svelte`).
- **Do not add a transient-liveness variant for missing models.** It would re-poll every 60s
  forever when a model never arrives.
- The frontend still needs a four-state classifier (Ready / Downloading / Missing / Failed) to
  render the Setup screen. Only `Missing` and `Failed` are interesting there, and neither
  blocks finishing.

### Download orchestration

- The work-list is driven from the frontend against the four existing per-subsystem commands
  (`start_ocr_model_download`, `start_audio_transcription_model_download`,
  `start_speaker_analysis_model_download`, `start_semantic_search_model_download`). No new Rust
  download machinery.
- A pure reducer folds the four progress streams plus known byte totals into one byte-weighted
  aggregate percent and a current-item label. Progress never moves backwards. Failure surfaces
  at the failed item with its real error.
- **Cancel disables the dependent processing feature but leaves the capture source on.**
  Cancelling Whisper turns transcription off; it does not turn the microphone off. The audio is
  still worth keeping and becomes transcribable later.
- Free-disk preflight before the work-list starts, not per-item after failure.

### Capture start

- **Capture starts at the Finale**, after the reserved Voice slot, so the bounded recorder (when
  built) has the microphone free.
- No download gates recording. Screen capture, OCR (Apple Vision), microphone capture, and
  system audio capture all need zero models; all three model-backed features are post-processing
  over stored segments.
- The status bar is unaffected — recording is recording (`status_bar.rs`).
- **The Finale has four states**: waiting for the first frame · success (first frame + first OCR
  hit) · **nothing granted, capture not running** (the real path of a user who denied Screen
  Recording — needs a message and a next action, never a blank screen) · capture failed to
  start, with the real reason.

### Idempotence and re-entry

- **Saved settings win; the resolver only fills gaps.** Re-entering must never silently
  re-enable something the user deliberately turned off.
- Re-entry must not touch secret-vault contents, Reasoning Engine provider configuration, or
  privacy-listed apps.
- Settings are committed atomically in one shot at finish. The existing privacy-slice partial
  sync discipline is preserved (`onboarding-privacy-sync.ts`).
- **Relaunch resume: persist the step index and always resume at Permissions.** A relaunch only
  ever originates from granting Screen Recording, so resuming there costs one Continue press and
  avoids serialising half-finished settings.
- Onboarding stays gated on its completion timestamp and does not re-run on launch.

### AI features

- The row is labelled **"AI features"**, not "Ask AI" — it configures the Reasoning Engine,
  which also powers digests and User Context. Settings already models this correctly with a
  separate Providers section (`lib/settings/groups.ts:161,170-172`), so there is no naming
  conflict.
- It appears on *Your settings* **visibly present and unticked**. Consent is never pre-ticked.
- Setup lives on *Change settings*, reusing `createOnboardingAiStore`. **Local providers
  (Ollama, Llamafile) are offered before cloud** — no key, nothing leaves the machine.
- **Configuring a provider enables the feature.** Otherwise the user types a key and nothing
  uses it.

### Voice enrollment

Schema already in place: `person_profiles` and `person_voice_embeddings`
(`migrations/0010_speaker_analysis.sql`), the latter with a nullable `source_cluster_id`.

- **Bounded recorder** — a thin new Tauri command recording a fixed-length clip from the
  microphone to a temp file. A hardware adapter holding no judgment.
- **Live-session guard** — the recorder must not run while a capture session is live. This is
  satisfied during onboarding by capture starting at the Finale, but is **required regardless**
  for the Settings re-enroll path: pause the microphone session, record, resume. Build it once;
  it is what lets enrollment run anywhere.
- **Enrollment embedder** — a Rust module taking an audio path and returning a voiceprint or a
  typed rejection: `MultipleSpeakers` (more than one cluster in the clip — someone else was
  audible), `TooShort`, `NoSpeech`. Reuses the existing speakrs entry point, which accepts an
  arbitrary path and touches no database. **All enrollment judgment lives here.**
- **Storage** — written against the owner's Person Profile with `source_cluster_id = NULL`. A
  migration adds a flag marking one Person Profile as the account owner.
- **Matching is already built and unchanged** — `list_person_enrollments_for_speaker_model`
  (`store.rs:2522`) loads profile voiceprints and writes recognition *suggestions* a human
  confirms. Enrollment just gives it something to match. **Thresholds are not changed.**
- **Enrolling switches recognition on.** `default_speaker_recognition_enabled()`
  (`recording.rs:383`) returns `false` today, so without this the voiceprint would be loaded by
  nothing.
- **Two screen states.** Speakrs Ready → inline enrollment with a supplied sentence, level
  meter, playback confirmation, and a working retry loop. Speakrs still downloading → says so,
  with "Set this up later" as the **primary** action, handing off to the same surface mounted on
  the dashboard. Never a spinner, never a wall.
- **Honesty, in one line each:** the voiceprint never leaves the device; recognition is
  imperfect and will not label every turn.
- **Identity is never inferred from capture family.** A microphone records whoever is in the
  room. Channel provenance describes *where* audio came from and carries no identity
  information; no part of this work may treat it as if it does.

### Owner-only high-confidence auto-linking

Recognition today never auto-links: every match is a suggestion a human confirms, so a
confirmed identity does not carry forward and each new recording re-asks. That is the real
"recognition doesn't stick" complaint. The account owner is the one case where it is safe to
close that loop — they gave a clean, close-mic, deliberately-recorded sample and explicitly
asked to be recognised.

Existing policy, unchanged (`speaker-analysis/src/providers/shared.rs:33-42`):
`MIN_RECOGNITION_SUGGESTION_SCORE = 0.60` to surface at all · `HIGH_RECOGNITION_SUGGESTION_SCORE
= 0.72` for `High` · `PERSON_AMBIGUITY_MARGIN = 0.05` suppresses a contested match ·
`REJECTED_PERSON_SIMILARITY_THRESHOLD = 0.80` blocks a previously-rejected person.

**The rule:** a cluster whose suggestion is for the **account-owner profile** *and* is
`RecognitionConfidence::High` is linked automatically. Everyone else stays suggest-and-confirm.

- **Thresholds are not changed.** The rule is a consumer of the existing tiers, not a retune.
  All three existing guards still apply first — an ambiguous or previously-rejected match never
  reaches the auto-link.
- **Reuse `link_speaker_cluster_to_person`**, the same path confirm takes. No parallel path.
- **Auto-links must pass `add_embedding = false`.** A human confirmation may add a voiceprint to
  the profile; an automatic one must not. Otherwise a slightly-wrong auto-link writes a
  contaminated voiceprint that makes the next wrong match *more* likely — a feedback loop that
  degrades the profile with no human ever in it.
- **Auto-linked must be distinguishable from human-confirmed** in the data and visibly in the
  UI, so the user can audit what was decided for them and undo it. Rejecting an auto-link works
  exactly as rejecting a suggestion does, and the existing rejection guard stops it recurring.
- **One setting**, on the enrollment surface, default on once enrolled: *label my voice
  automatically*. Turning it off reverts to suggest-and-confirm. Biometric labels applied
  without asking need an off switch.
- **Known limitation, deliberately accepted:** the 0.72 tier has never been measured for
  cross-channel matching (clean-mic voiceprint vs. clusters from compressed system audio). The
  mitigations are that auto-links are visible, reversible, and never feed the profile. If
  cross-channel proves weak in practice, the narrower fix is to restrict auto-linking by
  measured score rather than by capture family — **never** by channel, which would violate the
  no-identity-from-capture-family rule above.

### Retention correction (narrower than the issue states)

The issue claims enrollment would inherit a retention bug. **It would not.** The sweep is
`DELETE FROM person_voice_embeddings WHERE source_cluster_id IS NOT NULL AND NOT EXISTS(…)`
(`capture_retention.rs:1484-1488`), and enrollment writes `source_cluster_id = NULL`, so
enrolled voiceprints are **already immune**.

The real victim is the **audio-drawer "remember as profile" flow**, whose embeddings carry a
`source_cluster_id` and are deleted when the originating recording ages out — silently breaking
recognition one retention window after the user set it up.

- Fix: add `is_deliberate INTEGER NOT NULL DEFAULT 0` to `person_voice_embeddings`, set it when
  a user names a cluster, and add `AND is_deliberate = 0` to the sweep. One migration, one
  clause. Cluster-derived transient data still gets collected.

### Profile deletion

`delete_person_profile` exists (`app_infra.rs:5094`, registered `lib.rs:798`) and has **zero
frontend callers**. Storing a biometric sample with no delete affordance is not acceptable, so
Settings gains one, alongside a re-enroll entry point and a legible read-out of whether
recognition is on and whether a voiceprint exists.

## Testing Decisions

Tests exercise externally observable behaviour: given these permissions and this installed
state, the resolver produces these settings; given this toggle, the dependency module produces
this state. No running app, display, or network. They follow the shape of the existing pure
onboarding helpers (`onboarding-privacy-sync.ts`, `onboarding-attention.ts`).

1. **Setup resolver** — table-driven over permission combinations (screen only; screen + mic;
   screen + system-audio intent; all; none) crossed with installed-model states. Asserts the
   resolved feature set, model selections, and that the work-list omits installed models and is
   ordered speakrs → Whisper → nomic. **Must include a case asserting Deepgram is never
   selected.**
2. **Feature dependency rules** — every toggle in both directions, including the new upward
   cascades and the existing downward ones, plus lock-reason behaviour when permission is absent.
3. **Model readiness classifier** — all four states, and specifically that `Downloading` does
   not block finishing while nothing else does either.
4. **Aggregate progress reducer** — concurrent streams, out-of-order events, an item completing,
   an item failing, and the invariant that progress never moves backwards.
5. **Claim-time model gating (Rust, app-infra)** — a job whose model is locked is not claimed;
   releasing the lock makes it claimable; **a parked job never increments `failure_count`**;
   genuine failures still cap at 3. Follows the existing store tests, including
   `sql_claim_skips_legacy_sherpa_speaker_job_when_speakrs_model_locked`.
6. **Disk estimator** — the measured anchor scales linearly across the ladder; the 2s default
   returns ~400 MB/day.
7. **Gate predicates** — unwritable path and insufficient disk both block; everything else
   passes.
8. **Re-entry** — a deliberately disabled feature stays disabled after re-resolving.
9. **Enrollment embedder (Rust)** — fixture-driven over real audio files, following the
   `scripts/diarization_bench` pattern of driving the shipped speaker provider without a
   database: a clean single-speaker clip accepts; a two-speaker clip rejects as
   `MultipleSpeakers`; a sub-minimum clip rejects as `TooShort`; silence rejects as `NoSpeech`.
10. **Retention correction (Rust, store-level)** — an `is_deliberate` voiceprint survives
    cleanup of the recording that produced it; a cluster-derived one still does not. Extends the
    existing `capture_retention.rs` tests.
11. **Enrollment storage** — writing a voiceprint sets the account-owner flag on exactly one
    profile and flips `recognize_saved_people` on.
12. **Owner-only auto-linking** — a `High` suggestion for the owner auto-links; a `Medium` one
    for the owner does not; a `High` one for a non-owner does not; an ambiguous or
    previously-rejected match never auto-links even for the owner; an auto-link **never adds an
    embedding**; the setting off reverts to suggest-and-confirm; and rejecting an auto-link
    prevents recurrence via the existing rejection guard.

**Not tested:** Tauri command wiring, Svelte reactivity, the bookend animations, and the
**bounded microphone recorder** — hardware-dependent by nature, and kept thin precisely so
nothing important lives in it.

## Slices

1. **Setup resolver + disk estimator** — Goal: pure modules producing resolved settings and the
   ordered work-list, and MB/day from the measured anchor. Areas: new `lib/onboarding/`
   modules. Acceptance: tests 1 and 6, including the Deepgram-never case. Depends on: none.
   Parallel: yes.
2. **Feature dependency module** — Goal: one `applyToggle` with two-way cascades, replacing
   `featureLockReason` and the controller's toggle switch. Acceptance: test 2. Depends on: none.
   Parallel: yes.
3. **Readiness classifier + progress reducer** — Goal: four-state classification and one
   byte-weighted aggregate. Acceptance: tests 3 and 4. Depends on: slice 1's work-list type.
   Parallel: yes, after that type exists.
4. **Claim-time model gating (Rust)** — Goal: migration renaming the lock table with a `reason`
   column; downloads take/release a `'downloading'` lock; dashboard derives Preparing.
   Areas: `crates/app-infra/src/processing/store.rs`, migrations, the four download commands.
   Acceptance: test 5. Depends on: none. Parallel: yes — **this is the only Rust slice and the
   only one that fixes a bug that exists today; it can ship first, alone.**
5. **Onboarding shell** — Goal: step machine, resume-at-Permissions persistence, atomic commit,
   re-entry precedence, the two hard gates. Acceptance: tests 7 and 8. Depends on: 1, 2.
6. **Permissions screen** — Goal: one-at-a-time requests, system-audio intent + Request again,
   denial recovery, relaunch offer. Depends on: 5. Parallel: yes, with 7–9.
7. **Capture & Storage screen** — Goal: rate slider, storage location, retention, excluded-apps
   summary, both hard gates rendered with real error copy. Depends on: 1, 5.
8. **Your settings + Change settings** — Goal: read-only manifest at ≤7 content lines, and the
   dense round-trip editor including AI setup. Depends on: 1, 2, 5.
9. **Setup screen** — Goal: non-blocking downloads, per-item states, real errors, cancel
   semantics, free-disk preflight. Depends on: 3, 4.
10. **Welcome + Finale** — Goal: bookend motion, capture start, all four Finale states.
    Depends on: 5.
11. **Retention correction + profile deletion (Rust)** — Goal: `is_deliberate` column, set on
    the audio-drawer naming path, excluded from the sweep; verify `delete_person_profile`
    cascades voiceprints. Areas: `capture_retention.rs`, migrations, `processing/store.rs`.
    Acceptance: test 10. Depends on: none. **Parallel: yes — pure bug fix, ships alone.**
12. **Enrollment embedder (Rust)** — Goal: audio path → voiceprint or typed rejection, over the
    existing speakrs entry point. Acceptance: test 9, fixture-driven. Depends on: none.
    Parallel: yes.
13. **Bounded recorder + live-session guard (Rust/Tauri)** — Goal: fixed-length mic clip to a
    temp file; pause/resume the capture session around it. Areas: `native_capture/lifecycle.rs`
    as the owning seam, thin command adapter. Depends on: none. Parallel: yes.
14. **Enrollment storage** — Goal: account-owner flag migration, write voiceprint with
    `source_cluster_id = NULL` and `is_deliberate = 1`, flip `recognize_saved_people`.
    Acceptance: test 11. Depends on: 11, 12.
14b. **Owner-only auto-linking** — Goal: a `High` owner suggestion auto-links via
    `link_speaker_cluster_to_person(.., add_embedding: false)`; auto vs. human-confirmed is
    recorded and surfaced; the on/off setting. Areas: `processing/store.rs`, the speaker
    settings type, audio-drawer UI. Acceptance: test 12. Depends on: 14. **Thresholds in
    `speaker-analysis` are not touched.**
15. **Voice screen** — Goal: both states, supplied sentence, level meter, playback confirm, all
    three rejections with retry, skip as a first-class outcome. Depends on: 5, 13, 14.
16. **Settings: enrollment surface** — Goal: re-enroll entry point, delete voiceprint/profile
    (first caller for `delete_person_profile`), and a read-out of recognition state and whether
    a voiceprint exists. Depends on: 13, 14.
17. **Delete the accordion** — Goal: remove `FeatureStack`, `FeatureRow`, `feature-model.ts`,
    the 12 `*Body.svelte` files not reused, and `onboarding-attention.ts`'s attention
    predicates. Depends on: 6–10, 15. Parallel: no — last.

Parallel groups: `[1, 2, 4, 11, 12, 13]` · `[3, 14]` · `[5]` · `[6, 7, 8, 9, 10, 15, 16]` ·
`[17]`.

Slices 4, 11, and 12 have no dependencies and each fix or unlock something independent of the
UI work — 4 and 11 are live bugs.

## Out of Scope

- **Enrolling anyone other than the account owner.** Naming other people stays the existing
  retroactive Speaker Cluster flow. Capturing a third party's voiceprint through a guided
  prompt raises consent obligations this work does not address.
- **Guaranteeing cross-channel recognition.** A voiceprint enrolled from a close, clean
  microphone matched against clusters derived from compressed system audio is a harder problem
  and the repo has no measurement for it. Ship enrollment, measure that case afterwards.
- **Changing recognition thresholds.** The 0.60 / 0.72 / 0.05 / 0.80 constants are consumed as
  they are, never retuned.
- **Auto-promoting suggestions for anyone other than the account owner.** Everyone else stays
  strictly suggest-and-confirm; naming other people remains the retroactive cluster flow.
- Unifying the four per-subsystem model downloaders.
- Resumable downloads (no downloader sends a range request today).
- Metered-connection deferral of optional downloads — macOS gives no reliable signal, and
  *Change settings* already lets the user turn Semantic Search off.
- Cloud transcription in any automatic path.
- Changing diarization or embedding models, recognition thresholds, or the Semantic Search
  index format.
- Retention windows longer than 30 days (a settings-model change; real gap, not this work).
- Windows and Linux onboarding.

## Further Notes

- **Two live bugs are in here and both ship independently.** Slice 4 (missing-model job burn)
  and slice 11 (retention deleting deliberately-named voiceprints) each fix something broken
  today, have no dependencies, and need none of the UI work.
- **Owner-only auto-linking is what makes enrollment feel like it did something.** Without it,
  every new recording re-asks and a confirmed identity never carries forward — the known
  "recognition doesn't stick" complaint. With it, the owner's turns get named on their own at
  `High` confidence while everyone else stays human-confirmed. The Voice screen may now promise
  that, but should still say recognition will not catch every turn (story 29) — `Medium`
  matches still ask, and matches below 0.60 never surface.
- **The `add_embedding: false` rule on auto-links is the load-bearing safety constraint.** It is
  what stops a wrong auto-link from writing a contaminated voiceprint that makes the next wrong
  match more likely. Treat it as non-negotiable in review.
- **`RecognitionConfidence::Low` is declared but never produced** — `best_enrollment_match` only
  emits `High` or `Medium` (anything under 0.60 returns `None`). Harmless, but don't write code
  that expects `Low` to appear.
- **Slice 4 fixes a live bug and should ship independently of the rest.** Downloading or
  deleting a model from Settings while recording currently burns every job that lands in the
  window — dead in about six minutes, with the audio kept and the transcript lost forever.
  Onboarding merely makes it easy to hit.
- **Semantic Search is 550 MB of the 729 MB total.** It stays in the resolved settings because
  the measured retrieval difference is large (FTS 0.45 vs hybrid 0.76 nDCG) and the CPU/RSS
  objection died with the migration from `ort` to candle-Metal (ADR 0037). If the download
  total ever becomes the complaint, deferring Semantic Search to first use cuts it to 179 MB
  and is the single highest-leverage change available.
- **The storage anchor is n=1** — one machine, one set of habits, pause-on-inactivity on. Worth
  re-measuring on a second machine before the figure ships in a release.
- **Unrelated finding:** a 1.3 GB `db/corrupt-2026-07-27/` directory is left behind in the data
  folder, larger than the live database, with nothing cleaning it up. Worth its own ticket.
