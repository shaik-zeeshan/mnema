// The onboarding shell's step machine (issue #195, slice 5).
//
// Eight screens: Welcome → Permissions → Capture & Storage → Your settings ⇄
// Change settings → Setup → Voice → Finale. This class owns ONLY the flow:
// where we are, what the resolver produced, the two hard gates, and the atomic
// commit. Everything else — settings drafts, permissions, model stores, the AI
// store, privacy — stays on `OnboardingController`, which this composes rather
// than re-implements, and which every screen reaches through `flow.controller`.
//
// What is deliberately ABSENT: `canFinish`, `attentionCount`, and any notion of
// an "attention item". A downloading (or missing, or failed) model never blocks
// anything. The only gates in the whole flow are on Capture & Storage — see
// `$lib/onboarding/gates`.
import { invoke } from "@tauri-apps/api/core";
import {
  applyToggle,
  type FeatureId,
  type FeatureState,
  type PermissionIntents,
} from "$lib/onboarding/feature-rules";
import {
  CAPTURE_INTERVAL_LADDER_S,
  nearestLadderIndex,
} from "$lib/components/capture-rate";
import {
  captureStorageBlockReason,
  storageNeedBytes,
  type StorageProbe,
} from "$lib/onboarding/gates";
import {
  resolveSetup,
  workListBytes,
  type DownloadWorkItem,
  type ModelInventory,
  type ModelSelections,
  type ResolvedSettings,
  type SavedChoices,
} from "$lib/onboarding/resolve-setup";
import { OnboardingController } from "./onboarding.svelte";

export type { StorageProbe };

export type StepId =
  | "welcome"
  | "permissions"
  | "captureStorage"
  | "yourSettings"
  | "changeSettings"
  | "setup"
  | "voice"
  | "finale";

interface StepDef {
  id: StepId;
  /** Shown top-left in the window chrome. */
  label: string;
  /** Playhead position, 1-8. Change settings shares Your settings' position —
   *  it is a round trip, not forward progress (mockup frame 05). */
  position: number;
  /** Playhead suffix after "N / 8", e.g. "· round trip". */
  suffix: string;
  next: StepId | null;
  back: StepId | null;
}

// One flat table: no state-machine library, no router abstraction.
const STEPS: readonly StepDef[] = [
  { id: "welcome", label: "Mnema", position: 1, suffix: "", next: "permissions", back: null },
  { id: "permissions", label: "Permissions", position: 2, suffix: "", next: "captureStorage", back: "welcome" },
  { id: "captureStorage", label: "Capture & Storage", position: 3, suffix: "", next: "yourSettings", back: "permissions" },
  { id: "yourSettings", label: "Your settings", position: 4, suffix: "", next: "setup", back: "captureStorage" },
  { id: "changeSettings", label: "Change settings", position: 4, suffix: "· round trip", next: "yourSettings", back: "yourSettings" },
  { id: "setup", label: "Setup", position: 6, suffix: "", next: "voice", back: "yourSettings" },
  { id: "voice", label: "Voice", position: 7, suffix: "· optional", next: "finale", back: "setup" },
  { id: "finale", label: "Finale", position: 8, suffix: "", next: null, back: "voice" },
];

export const STEP_COUNT = 8;

/**
 * Resume marker. The step id is persisted, but the flow ALWAYS resumes at
 * Permissions: a relaunch only ever originates from granting Screen Recording,
 * so resuming there costs one Continue press and avoids serialising
 * half-finished settings. Onboarding itself stays gated on its completion
 * timestamp and does not re-run on launch.
 */
const RESUME_KEY = "mnema.onboarding.step";

function stepDef(id: StepId): StepDef {
  return STEPS.find((step) => step.id === id) ?? STEPS[0];
}

export class OnboardingFlow {
  /** Settings drafts, permissions, model stores, AI + privacy. Screens read it. */
  readonly controller = new OnboardingController();

  step = $state<StepId>(restoreStep());
  /** The resolver's output for this run. `null` until `load()` completes. */
  resolved = $state<ResolvedSettings | null>(null);
  /** Live feature enablements — seeded from `resolved`, edited by Change settings. */
  features = $state<FeatureState>(emptyFeatures());
  /** Measured by the Capture & Storage screen. `null` = not measured (never blocks). */
  storageProbe = $state<StorageProbe | null>(null);

  /** True once the user has explicitly saved recording settings before — the
   *  difference between a genuine first run and a re-entry. */
  private everSaved = $state(false);

  /** One-shot latch for `seedRecommendedExcludedApps` — see there. Plain, not
   *  `$state`: nothing renders off it. */
  private seededRecommendedApps = false;

