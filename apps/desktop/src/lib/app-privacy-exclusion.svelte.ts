import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type {
  ExcludedAppEntry,
  RecordingSettingsDomainUpdateResponse,
} from "$lib/types";
import {
  appIconFallback,
  canonicalBundleIdForComparison,
  filteredPrivacyAppCandidates,
  iconPathForBundleId,
  makePrivacyAppCandidate,
  mergeIconResolutions,
  pendingRecommendedApps,
  recommendationActionFor,
  recommendationActionLabel,
  recommendationBundleIds,
  shouldShowSensitiveRecommendationPrompt,
  unresolvedIconBundleIds,
  visibleBrowserDisclosureApps,
  type AppIconResolution,
  type AppPrivacyRecommendation,
  type PrivacyAppCandidate,
  type PrivacyAppCandidateDto,
  type SensitiveCaptureRecommendations,
} from "$lib/app-privacy-exclusion";
import { detectKeyboardPlatform } from "$lib/keyboard";

type AppPrivacyExclusionHost = {
  getExcludedApps: () => ExcludedAppEntry[];
  onSettingsUpdated: (response: RecordingSettingsDomainUpdateResponse) => void;
  setError: (message: string | null) => void;
  beforePrivacyCommand?: () => void;
  enableExistingUserPrompt?: boolean;
};

type AppPrivacyExclusionState = {
  candidates: PrivacyAppCandidate[];
  iconPathsByBundleId: Record<string, string>;
  recommendations: SensitiveCaptureRecommendations | null;
  comboboxQuery: string;
  comboboxOpen: boolean;
  commandInFlight: boolean;
  promptActionInFlight: boolean;
  promptMarkedId: string | null;
};

function serializeError(err: unknown): string {
  return humanizeError(err);
}

