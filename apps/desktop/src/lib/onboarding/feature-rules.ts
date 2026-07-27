// Onboarding feature dependency rules (issue #195, slice 2).
//
// ONE pure function owns the whole feature graph: `applyToggle(state, id)`.
// It replaces `routes/onboarding/feature-model.ts:featureLockReason` and the
// hand-written toggle switch in `onboarding.svelte.ts:483`. No Svelte, no
// `invoke`, no side effects — testable under `bun test`.
//
// `FeatureState` is also the shape the resolver produces (see `resolve-setup.ts`)
// and the shape *Your settings* / *Change settings* render, so there is exactly
// one state object in the flow.
//
// Cascades run BOTH directions:
//   up   — enabling an audio source enables transcription and speaker separation
//   down — disabling the LAST audio source disables both again
// The downward half is expressed once, declaratively, in `normalizeFeatures`.

/** Every directly toggleable row on *Change settings*. */
export type FeatureId =
  | "screen"
  | "microphone"
  | "systemAudio"
  | "ocr"
  | "transcription"
  | "speakerSeparation"
  | "semanticSearch"
  | "aiFeatures"
  | "privacy";

/**
 * What we know about each OS permission at resolve/toggle time.
 *
 * `screen` and `microphone` are real, queryable grants. `systemAudio` can never
 * be queried (ADR 0052) — this field carries INTENT: true once the user has
 * asked for it on the Permissions screen. Intent is a ROW ANNOTATION, never a
 * lock (see `featureLockReason`).
 */
export interface PermissionIntents {
  screen: boolean;
  microphone: boolean;
  systemAudio: boolean;
}

/**
 * The resolved feature enablements, plus the permission context the lock rules
 * read. Companion flags (`transcribe*`, `recognizeSavedPeople`) are DERIVED —
 * never toggled directly — so they can't drift out of sync with their source.
 */
export interface FeatureState {
  permissions: PermissionIntents;
  screen: boolean;
  microphone: boolean;
  systemAudio: boolean;
  ocr: boolean;
  transcription: boolean;
  speakerSeparation: boolean;
  semanticSearch: boolean;
  /** Reasoning Engine. Never pre-ticked — configuring a provider enables it. */
  aiFeatures: boolean;
  /** Frontend-only flag; there is no backend `privacy.enabled` field. */
  privacy: boolean;
  // ── derived ──────────────────────────────────────────────────────────────
  transcribeMicrophone: boolean;
  transcribeSystemAudio: boolean;
  recognizeSavedPeople: boolean;
}

/**
 * Re-derive every companion flag and run the downward cascades. Idempotent.
 * Both `applyToggle` and the resolver funnel through it so a state produced by
 * either obeys the same invariants.
 */
export function normalizeFeatures(state: FeatureState): FeatureState {
  const hasAudioSource = state.microphone || state.systemAudio;
  // Transcription with no audio source is an orphan; speaker separation needs a
  // transcript to split.
  const transcription = state.transcription && hasAudioSource;
  const speakerSeparation = state.speakerSeparation && transcription;
  return {
    ...state,
    transcription,
    speakerSeparation,
    // The old controller only ever set these in lockstep with their source
    // (onboarding.svelte.ts:498-509, 520-528), so deriving them is
    // behaviour-identical and removes the drift that fed the old
    // `transcriptionRequestedWhileOff` attention rule.
    transcribeMicrophone: transcription && state.microphone,
    transcribeSystemAudio: transcription && state.systemAudio,
    recognizeSavedPeople: state.recognizeSavedPeople && speakerSeparation,
  };
}

/**
 * Why `id` cannot be turned ON yet, or null. Turning a feature OFF is always
 * permitted, so callers only consult this on the enable path.
 *
 * System audio can NEVER carry a lock — not on the screen permission, not on
 * screen liveness (ADR 0052), and not on its own intent bit. Its grant is
 * unreadable, so a lock would be a gate we can never open: a user who skipped
 * the Permissions screen, saw system audio on, and turned it off would be
 * unable to turn it back on. Intent renders as a row annotation instead —
 * see `systemAudioNeedsRequest`.
 */
export function featureLockReason(state: FeatureState, id: FeatureId): string | null {
  switch (id) {
    case "microphone":
      return state.permissions.microphone ? null : "Needs Microphone permission";
    case "speakerSeparation":
      return state.transcription ? null : "Needs Audio transcription on";
    default:
      return null;
  }
}

/**
 * True when system audio is on but the user has never raised the OS prompt, so
 * the row reads "macOS can't confirm the grant" and the button offers *Request*
 * (or *Request again* — a closed prompt is indistinguishable from a denial).
 * Never a green check, and never a gate on the toggle.
 */
export function systemAudioNeedsRequest(state: FeatureState): boolean {
  return state.systemAudio && !state.permissions.systemAudio;
}

/** A toggle is disabled only when the feature is OFF and its lock is unmet. */
export function featureToggleDisabled(state: FeatureState, id: FeatureId): boolean {
  return !state[id] && featureLockReason(state, id) !== null;
}

/**
 * Flip `id` and run the cascades in both directions. Returns a NEW state; the
 * input is never mutated. A locked enable is a no-op (the same state is
 * returned, referentially).
 */
export function applyToggle(state: FeatureState, id: FeatureId): FeatureState {
  const turningOn = !state[id];
  if (turningOn && featureLockReason(state, id) !== null) return state;

  const next: FeatureState = { ...state, [id]: turningOn };

  // Upward cascade: an audio source with no transcript is silent audio, and a
  // transcript with no speaker split is the thing users report as broken.
  if (turningOn && (id === "microphone" || id === "systemAudio")) {
    next.transcription = true;
    next.speakerSeparation = true;
  }
  // Upward cascade: enabling transcription with sources already on binds them
  // (handled by `normalizeFeatures`); enabling it also re-offers speaker
  // separation, but does not force it — the user turned it off deliberately.

  return normalizeFeatures(next);
}
