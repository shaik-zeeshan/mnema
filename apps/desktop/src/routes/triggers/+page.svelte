<script lang="ts">
  // /triggers — the Triggers management surface (issue #182; final design in
  // docs/triggers/mockups/final/DESIGN.md). This page owns the data (list +
  // status + provider gate) and swaps between the list and the 3-step wizard;
  // TriggersList / TriggerWizard are presentation.
  import { goto } from "$app/navigation";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import { openSettings } from "$lib/surface-windows";
  import TriggersList from "$lib/triggers/TriggersList.svelte";
  import TriggerWizard from "$lib/triggers/TriggerWizard.svelte";
  import {
    deleteTrigger,
    listTriggers,
    listTriggersStatus,
    runTriggerAgain,
    triggersProviderReady,
    updateTrigger,
    type ConditionType,
    type TriggerDefinition,
    type TriggerDraft,
    type TriggerStatus,
  } from "$lib/triggers/api";
  import { parseTriggerJson, shareTriggerJson } from "$lib/triggers/share";

  const REFRESH_INTERVAL_MS = 30_000;

  let triggers = $state<TriggerDefinition[]>([]);
  let statuses = $state<Map<string, TriggerStatus>>(new Map());
  let providerReady = $state(true);
  let loaded = $state(false);
  let loadError = $state<string | null>(null);
  let importError = $state<string | null>(null);
  let flashId = $state<string | null>(null);

  type View =
    | { mode: "list" }
    | {
        mode: "wizard";
        wizardMode: "create" | "edit" | "import";
        presetCond?: ConditionType;
        editing?: TriggerDefinition;
        imported?: TriggerDraft;
        /** Remount key so each wizard open re-initializes from its props. */
        nonce: number;
      };
  let view = $state<View>({ mode: "list" });
  let wizardNonce = 0;

  // Run Again in flight: trigger id → the failed firing's firedAtMs at click
  // time. A refresh showing a DIFFERENT last firing means the retry's ledger
  // row landed — the retry is over.
  let retrying = $state<Map<string, number>>(new Map());
  const retryingIds = $derived(new Set(retrying.keys()));

  async function refresh(): Promise<void> {
    try {
      const [defs, statusRows, ready] = await Promise.all([
        listTriggers(),
        listTriggersStatus(),
        triggersProviderReady(),
      ]);
      triggers = defs;
      statuses = new Map(statusRows.map((row) => [row.id, row]));
      providerReady = ready;
      loadError = null;
      if (retrying.size > 0) {
        const settled = new Map(retrying);
        for (const [id, firedAtMs] of settled) {
          if (statuses.get(id)?.lastFiring?.firedAtMs !== firedAtMs) settled.delete(id);
        }
        if (settled.size !== retrying.size) retrying = settled;
      }
    } catch (error) {
      loadError = String(error);
    } finally {
      loaded = true;
    }
  }

  async function onRunAgain(trigger: TriggerDefinition, conversationId: string): Promise<void> {
    const firedAtMs = statuses.get(trigger.id)?.lastFiring?.firedAtMs;
    if (firedAtMs === undefined || retrying.has(trigger.id)) return;
    retrying = new Map(retrying).set(trigger.id, firedAtMs);
    try {
      await runTriggerAgain(trigger.id, conversationId);
    } catch (error) {
      const next = new Map(retrying);
      next.delete(trigger.id);
      retrying = next;
      await message(`Could not run "${trigger.name}" again: ${error}`, {
        title: "Triggers",
        kind: "error",
      });
    }
  }

  // While a retry is in flight, poll faster than the ambient 30s so the
  // outcome row shows up promptly.
  $effect(() => {
    if (retrying.size === 0) return;
    const interval = setInterval(() => void refresh(), 5_000);
    return () => clearInterval(interval);
  });

  // Load on mount, refresh on focus/visibility, and poll quietly while the
  // page is open so a firing's ledger row shows up without a manual reload.
  $effect(() => {
    void refresh();
    const onFocus = () => void refresh();
    const onVisibility = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    const interval = setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, REFRESH_INTERVAL_MS);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
      clearInterval(interval);
    };
  });

  function openWizard(
    wizardMode: "create" | "edit" | "import",
    extra: { presetCond?: ConditionType; editing?: TriggerDefinition; imported?: TriggerDraft } = {},
  ): void {
    wizardNonce += 1;
    importError = null;
    view = { mode: "wizard", wizardMode, nonce: wizardNonce, ...extra };
  }

  function onSaved(trigger: TriggerDefinition): void {
    view = { mode: "list" };
    flashId = trigger.id;
    setTimeout(() => {
      if (flashId === trigger.id) flashId = null;
    }, 1800);
    void refresh();
  }

  async function onToggle(trigger: TriggerDefinition): Promise<void> {
    // Optimistic flip; the refresh re-reads the authoritative file.
    triggers = triggers.map((t) =>
      t.id === trigger.id ? { ...t, enabled: !t.enabled } : t,
    );
    try {
      await updateTrigger({ ...trigger, enabled: !trigger.enabled });
    } catch (error) {
      await message(`Could not update "${trigger.name}": ${error}`, {
        title: "Triggers",
        kind: "error",
      });
    }
    void refresh();
  }

  async function onDelete(trigger: TriggerDefinition): Promise<void> {
    const ok = await confirm(
      `Delete "${trigger.name}"? Past runs stay in your chat rail.`,
      { title: "Delete trigger", kind: "warning" },
    );
    if (!ok) return;
    try {
      await deleteTrigger(trigger.id);
    } catch (error) {
      await message(`Could not delete "${trigger.name}": ${error}`, {
        title: "Triggers",
        kind: "error",
      });
    }
    void refresh();
  }

  function onShare(trigger: TriggerDefinition): void {
    void writeText(shareTriggerJson(trigger)).catch(() => {});
  }

  async function onImport(): Promise<void> {
    let text = "";
    try {
      text = await readText();
    } catch {
      // Empty clipboard reads as empty text; the parser explains it inline.
    }
    const parsed = parseTriggerJson(text);
    if (!parsed.ok) {
      importError = parsed.error;
      return;
    }
    importError = null;
    openWizard("import", { imported: parsed.draft });
  }

  function onOpenRun(conversationId: string): void {
    conversationStore.requestOpen(conversationId);
    void goto("/insights");
  }

  function onSetupProvider(): void {
    void openSettings("intelligence");
  }