function makeDraftId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export function createAppPrivacyExclusionController(host: AppPrivacyExclusionHost) {
  const state = $state<AppPrivacyExclusionState>({
    candidates: [],
    iconPathsByBundleId: {},
    recommendations: null,
    comboboxQuery: "",
    comboboxOpen: false,
    commandInFlight: false,
    promptActionInFlight: false,
    promptMarkedId: null,
  });
  const requestedCanonicalIconBundleIds = new Set<string>();

  async function loadPrivacyAppCandidates(): Promise<void> {
    try {
      const candidates = await invoke<PrivacyAppCandidateDto[]>("list_privacy_app_candidates");
      requestedCanonicalIconBundleIds.clear();
      state.candidates = candidates.map((candidate) => (
        makePrivacyAppCandidate(candidate, makeDraftId("app-candidate"))
      ));
    } catch {
      state.candidates = [];
    }
  }

  async function loadSensitiveCaptureRecommendations(): Promise<void> {
    try {
      state.recommendations = await invoke<SensitiveCaptureRecommendations>("get_sensitive_capture_recommendations");
      void resolveAppIcons(recommendationBundleIds(state.recommendations));
    } catch {
      state.recommendations = null;
    }
  }

  async function resolveAppIcons(bundleIds: Array<string | null | undefined>): Promise<void> {
    const unresolvedBundleIds = unresolvedIconBundleIds(
      bundleIds,
      state.iconPathsByBundleId,
      requestedCanonicalIconBundleIds,
    );
    if (unresolvedBundleIds.length === 0) return;
    for (const bundleId of unresolvedBundleIds) {
      requestedCanonicalIconBundleIds.add(canonicalBundleIdForComparison(bundleId));
    }

    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: unresolvedBundleIds },
      });
      const result = mergeIconResolutions(state.iconPathsByBundleId, icons);
      if (!result.changed) return;
      state.iconPathsByBundleId = result.iconPathsByBundleId;
      state.candidates = state.candidates.map((candidate) => ({
        ...candidate,
        iconPath: iconPathForBundleId(candidate.bundleId, result.iconPathsByBundleId) ?? candidate.iconPath,
      }));
    } catch {
      for (const bundleId of unresolvedBundleIds) {
        requestedCanonicalIconBundleIds.delete(canonicalBundleIdForComparison(bundleId));
      }
      // App icons are decorative; app discovery must keep working if extraction fails.
    }
  }

  async function runPrivacySettingsCommand(
    command: "add_privacy_excluded_app" | "set_privacy_excluded_app_enabled" | "remove_privacy_excluded_app",
    args: Record<string, unknown>,
  ): Promise<RecordingSettingsDomainUpdateResponse | null> {
    host.beforePrivacyCommand?.();
    state.commandInFlight = true;
    host.setError(null);
    try {
      const updated = await invoke<RecordingSettingsDomainUpdateResponse>(command, args);
      host.onSettingsUpdated(updated);
      void loadSensitiveCaptureRecommendations();
      return updated;
    } catch (err) {
      host.setError(serializeError(err));
      return null;
    } finally {
      state.commandInFlight = false;
    }
  }

  function appIconPathForBundleId(bundleId: string): string | null {
    return iconPathForBundleId(bundleId, state.iconPathsByBundleId);
  }

  function appIconSrcForBundleId(bundleId: string): string | null {
    const iconPath = appIconPathForBundleId(bundleId);
    return iconPath ? convertFileSrc(iconPath) : null;
  }

  function privacyAppIconSrc(candidate: PrivacyAppCandidate): string | null {
    const iconPath = appIconPathForBundleId(candidate.bundleId) ?? candidate.iconPath;
    return iconPath ? convertFileSrc(iconPath) : null;
  }

  function addPrivacyAppCandidate(candidate: PrivacyAppCandidate | null): void {
    const action = candidate
      ? recommendationActionFor(
          { ...candidate, exclusionState: "missing" },
          host.getExcludedApps(),
        )
      : { kind: "none" as const, command: null, args: null };
    if (action.kind !== "add") return;
    void runPrivacySettingsCommand(action.command, action.args);
    state.comboboxQuery = "";
    state.comboboxOpen = false;
  }

  function applyRecommendation(app: AppPrivacyRecommendation): void {
    const action = recommendationActionFor(app, host.getExcludedApps());
    if (action.kind === "none") return;
    void runPrivacySettingsCommand(action.command, action.args);
  }

  function setPrivacyExcludedAppEnabled(sourceId: string, enabled: boolean): void {
    void runPrivacySettingsCommand("set_privacy_excluded_app_enabled", { sourceId, enabled });
  }

  function removePrivacyApp(sourceId: string): void {
    void runPrivacySettingsCommand("remove_privacy_excluded_app", { sourceId });
  }

  function handlePrivacyAppComboboxInput(): void {
    state.comboboxOpen = true;
  }

  function addSelectedPrivacyApp(): void {
    const candidate = state.comboboxQuery.trim() ? controller.filteredCandidates[0] : null;
    addPrivacyAppCandidate(candidate ?? null);
  }

  function handlePrivacyAppComboboxKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      addSelectedPrivacyApp();
      return;
    }
    if (event.key === "Escape") {
      state.comboboxOpen = false;
      return;
    }
    if (event.key === "ArrowDown") {
      state.comboboxOpen = true;
    }
  }

  function closePrivacyAppComboboxSoon(): void {
    if (typeof window === "undefined") {
      state.comboboxOpen = false;
      return;
    }
    window.setTimeout(() => {
      state.comboboxOpen = false;
    }, 120);
  }

  async function dismissSensitiveRecommendationPrompt(): Promise<void> {
    const promptId = state.recommendations?.promptId;
    if (!promptId || state.promptActionInFlight) return;
    state.promptActionInFlight = true;
    try {
      await invoke("dismiss_one_time_prompt", { promptId });
      if (state.recommendations) {
        state.recommendations = {
          ...state.recommendations,
          shouldShowExistingUserPrompt: false,
        };
      }
    } catch (err) {
      host.setError(serializeError(err));
    } finally {
      state.promptActionInFlight = false;
    }
  }

  async function applyAllRecommendedPrivacyApps(): Promise<void> {
    const promptId = state.recommendations?.promptId;
    const recommendations = [...pendingRecommendedApps(state.recommendations)];
    if (!promptId || recommendations.length === 0 || state.promptActionInFlight) return;

    host.beforePrivacyCommand?.();
    state.promptActionInFlight = true;
    state.commandInFlight = true;
    host.setError(null);
    try {
      for (const app of recommendations) {
        const action = recommendationActionFor(app, host.getExcludedApps());
        if (action.kind === "none") continue;
        const updated = await invoke<RecordingSettingsDomainUpdateResponse>(action.command, action.args);
        host.onSettingsUpdated(updated);
      }
      await invoke("complete_one_time_prompt", { promptId });
      if (state.recommendations) {
        state.recommendations = {
          ...state.recommendations,
          shouldShowExistingUserPrompt: false,
        };
      }
      await loadSensitiveCaptureRecommendations();
    } catch (err) {
      host.setError(serializeError(err));
    } finally {
      state.commandInFlight = false;
      state.promptActionInFlight = false;
    }
  }

  const controller = {
    get candidates(): PrivacyAppCandidate[] {
      return state.candidates;
    },
    get recommendations(): SensitiveCaptureRecommendations | null {
      return state.recommendations;
    },
    get comboboxQuery(): string {
      return state.comboboxQuery;
    },
    set comboboxQuery(value: string) {
      state.comboboxQuery = value;
    },
    get comboboxOpen(): boolean {
      return state.comboboxOpen;
    },
    set comboboxOpen(value: boolean) {
      state.comboboxOpen = value;
    },
    get commandInFlight(): boolean {
      return state.commandInFlight;
    },
    get promptActionInFlight(): boolean {
      return state.promptActionInFlight;
    },
    get excludedApps(): ExcludedAppEntry[] {
      return host.getExcludedApps();
    },
    get filteredCandidates(): PrivacyAppCandidate[] {
      return filteredPrivacyAppCandidates(
        state.candidates,
        host.getExcludedApps(),
        state.comboboxQuery,
      );
    },
    get pendingRecommendedApps() {
      return pendingRecommendedApps(state.recommendations);
    },
    get visibleBrowserDisclosureApps() {
      return visibleBrowserDisclosureApps(state.recommendations);
    },
    get showSensitiveRecommendationPrompt(): boolean {
      // App Privacy Exclusion is macOS-only (ADR 0025): Windows v1 has no live
      // privacy filter, so excluding an app does nothing. Never surface the
      // first-run recommended-exclusions prompt off macOS — gating here also
      // stops the `mark_one_time_prompt_shown` effect below from firing.
      if (detectKeyboardPlatform() !== "macos") return false;
      return shouldShowSensitiveRecommendationPrompt(state.recommendations);
    },
    loadPrivacyAppCandidates,
    loadSensitiveCaptureRecommendations,
    resolveAppIcons,
    appIconSrcForBundleId,
    privacyAppIconSrc,
    appIconFallback,
    recommendationActionLabel,
    addPrivacyAppCandidate,
    applyRecommendation,
    setPrivacyExcludedAppEnabled,
    removePrivacyApp,
    handlePrivacyAppComboboxInput,
    handlePrivacyAppComboboxKeydown,
    closePrivacyAppComboboxSoon,
    addSelectedPrivacyApp,
    dismissSensitiveRecommendationPrompt,
    applyAllRecommendedPrivacyApps,
  };

  $effect(() => {
    void resolveAppIcons(host.getExcludedApps().map((app) => app.bundleId));
  });

  $effect(() => {
    if (!state.comboboxOpen) return;
    void resolveAppIcons(controller.filteredCandidates.map((candidate) => candidate.bundleId));
  });

  // ponytail: candidate list is otherwise mount-only, so newly-seen apps never
  // surface until reload. Window focus is the cheap "user came back, refresh"
  // signal for this main-window-only surface; timeline_data_changed if capture-live freshness matters.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onFocus = () => {
      void loadPrivacyAppCandidates();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  $effect(() => {
    if (!host.enableExistingUserPrompt) return;
    const promptId = state.recommendations?.promptId;
    if (!promptId || !controller.showSensitiveRecommendationPrompt) return;
    if (state.promptMarkedId === promptId) return;
    state.promptMarkedId = promptId;
    void invoke("mark_one_time_prompt_shown", { promptId });
  });

  return controller;
}

export type AppPrivacyExclusionController = ReturnType<typeof createAppPrivacyExclusionController>;