  /** One-shot latch for pushing the resolved model picks into the drafts. */
  private seededModels = false;

  // ── Where we are ─────────────────────────────────────────────────────────
  get def(): StepDef {
    return stepDef(this.step);
  }
  stepLabel = $derived(this.def.label);
  stepPosition = $derived(this.def.position);
  stepSuffix = $derived(this.def.suffix);

  // ── The live model picture ───────────────────────────────────────────────
  // `resolved.models` is a one-shot: the resolver fills gaps at load and then
  // seeds the controller's drafts (see `resolve`). AFTER that the drafts are the
  // truth — *Change settings* writes straight into them — so everything that
  // prices or downloads a model reads THESE, never `resolved.models`. Reading
  // the frozen copy is what made the Setup screen fetch the default models
  // instead of the ones the user picked.
  models = $derived<ModelSelections>({
    ocrProvider: this.controller.draftOcrProvider,
    ocrModelId: this.controller.draftOcrModelId,
    transcriptionProvider: this.controller.draftTranscriptionProvider,
    transcriptionModelId: this.controller.draftTranscriptionModelId,
    speakerProvider: this.controller.draftSpeakerProvider,
    speakerModelId: this.controller.draftSpeakerModelId,
    semanticSearchModelId: this.controller.draftSemanticSearchModelId,
  });

  /** Live status for whichever model each subsystem currently has selected. */
  modelFacts = $derived<ModelInventory>({
    speakerAnalysis: facts(this.controller.selectedSpeakerModel, (m) => m.download?.byteSize),
    audioTranscription: facts(
      this.controller.selectedTranscriptionModel,
      (m) => m.download?.byteSize,
    ),
    // Semantic Search declares an APPROXIMATE size and has no `download` block.
    semanticSearch: facts(
      this.controller.selectedSemanticSearchModel,
      (m) => m.approxDownloadBytes,
    ),
  });

  /** The download agenda, rebuilt live off the current features + model picks. */
  workList = $derived<DownloadWorkItem[]>(
    this.resolved === null
      ? []
      : resolveSetup(this.features.permissions, this.modelFacts, {
          features: this.features,
          models: this.models,
          excludedApps: this.resolved.excludedApps,
        }).workList,
  );

  // ── The two hard gates (plus range validation), all on Capture & Storage ──
  downloadBytes = $derived(workListBytes(this.workList));
  captureIntervalSeconds = $derived(
    CAPTURE_INTERVAL_LADDER_S[nearestLadderIndex(this.controller.draftFrameRate)]!,
  );
  /** Everything the volume must hold: reserve + downloads + a day of capture. */
  requiredBytes = $derived(
    storageNeedBytes(this.downloadBytes, this.captureIntervalSeconds),
  );
  blockReason = $derived(
    this.step === "captureStorage"
      ? captureStorageBlockReason({
          probe: this.storageProbe,
          requiredBytes: this.downloadBytes,
          captureIntervalSeconds: this.captureIntervalSeconds,
          customResolutionErrors: this.controller.customResolutionErrors,
          customBitrateErrors: this.controller.customBitrateErrors,
        })
      : null,
  );
  /** Continue is live everywhere except a blocked Capture & Storage. Setup and
   *  Voice never disable it — a download in flight is progress, not a problem. */
  canContinue = $derived(this.blockReason === null);

  busy = $derived(
    this.controller.loading || this.controller.saving || this.controller.completing,
  );
  get errorMessage(): string | null {
    return this.controller.errorMessage;
  }

  // ── Navigation ───────────────────────────────────────────────────────────
  goTo(step: StepId): void {
    this.step = step;
    persistStep(step);
  }

  next(): void {
    if (!this.canContinue) return;
    // Permissions are the resolver's input, so re-resolve on the way out of the
    // Permissions screen — a grant made there must reach the settings.
    if (this.step === "permissions") this.resolve();
    const target = this.def.next;
    if (target) this.goTo(target);
  }

  back(): void {
    const target = this.def.back;
    if (target) this.goTo(target);
  }

  /**
   * Flip a feature and run the cascades. Returns the PRE-toggle state so a
   * caller can offer undo (assign it back to `flow.features`), or null when the
   * flip was refused — a locked enable. Existing callers ignore the value.
   */
  toggleFeature(id: FeatureId): FeatureState | null {
    const before = this.features;
    const after = applyToggle(before, id);
    if (after === before) return null;
    this.features = after;
    return before;
  }