</script>

<div class="triggers-page">
  {#if view.mode === "list"}
    <div class="list-scroll">
      <div class="list-col">
        <div class="page-head">
          <div>
            <h1>Triggers</h1>
            <p class="sub">
              When something happens, Mnema runs your prompt over what it captured and saves the
              result as a document.
            </p>
          </div>
          <span class="spacer"></span>
          <button
            class="btn"
            type="button"
            title="Paste shared Trigger JSON — it prefills the wizard for review, never saves directly"
            onclick={() => void onImport()}
          >Import</button>
        </div>
        {#if importError}
          <p class="import-error" role="alert">
            Import failed: {importError}
            <button type="button" class="dismiss" onclick={() => (importError = null)}
              >dismiss</button>
          </p>
        {/if}
        {#if loadError}
          <p class="load-error" role="alert">{loadError}</p>
        {/if}
        {#if loaded}
          <TriggersList
            {triggers}
            {statuses}
            {providerReady}
            {flashId}
            ontoggle={(trigger) => void onToggle(trigger)}
            onedit={(trigger) => openWizard("edit", { editing: trigger })}
            onshare={onShare}
            ondelete={(trigger) => void onDelete(trigger)}
            onadd={(cond) => openWizard("create", { presetCond: cond })}
            onopenrun={onOpenRun}
            onrunagain={(trigger, conversationId) => void onRunAgain(trigger, conversationId)}
            {retryingIds}
            onsetupprovider={onSetupProvider}
          />
        {/if}
      </div>
    </div>
  {:else}
    <div class="wiz-scroll">
      {#key view.nonce}
        <TriggerWizard
          mode={view.wizardMode}
          presetCond={view.presetCond}
          editing={view.editing ?? null}
          imported={view.imported ?? null}
          {providerReady}
          oncancel={() => (view = { mode: "list" })}
          onsaved={onSaved}
          onsetupprovider={onSetupProvider}
        />
      {/key}
    </div>
  {/if}
</div>

<style>
  /* WKWebView flex trap: fill via flex on a flex-column parent, not height:100%. */
  .triggers-page {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--app-bg);
    color: var(--app-fg);
  }
  .list-scroll,
  .wiz-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 22px 24px 28px;
  }
  .wiz-scroll {
    padding-top: 26px;
  }
  .list-col {
    max-width: 860px;
    margin: 0 auto;
  }

  .page-head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 22px;
  }
  .page-head h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    line-height: 1.3;
  }
  .page-head .sub {
    margin: 3px 0 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }
  .page-head .spacer {
    flex: 1 1 auto;
  }

  .load-error {
    margin: 0 0 14px;
    font-size: 11px;
    color: var(--app-danger-text);
  }

  .import-error {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0 0 14px;
    padding: 7px 10px;
    font-size: 11.5px;
    color: var(--app-danger-text);
    border: 1px solid var(--app-danger-border, var(--app-border));
    border-radius: 6px;
  }
  .import-error .dismiss {
    margin-left: auto;
    font: inherit;
    font-size: 11px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--app-text-muted);
    cursor: pointer;
    text-decoration: underline;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 12px;
    padding: 5px 11px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .btn:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .btn:active {
    transform: translateY(1px);
  }
  .btn:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }
</style>
