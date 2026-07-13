// MCP-connector actions for the Settings controller — split out of
// controller.svelte.ts for the 800-line cap (createProcessingModelsView /
// createSemanticSearchView precedent). The controller re-exposes these as thin
// delegates so call sites stay `c.addMcpServerFromPreset(...)` etc.
//
// No runes here: these are plain actions over the injected stores.

import { confirm } from "@tauri-apps/plugin-dialog";
import { presetToDraft, type McpPreset } from "./mcp-presets";
import type { McpServerConfig } from "$lib/types";
import type { RecordingStore } from "./recording.svelte";
import type { AiRuntimeStore } from "./ai-runtime.svelte";

export interface McpConnectorActionDeps {
  rec: RecordingStore;
  aiRuntime: AiRuntimeStore;
  /** Direct (non-debounced) ai_runtime domain save — the controller's saveRecordingDomain("ai_runtime"). */
  saveAiRuntime: () => Promise<void>;
}

export function createMcpConnectorActions(deps: McpConnectorActionDeps) {
  const { rec, aiRuntime } = deps;

  // Push a fully-formed connector draft (the picker builds it — preset or the
  // Custom form). Prepended: the mockup flashes the new row in at the TOP of
  // the connector list. Returns the draft's id so the caller can save its
  // secret and verify the connection against it.
  function addMcpServerDraft(draft: McpServerConfig): string {
    rec.draftMcpServers = [draft, ...rec.draftMcpServers];
    void aiRuntime.refreshMcpServerSecretPresence();
    return draft.id;
  }

  // Pre-fill a draft from a catalog preset (Plan: MCP Connector Preset Picker,
  // slice 3). `overrides` carries the picker's ADVANCED edits and the
  // stdio-while-Node-missing `enabled: false` start.
  function addMcpServerFromPreset(
    preset: McpPreset,
    overrides?: Partial<McpServerConfig>,
  ): string {
    return addMcpServerDraft({
      ...presetToDraft(preset, rec.draftMcpServers),
      ...overrides,
    });
  }

  // `confirm: false` is the picker's verify-on-add ROLLBACK path: the user never
  // saw the connector land, so undoing the just-added draft (and its just-saved
  // secret) must not raise a "Remove this connector?" dialog.
  async function removeMcpServer(id: string, opts?: { confirm?: boolean }): Promise<void> {
    const removed = rec.draftMcpServers.find((s) => s.id === id);
    if (!removed) return;
    if (opts?.confirm !== false) {
      const label = removed.label.trim() || id;
      const confirmed = await confirm(
        `Removing “${label}” deletes its saved secret from the system keychain right away and stops offering its tools to chat.`,
        {
          title: "Remove this connector?",
          kind: "warning",
          okLabel: "Remove & Delete Secret",
          cancelLabel: "Keep Connector",
        },
      );
      if (!confirmed) return;
    }
    rec.draftMcpServers = rec.draftMcpServers.filter((s) => s.id !== id);
    // Tear down the keychain secret for the removed connector (best effort).
    await aiRuntime.clearSecretForRemovedMcpServer(id);
  }

  // Flush the ai_runtime autosave NOW. Verify-on-add lists a just-added
  // connector's tools, and `mcp_list_server_tools` reads the PERSISTED config —
  // the 450ms debounce would race it. Waits out an in-flight ai_runtime save
  // (whose snapshot pre-dates the new draft), saves directly, and throws when
  // the domain is still dirty afterwards (failed save / armed backoff), so the
  // caller surfaces the error instead of verifying against stale state.
  async function flushAiRuntimeSave(): Promise<void> {
    for (let waited = 0; rec.savingRecDomains.ai_runtime && waited < 5000; waited += 100) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    await deps.saveAiRuntime();
    const baseline = rec.lastSavedRecSnapshots.ai_runtime;
    if (baseline === null || rec.buildRecDomainSnapshot("ai_runtime") !== baseline) {
      throw new Error(rec.recError ?? "Could not save the connector settings — try again.");
    }
  }

  return {
    addMcpServerDraft,
    addMcpServerFromPreset,
    removeMcpServer,
    flushAiRuntimeSave,
  };
}

export type McpConnectorActions = ReturnType<typeof createMcpConnectorActions>;