  // ── Load + resolve ───────────────────────────────────────────────────────
  async load(): Promise<void> {
    // `get_onboarding_state` is also read inside `loadOnboarding` (which redirects
    // when onboarding is already complete); we read it for the one field that
    // decides first-run vs re-entry.
    try {
      const state = await invoke<{ recordingSettingsEverSaved: boolean }>(
        "get_onboarding_state",
      );
      this.everSaved = state.recordingSettingsEverSaved;
    } catch {
      // Non-fatal: `loadOnboarding` surfaces the real failure. Treat an
      // unreadable state as a first run — the resolver then fills every gap.
    }
    await this.controller.load();
    await this.controller.loadModelStatuses();
    this.resolve();
  }

  /**
   * Re-run the resolver against the CURRENT permissions and installed models.
   * SAVED SETTINGS WIN: on re-entry the user's persisted choices are passed in
   * as `SavedChoices`, so the resolver only fills gaps and never re-enables
   * something turned off deliberately.
   */
  resolve(): void {
    const c = this.controller;
    const permissions: PermissionIntents = {
      screen: c.permissions?.screen === "granted",
      microphone: c.permissions?.microphone === "granted",
      // System audio has no readable grant (ADR 0052) — INTENT only.
      systemAudio:
        c.sysAudioPromptRaised || c.permissions?.systemAudio === "assumed_working",
    };
    const resolved = resolveSetup(permissions, this.modelFacts, this.savedChoices());
    this.resolved = resolved;
    this.features = resolved.features;
    // Hand the resolved model picks to the drafts ONCE, then never again: from
    // here on the drafts are the truth and re-resolving (a return trip through
    // Permissions) must not undo a choice made on *Change settings*. Model
    // selection does not depend on permissions, so there is nothing to redo.
    if (!this.seededModels) {
      this.seededModels = true;
      applyModelsToDrafts(c, resolved.models);
    }
  }

  /** `null` on a genuine first run — every field is a gap for the resolver. */
  private savedChoices(): SavedChoices | null {
    if (!this.everSaved) return null;
    const c = this.controller;
    return {
      features: {
        screen: c.draftCaptureScreen,
        microphone: c.draftCaptureMicrophone,
        systemAudio: c.draftCaptureSystemAudio,
        ocr: c.draftOcrEnabled,
        transcription: c.draftTranscriptionEnabled,
        speakerSeparation: c.draftSpeakerSeparateSpeakers,
        semanticSearch: c.draftSemanticSearchEnabled,
        aiFeatures: c.draftAskAiEnabled,
        privacy: c.privacyEnabled,
      },
      models: {
        ocrProvider: c.draftOcrProvider,
        ocrModelId: c.draftOcrModelId,
        transcriptionProvider: c.draftTranscriptionProvider,
        transcriptionModelId: c.draftTranscriptionModelId,
        speakerProvider: c.draftSpeakerProvider,
        speakerModelId: c.draftSpeakerModelId,
        semanticSearchModelId: c.draftSemanticSearchModelId,
      },
      // Present (even empty) means the user has been here — the recommended
      // list is NOT re-applied over it.
      excludedApps: c.draftExcludedApps.map((app) => app.bundleId),
    };
  }

  /**
   * Turn the resolver's recommended privacy exclusions into REAL, strikeable
   * rules. `applyRecommendedExcludedApps` is first-run-only data, so this is a
   * ONE-SHOT for the whole run — *Capture & Storage* calls it on arrival (so
   * "Never recorded" shows what the app decided rather than an empty line) and
   * `finish()` calls it as the backstop.
   *
   * The latch is load-bearing, not defensive: `pendingRecommendedApps` filters
   * on `exclusionState !== "enabled"`, so an app the user has just STRUCK reads
   * as pending again. Without the latch, a second call would silently re-enable
   * every recommendation the user turned off.
   */
  async seedRecommendedExcludedApps(): Promise<void> {
    if (this.seededRecommendedApps) return;
    if (!this.resolved?.applyRecommendedExcludedApps) return;
    if (this.controller.appPrivacyExclusion.pendingRecommendedApps.length === 0) return;
    this.seededRecommendedApps = true;
    // ONLY through the privacy controller: its commands sync the privacy slice
    // alone. A full `syncDrafts` would clobber in-progress toggles
    // (`onboarding-privacy-sync.ts`).
    await this.controller.appPrivacyExclusion.applyAllRecommendedPrivacyApps();
  }

