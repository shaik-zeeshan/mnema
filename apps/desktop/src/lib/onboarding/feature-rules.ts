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
 * Chain order: the order the rows are drawn in (a parent always above its
 * children) and therefore the order a cascade is reported in. `privacy` has no
 * row on *Change settings* and sits last.
 */
export const FEATURE_ORDER: readonly FeatureId[] = [
  "screen",
  "ocr",
  "microphone",
  "systemAudio",
  "transcription",
  "speakerSeparation",
  "semanticSearch",
  "aiFeatures",
  "privacy",
];

/** Row names, so a fix button and an announced sentence use the same words. */
export const FEATURE_LABELS: Record<FeatureId, string> = {
  screen: "Screen capture",
  ocr: "Read on-screen text",
  microphone: "Microphone",
  systemAudio: "System audio",
  transcription: "Transcription",
  speakerSeparation: "Who's speaking",
  semanticSearch: "Semantic Search",
  aiFeatures: "AI features",
  privacy: "Privacy exclusions",
};

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
    case "transcription":
      // Without this case the enable is a SILENT no-op: `applyToggle` sets it
      // and `normalizeFeatures` immediately unsets it, so the switch bounces
      // with no explanation. The rule was already true — this makes it speakable.
      return state.microphone || state.systemAudio ? null : "Needs an audio source on";
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

/** What is actually behind the AI row, for the one row whose copy isn't in `FeatureState`. */
export interface AiRowState {
  /** A provider AND a default model are really configured. */
  configured: boolean;
  /** The real reason AI is not ready, when there is one (`ai.aiConfigMissing`). */
  note?: string | null;
}

/**
 * The line under a row's name on *Change settings*.
 *
 * It lives here, next to `FEATURE_LABELS`, because every branch has to be read
 * off the SAME state the switch renders — a row describing a state it is not in
 * is the bug this screen exists to remove. Two branches earned their keep:
 * microphone is ON by default even when the grant is missing (`resolveSetup`),
 * so the ungranted copy must say "on, recording nothing" rather than read like a
 * lock; and transcription may be off with two audio sources above it, so "needs
 * an audio source" is only printed when `featureLockReason` says it is true.
 */
export function featureNote(state: FeatureState, id: FeatureId, ai?: AiRowState): string {
  switch (id) {
    case "screen":
      return state.permissions.screen
        ? "Frames of your screen. Everything else is built on these."
        : "Screen Recording is not granted — stays listed, records nothing.";
    case "microphone":
      if (state.permissions.microphone) {
        return "Your voice, from the built-in or a connected mic.";
      }
      return state.microphone
        ? "Microphone permission is not granted — stays on, records nothing."
        : "Microphone permission is not granted — grant it to turn this on.";
    case "systemAudio":
      return (
        "What your Mac plays. Excludes Mnema itself and every privacy-listed app." +
        (systemAudioNeedsRequest(state) ? " macOS can't confirm this grant." : "")
      );
    case "ocr":
      return state.screen
        ? "On-device. Apple Vision needs no download."
        : "Nothing to read while screen capture is off.";
    case "transcription":
      if (state.transcription) return "Runs locally on Whisper base.";
      return featureLockReason(state, "transcription")
        ? "Needs an audio source above it."
        : "Off — the audio is still recorded, just never turned into text.";
    case "speakerSeparation":
      return state.speakerSeparation
        ? "Splits a conversation into voices, on-device."
        : "A transcript is what it splits.";
    case "semanticSearch":
      return "Finds by meaning, not by word. The biggest single download.";
    case "aiFeatures":
      if (ai?.configured) return "Ready — a default model is configured.";
      return state.aiFeatures
        ? (ai?.note ?? "On, with nothing connected to answer with.")
        : "Never pre-ticked. Connecting a provider is what turns it on.";
    default:
      return "";
  }
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

/**
 * The rows that moved for a reason other than the one the user touched, in
 * chain order. `applyToggle` never mutates its input, so this is just a diff.
 */
export function cascadeOf(
  before: FeatureState,
  after: FeatureState,
  touched: FeatureId,
): FeatureId[] {
  return FEATURE_ORDER.filter((id) => id !== touched && before[id] !== after[id]);
}

/** What flipping `id` WOULD do, without doing it. */
export interface TogglePreview {
  /** The state the flip would produce (`=== state` when it is refused). */
  after: FeatureState;
  /** What `id` becomes — unchanged when `noop`. */
  next: boolean;
  /** True when the flip is refused: a locked enable. */
  noop: boolean;
  /** Why it is refused, or null. Same string `featureLockReason` reports. */
  lockReason: string | null;
  /** Rows other than `id` that would move, in chain order. Read `after[row]`
   *  for the value each one lands on. */
  cascade: FeatureId[];
}

/** `applyToggle` plus the diff, so a cascade can be shown before it commits. */
export function preview(state: FeatureState, id: FeatureId): TogglePreview {
  const after = applyToggle(state, id);
  const noop = after === state;
  return {
    after,
    next: after[id],
    noop,
    lockReason: noop ? featureLockReason(state, id) : null,
    cascade: cascadeOf(state, after, id),
  };
}

/** The action that clears a lock — never a dead control. */
export interface LockFix {
  /** `grant` — an OS permission no row can resolve. `toggle` — flip `id`. */
  act: "grant" | "toggle";
  id: FeatureId;
  label: string;
}

/**
 * Which row a lock hangs off. Microphone's lock is an OS grant, not a row.
 * Transcription points at system audio because that is the one audio source
 * that can never lock (ADR 0052), so the walk always ends somewhere flippable.
 */
const LOCK_PARENT: Partial<Record<FeatureId, FeatureId>> = {
  speakerSeparation: "transcription",
  transcription: "systemAudio",
};

/**
 * The fix for a locked row, or null when the row is not locked. Walks up to the
 * first ancestor that can actually be flipped, so the button is never a dead
 * end — *Who's speaking* with no audio at all offers "Turn System audio on",
 * not "Turn Transcription on", which would itself no-op.
 */
export function lockFix(state: FeatureState, id: FeatureId): LockFix | null {
  if (!featureToggleDisabled(state, id)) return null;
  if (id === "microphone") {
    return { act: "grant", id, label: `Grant ${FEATURE_LABELS.microphone}` };
  }
  const parent = LOCK_PARENT[id];
  if (!parent) return null;
  if (featureToggleDisabled(state, parent)) return lockFix(state, parent);
  return { act: "toggle", id: parent, label: `Turn ${FEATURE_LABELS[parent]} on` };
}
