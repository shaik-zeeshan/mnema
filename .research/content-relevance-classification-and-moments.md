# Content Relevance Classification and Moment Triggers

Date: 2026-05-07  
Status: research / design notes

## Context

This discussion started from whether Mnema should add VAD for system audio. The conclusion was that system-audio VAD is the wrong default primitive for memory relevance:

- Microphone activity is speech-first, so VAD is appropriate.
- System audio can be relevant even when it is not speech: videos, music production, movies, games, demos, lectures, etc.
- System audio can also be irrelevant even when loud: background music, notification sounds, idle media.

The better abstraction is **Content Relevance Classification**: keep capture conservative, then identify and surface meaningful spans as memory objects.

## Current codebase observations

- Microphone VAD already exists through `crates/capture-vad` and microphone PCM feed work in `crates/capture-microphone`.
- System audio activity currently uses peak-level audio activity in `crates/capture-screen` and inactivity policy code.
- `crates/app-infra` persists captured frames, OCR jobs, frame batches, and audio segments.
- There is no existing persisted model for active app, browser URL, window title, media metadata, context observations, or semantic moments.
- The **Recording Lifecycle** should remain focused on capture/pause/resume/rotation; relevance classification should live above capture.

## Core decisions

1. **Do not add system-audio VAD as the default relevance mechanism.**
   - Keep system audio capture/activity detection broad.
   - Use classification to decide memory relevance.

2. **Content relevance should initially control ranking/filtering and memory generation, not capture.**
   - Do not pause/drop capture based on early classifiers.
   - Avoid losing evidence before the relevance model is trusted.

3. **Classify semantic Moments, not raw capture segments.**
   - Existing recording segments are implementation/storage units.
   - A **Moment** is a user-meaningful span created by contextual triggers.

4. **Moment creation should be trigger-driven.**
   Boundary signals include:
   - active app identity change
   - browser URL/domain change
   - meaningful window/document title change
   - media item/title/source change
   - large visual/OCR topic change as fallback
   - long stability/silence timeout

5. **Custom Moment triggers should be declarative rules.**
   - No arbitrary user/plugin code at first.
   - Rules should match known observation fields.
   - This is safer, testable, and UI-friendly.

6. **Architecture should separate observation collection from rule evaluation.**
   - Native/context collector emits observations.
   - `app-infra` persists observations and evaluates rules into Moments.
   - Frontend provides rule builder and Moment timeline/search UI.

7. **Persist context locally with privacy controls.**
   Suggested defaults:
   - app bundle/name: raw
   - browser origin/domain: raw by default
   - full URL: optional
   - window/page/media title: raw by default, user-controllable
   - private/incognito windows: store nothing or only app identity
   - per-app exclusions

8. **Only one canonical Moment is active at a time.**
   - Other rule matches become annotations/labels on the active Moment.

9. **Multiple matching triggers are resolved by priority.**
   - Highest-priority matching rule determines the primary Moment kind.
   - Other matched rules become annotations.

10. **A Moment ends when its primary rule stops matching.**
    - Wait ~3 seconds before ending to avoid flicker.
    - If the same primary rule resumes within ~10 seconds and no different primary Moment started, merge/resume the prior Moment.

## Domain terms

### Context Observation

A timestamped observation about the user's current context. Examples:

- active app bundle id/name
- window title
- browser URL/origin/title
- media title/artist/source/playback state
- OCR/topic signals
- audio presence/speech flags

### Moment

A canonical, non-overlapping semantic span in history. A Moment has:

- start time
- optional end time
- primary kind
- primary trigger rule
- title/summary metadata
- linked observations/captured frames/audio segments

### Trigger Rule

A declarative user/system rule that matches Context Observations and can start or label a Moment.

Example:

```json
{
  "name": "YouTube learning video",
  "enabled": true,
  "priority": 100,
  "when": {
    "all": [
      { "field": "active_app.bundle_id", "op": "in", "value": ["com.apple.Safari", "com.google.Chrome"] },
      { "field": "browser.url.host", "op": "in", "value": ["youtube.com", "www.youtube.com"] },
      { "field": "media.playing", "op": "eq", "value": true }
    ]
  },
  "then": {
    "moment_kind": "video_learning",
    "labels": ["youtube", "video"]
  },
  "end_grace_ms": 3000,
  "merge_gap_ms": 10000
}
```

