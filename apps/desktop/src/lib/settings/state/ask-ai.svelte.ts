// Ask AI availability state: the `ask_ai_availability` snapshot (interactive
// opt-in + shared-engine prerequisite) surfaced in the AI settings panel.

import { invoke } from "@tauri-apps/api/core";
import { errorText } from "./format";

// Mirrors the Rust `ask_ai_availability` shape.
export type AskAiAvailability = {
  available: boolean;
  reason?: string | null;
};

// Friendly copy for an Ask AI availability reason code. Covers the Ask AI gate
// (ask_ai_disabled) plus the shared engine-configured prerequisite codes; the
// fallback delegates to the AI-runtime reason labeller passed in (those reasons
// reference connected provider labels, which live in the AI-runtime store).
export function askAiReasonLabel(
  reason: string | null | undefined,
  aiRuntimeReasonLabel: (r: string | null | undefined) => string,
): string {
  if (!reason) return "Ask AI is unavailable.";
  switch (reason) {
    case "ask_ai_disabled":
      return "Ask AI is turned off.";
    case "ai_runtime_disabled":
      return "AI features are turned off — enable them in the Providers card above.";
    case "no_providers":
      return "Connect an AI provider in the Providers card above.";
    case "no_default_model":
      return "Choose a global default model in the Providers card above.";
    default:
      return aiRuntimeReasonLabel(reason);
  }
}

export function createAskAiStore() {
  let askAiAvailability = $state<AskAiAvailability | null>(null);
  let askAiAvailabilityLoading = $state(false);
  let askAiAvailabilityError = $state<string | null>(null);

  const askAiAvailable = $derived(askAiAvailability?.available === true);

  async function loadAskAiAvailability() {
    askAiAvailabilityLoading = true;
    askAiAvailabilityError = null;
    try {
      askAiAvailability = await invoke<AskAiAvailability>("ask_ai_availability");
    } catch (err) {
      askAiAvailability = { available: false, reason: errorText(err) };
      askAiAvailabilityError = errorText(err);
    } finally {
      askAiAvailabilityLoading = false;
    }
  }

  return {
    get askAiAvailability() { return askAiAvailability; },
    get askAiAvailabilityLoading() { return askAiAvailabilityLoading; },
    get askAiAvailabilityError() { return askAiAvailabilityError; },
    get askAiAvailable() { return askAiAvailable; },
    loadAskAiAvailability,
  };
}

export type AskAiStore = ReturnType<typeof createAskAiStore>;
