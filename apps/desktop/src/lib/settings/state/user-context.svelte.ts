// User Context (derivation) state: the read-only status surface, run-now, and
// the destructive Wipe action. Owns its own non-draft reactive state; the
// `draft*` user-context toggles (enabled, budget tier, backfill) stay local in
// the page because they bind in markup and autosave through the engine.

import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import type {
  UserContextDerivationRunResult,
  UserContextDistillationSummary,
  UserContextStatus,
} from "$lib/types";
import { errorText } from "./format";

// ── Pure label/format helpers ───────────────────────────────────────────────

export function formatLastDerived(ms: number | null | undefined): string {
  if (!ms) return "never";
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
  });
}

// Plain-language line for what the last distillation pass withheld, so a thin
// dossier is explainable and not a silent no-op. Empty string when nothing was
// withheld.
export function distillationWithheldLine(
  summary: UserContextDistillationSummary | null | undefined,
): string {
  if (!summary) return "";
  const reasons: string[] = [];
  if (summary.guardrailSuppressed > 0)
    reasons.push(`${summary.guardrailSuppressed} by the privacy guardrail`);
  if (summary.belowFormationBar > 0)
    reasons.push(`${summary.belowFormationBar} needing more evidence`);
  if (summary.resurfaceBlocked > 0)
    reasons.push(`${summary.resurfaceBlocked} honoring a dismissal`);
  if (summary.ungrounded > 0)
    reasons.push(`${summary.ungrounded} without grounding`);
  if (reasons.length === 0) return "";
  const total =
    summary.guardrailSuppressed +
    summary.belowFormationBar +
    summary.resurfaceBlocked +
    summary.ungrounded;
  return `Last distillation held back ${total} draft ${
    total === 1 ? "conclusion" : "conclusions"
  }: ${reasons.join(", ")}.`;
}

// `onWiped` lets the page refresh adjacent surfaces (AI runtime status) after a
// wipe without this module reaching into the AI store directly.
export function createUserContextStore(opts: { onWiped?: () => void } = {}) {
  let userContextStatus = $state<UserContextStatus | null>(null);
  let userContextStatusError = $state<string | null>(null);
  let userContextRunNowRunning = $state(false);
  let userContextRunNowMessage = $state<string | null>(null);
  let userContextWiping = $state(false);

  async function loadUserContextStatus() {
    try {
      userContextStatus = await invoke<UserContextStatus>("get_user_context_status");
      userContextStatusError = null;
    } catch (err) {
      userContextStatusError = errorText(err);
    }
  }

  async function refreshUserContext() {
    await loadUserContextStatus();
  }

  async function runUserContextDerivationNow() {
    userContextRunNowRunning = true;
    userContextRunNowMessage = null;
    try {
      const result = await invoke<UserContextDerivationRunResult>(
        "user_context_run_derivation_now",
      );
      userContextRunNowMessage = result.message;
      await refreshUserContext();
    } catch (err) {
      userContextRunNowMessage = errorText(err);
    } finally {
      userContextRunNowRunning = false;
    }
  }

  // The explicit, full clear of the derived dossier (ADR 0029). Clears all
  // derived understanding, keeps raw captures + settings, and turns the master
  // "AI features" switch off (the recording_settings_changed event flips the
  // toggle for us). Disk-destructive of derived data, so it goes behind a
  // confirm dialog.
  async function wipeUserContext() {
    if (userContextWiping) return;
    const confirmed = await confirm(
      "This permanently clears everything Mnema has derived about you — all Activities, Conclusions, and your Dismissal corrections. Your raw recordings and settings are kept. AI features will be turned off; turning them back on starts learning from scratch.",
      { title: "Wipe User Context?", kind: "warning", okLabel: "Wipe", cancelLabel: "Cancel" },
    );
    if (!confirmed) return;
    userContextWiping = true;
    try {
      await invoke("wipe_user_context");
      // The surface is now empty; the recording_settings_changed event turns the
      // master AI switch off on its own, but refresh status here immediately so
      // the card reflects the wipe at once.
      await refreshUserContext();
      opts.onWiped?.();
    } catch (err) {
      userContextStatusError = errorText(err);
    } finally {
      userContextWiping = false;
    }
  }

  return {
    get userContextStatus() { return userContextStatus; },
    get userContextStatusError() { return userContextStatusError; },
    get userContextRunNowRunning() { return userContextRunNowRunning; },
    get userContextRunNowMessage() { return userContextRunNowMessage; },
    get userContextWiping() { return userContextWiping; },
    loadUserContextStatus,
    refreshUserContext,
    runUserContextDerivationNow,
    wipeUserContext,
  };
}

export type UserContextStore = ReturnType<typeof createUserContextStore>;