### Annotation

Metadata attached to a Moment without creating another overlapping Moment. Examples:

- labels: `youtube`, `learning`, `rust`
- matched rule ids
- confidence scores
- signal summaries: system audio present, OCR text present, media playing
- user notes
- future AI summaries/transcript snippets

## Data model sketch

Possible new tables in `crates/app-infra/migrations`:

### `context_observations`

Stores timestamped context samples.

Potential fields:

- `id`
- `observed_at`
- `source_kind`
- `active_app_bundle_id`
- `active_app_name`
- `window_title`
- `browser_origin`
- `browser_url` nullable / optional based on privacy settings
- `media_title`
- `media_artist`
- `media_source`
- `media_playing`
- `payload_json`
- `created_at`

### `moment_trigger_rules`

Stores user/system declarative rules.

Potential fields:

- `id`
- `name`
- `enabled`
- `priority`
- `condition_json`
- `action_json`
- `end_grace_ms`
- `merge_gap_ms`
- `created_at`
- `updated_at`

### `moments`

Stores canonical semantic spans.

Potential fields:

- `id`
- `started_at`
- `ended_at`
- `status` (`active`, `closed`)
- `primary_kind`
- `primary_rule_id`
- `title`
- `confidence`
- `created_at`
- `updated_at`

### `moment_annotations`

Stores labels and extra facts attached to Moments.

Potential fields:

- `id`
- `moment_id`
- `kind`
- `key`
- `value_json`
- `source_rule_id`
- `created_at`

## Evaluation loop sketch

1. Native/context collector emits a Context Observation.
2. `app-infra` persists the observation.
3. Rule engine evaluates enabled rules against the latest observation/context window.
4. Highest-priority matching rule becomes the primary candidate.
5. If no active Moment exists, start one.
6. If the active Moment's primary rule still matches, extend/update it and attach annotations.
7. If it stops matching, start a 3-second pending-close grace period.
8. If it resumes during grace, keep the Moment open.
9. If it remains unmatched, close the Moment.
10. If the same primary rule resumes within 10 seconds and no different primary Moment started, merge/resume the prior Moment.

## Example Moment flow

- User opens Safari on YouTube.
- Observation: Safari + youtube.com + media playing.
- Rule `YouTube learning video` matches.
- Moment starts: `video_learning`.
- Title updates from page/media title.
- OCR and audio presence are attached as annotations.
- User switches to editor.
- YouTube rule stops matching.
- After 3 seconds, Moment closes.
- User returns to same video within 10 seconds.
- Moment resumes/merges instead of creating a fragmented new Moment.

## Suggested implementation phases

1. **Domain + storage**
   - Add Context Observation, Moment Trigger Rule, Moment, and Moment Annotation types/tables in `crates/app-infra`.

2. **Basic context collector**
   - Active app identity.
   - Window title where permissions allow.

3. **Rule engine**
   - Deterministic declarative evaluator.
   - Priority resolution.
   - Grace/merge behavior.

4. **Moment generation worker**
   - Persist observations and update Moments outside Recording Lifecycle.

5. **Frontend read model**
   - Show Moments on timeline.
   - Filter/search by kind/label/app/title.

6. **Custom rule UI**
   - Rule builder for app/url/title/media fields.

7. **Enrichment**
   - Browser URL adapters.
   - Media metadata adapters.
   - OCR/topic-change fallback.
   - Later: transcription/audio classification/summaries.

## Open questions

- Exact macOS APIs/adapters for browser URL and media metadata.
- How to detect private/incognito reliably per browser.
- Whether full URL storage should be opt-in or prompted per browser.
- Moment title derivation rules.
- Whether Moment generation uses existing processing jobs or a dedicated app-infra background loop.
- How to link Moments to Captured Frames and Audio Segments efficiently.
- When, if ever, classification should affect retention/compression/capture control.