  // ── Atomic commit ────────────────────────────────────────────────────────
  /**
   * Commit everything in one shot and (optionally) start capture. Called from
   * the Finale. The whole recording config goes through the atomic
   * `update_recording_settings` command inside `finishOnboarding`, so a late
   * validation failure cannot leave a half-persisted configuration behind.
   */
  async finish(startRecording = true): Promise<void> {
    const c = this.controller;
    // Privacy first. Normally a no-op — *Capture & Storage* already seeded on
    // arrival — and the latch is what keeps this backstop from re-enabling a
    // recommendation the user struck there.
    await this.seedRecommendedExcludedApps();
    // Features only. The model drafts were seeded at `resolve()` and are the
    // user's since — re-applying `resolved.models` here would silently revert a
    // model chosen on *Change settings*.
    applyResolvedToDrafts(c, this.features);
    // `controller.finish` → `finishOnboarding` (onboarding-lifecycle.ts): one
    // atomic `update_recording_settings`, then `complete_onboarding`, then
    // `start_native_capture`.
    await c.finish(startRecording);
    clearStep();
  }
}

/** Live facts for one subsystem's selected model, in the resolver's shape. The
 *  four status shapes agree on id/name/available and disagree only on where the
 *  size lives, so `size` is the one thing a caller supplies. */
function facts<T extends { modelId: string | null; displayName: string; available: boolean }>(
  model: T | null | undefined,
  size: (model: T) => number | null | undefined,
): ModelInventory["speakerAnalysis"] {
  if (!model) return null;
  return {
    modelId: model.modelId,
    displayName: model.displayName,
    byteSize: size(model) ?? null,
    installed: model.available,
  };
}

/**
 * Fold the resolved feature set into the controller's draft fields, which
 * `buildSettingsRequestFrom` then serializes. The companion `transcribe*` /
 * `recognizeSavedPeople` flags are taken from `FeatureState`, where they are
 * derived — never set independently.
 */
function applyResolvedToDrafts(c: OnboardingController, f: FeatureState): void {
  c.draftCaptureScreen = f.screen;
  c.draftCaptureMicrophone = f.microphone;
  c.draftCaptureSystemAudio = f.systemAudio;
  c.draftOcrEnabled = f.ocr;
  c.draftTranscriptionEnabled = f.transcription;
  c.draftTranscriptionMicrophoneEnabled = f.transcribeMicrophone;
  c.draftTranscriptionSystemAudioEnabled = f.transcribeSystemAudio;
  c.draftSpeakerSeparateSpeakers = f.speakerSeparation;
  c.draftSpeakerRecognizeSavedPeople = f.recognizeSavedPeople;
  c.draftSemanticSearchEnabled = f.semanticSearch;
  c.draftAskAiEnabled = f.aiFeatures;
  c.privacyEnabled = f.privacy;
}

/**
 * Seed the model drafts from the resolver. Called ONCE per run, at the first
 * `resolve()`; after that *Change settings* owns these fields.
 */
function applyModelsToDrafts(c: OnboardingController, models: ModelSelections): void {
  // The `choose*` setters carry the provider→default-model coupling; the
  // explicit model id afterwards is the resolver's (or the saved) choice.
  c.chooseOcrProvider(models.ocrProvider);
  c.draftOcrModelId = models.ocrModelId;
  c.chooseTranscriptionProvider(models.transcriptionProvider);
  c.draftTranscriptionModelId = models.transcriptionModelId;
  c.draftSpeakerProvider = models.speakerProvider;
  c.draftSpeakerModelId = models.speakerModelId;
  c.draftSemanticSearchModelId = models.semanticSearchModelId;
}

function restoreStep(): StepId {
  // A persisted step means the user was mid-flow when the app relaunched (only
  // ever after granting Screen Recording), so resume at Permissions.
  try {
    const saved = localStorage.getItem(RESUME_KEY);
    return saved !== null && saved !== "welcome" ? "permissions" : "welcome";
  } catch {
    return "welcome";
  }
}

function persistStep(step: StepId): void {
  try {
    localStorage.setItem(RESUME_KEY, step);
  } catch {
    // Resume is a convenience; a storage failure must not break the flow.
  }
}

function clearStep(): void {
  try {
    localStorage.removeItem(RESUME_KEY);
  } catch {
    // as above
  }
}

/** Placeholder until `load()` resolves — everything off, nothing granted. */
function emptyFeatures(): FeatureState {
  return {
    permissions: { screen: false, microphone: false, systemAudio: false },
    screen: false,
    microphone: false,
    systemAudio: false,
    ocr: false,
    transcription: false,
    speakerSeparation: false,
    semanticSearch: false,
    aiFeatures: false,
    privacy: false,
    transcribeMicrophone: false,
    transcribeSystemAudio: false,
    recognizeSavedPeople: false,
  };
}
