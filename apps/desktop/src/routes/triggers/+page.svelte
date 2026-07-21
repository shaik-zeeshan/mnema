<script lang="ts">
  // /triggers — the Triggers management surface (issue #182; final design in
  // docs/triggers/mockups/final/DESIGN.md). This page owns the data (list +
  // status + provider gate) and swaps between the list, the per-trigger Runs
  // view (Screen 2, driven by `?runs=<id>`), and the 3-step wizard
  // (`?edit=<id>` deep-links into edit); TriggersList / TriggerWizard are
  // presentation.
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import IconArrowUpRight from "~icons/lucide/arrow-up-right";
  import { tip } from "$lib/components/tooltip";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import { openSettings } from "$lib/surface-windows";
  import TriggersList from "$lib/triggers/TriggersList.svelte";
  import TriggerWizard from "$lib/triggers/TriggerWizard.svelte";
  import {
    CONDITION_LABEL,
    conditionDetail,
    deleteTrigger,
    fmtWhen,
    listTriggerFirings,
    listTriggers,
    listTriggersStatus,
    runTriggerAgain,
    triggersProviderReady,
    updateTrigger,
    type ConditionType,
    type TriggerDefinition,
    type TriggerDraft,
    type TriggerLastFiring,
    type TriggerStatus,
  } from "$lib/triggers/api";
  import { CONDITION_ICON } from "$lib/triggers/condition-icons";
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

  // ── URL contract ───────────────────────────────────────────────────────────
  // `/triggers?runs=<id>` → the per-trigger Runs view; `/triggers?edit=<id>` →
  // the wizard in edit mode (consumed once, then the param is cleared).
  const runsId = $derived($page.url.searchParams.get("runs"));
  const runsTrigger = $derived(
    runsId === null ? null : (triggers.find((t) => t.id === runsId) ?? null),
  );

  // Unknown/deleted `runs` id → fall back to the list gracefully.
  $effect(() => {
    if (loaded && runsId !== null && runsTrigger === null) {
      void goto("/triggers", { replaceState: true });
    }
  });

  // `?edit=<id>` deep link: open the wizard once, then clear the param.
  $effect(() => {
    const editId = $page.url.searchParams.get("edit");
    if (editId === null || !loaded) return;
    const trigger = triggers.find((t) => t.id === editId);
    if (trigger) openWizard("edit", { editing: trigger });
    void goto("/triggers", { replaceState: true });
  });

  // ── Runs ledger (Screen 2 data) ──────────────────────────────────────────
  // Re-fetched whenever the trigger changes or a status refresh lands, so a
  // retry's new ledger row shows up without a manual reload.
  let firings = $state<TriggerLastFiring[] | null>(null);
  let firingsFor = $state<string | null>(null);
  const runFirings = $derived(firingsFor === runsId ? firings : null);
  $effect(() => {
    const id = runsId;
    void statuses; // refresh cadence piggybacks on the status poll
    if (id === null) return;
    listTriggerFirings(id).then(
      (rows) => {
        firingsFor = id;
        firings = rows;
      },
      () => {
        firingsFor = id;
        firings = [];
      },
    );
  });

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

  // While a retry or a firing is in flight, poll faster than the ambient 30s
  // so the running state and the outcome row show up promptly.
  const anyRunning = $derived(
    [...statuses.values()].some((status) => status.runningSinceMs !== undefined),
  );
  $effect(() => {
    if (retrying.size === 0 && !anyRunning) return;
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

  // The "copied" flash in TriggersList waits on this promise — a clipboard
  // failure must reject, never flash success.
  function onShare(trigger: TriggerDefinition): Promise<void> {
    return writeText(shareTriggerJson(trigger));
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

  function onOpenRuns(trigger: TriggerDefinition): void {
    void goto(`/triggers?runs=${encodeURIComponent(trigger.id)}`);
  }

  function onSetupProvider(): void {
    void openSettings("intelligence");
  }

  const RUN_WORD: Record<TriggerLastFiring["outcome"], string> = {
    completed: "completed",
    skipped: "skipped",
    failed: "failed",
  };
  const RUN_KIND: Record<TriggerLastFiring["outcome"], string> = {
    completed: "ok",
    skipped: "skip",
    failed: "fail",
  };
</script>

<div class="triggers-page">
  {#if view.mode === "wizard"}
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
  {:else if runsTrigger !== null}
    {@const RunsIcon = CONDITION_ICON[runsTrigger.condition.type]}
    <div class="list-scroll">
      <div class="list-col">
        <nav class="crumb">
          <button type="button" class="back" onclick={() => void goto("/triggers")}
            >triggers</button>
          <span class="sep" aria-hidden="true">/</span>
          <span class="here">{runsTrigger.name}</span>
        </nav>
        <div class="runs-head">
          <h1>{runsTrigger.name}</h1>
          <span class="cond-echo">
            <span class="cond-icon" aria-hidden="true"><RunsIcon /></span>
            {CONDITION_LABEL[runsTrigger.condition.type]}
            {conditionDetail(runsTrigger.condition)}
          </span>
        </div>
        <p class="runs-sub">
          Every firing, including the quiet ones — skips and failures never notify, they just show
          up here.
        </p>
        <div class="run-rows">
          {#if runFirings === null}
            <p class="runs-empty">loading…</p>
          {:else if runFirings.length === 0}
            <p class="runs-empty">
              {providerReady
                ? "No runs yet — it hasn't fired. You'll get a notification when a run completes."
                : "No runs — this trigger needs an AI provider before it can fire."}
            </p>
          {:else}
            <!-- keyed by index: firedAtMs could collide across a retry and its
                 source row, and a duplicate key crashes the whole page -->
            {#each runFirings as firing, i (i)}
              <div class="run-row">
                <span class="run-status st-{RUN_KIND[firing.outcome]}">
                  <span class="dot" aria-hidden="true"></span>
                  <span class="word">{RUN_WORD[firing.outcome]}</span>
                </span>
                {#if firing.outcome === "completed" && firing.conversationId !== undefined}
                  <button
                    type="button"
                    class="run-open"
                    use:tip={"This run produced a document — click to read it"}
                    onclick={() => {
                      if (firing.conversationId !== undefined) onOpenRun(firing.conversationId);
                    }}
                  >
                    open
                    <span class="open-ind" aria-hidden="true"><IconArrowUpRight /></span>
                  </button>
                {:else if firing.reason !== undefined && firing.reason !== ""}
                  <span class="run-reason">— {firing.reason}</span>
                {/if}
                <span class="spacer"></span>
                {#if firing.outcome === "failed" && firing.conversationId !== undefined && providerReady}
                  <button
                    type="button"
                    class="run-again"
                    disabled={retryingIds.has(runsTrigger.id)}
                    use:tip={"Retry this exact firing — same meeting or window, a fresh attempt"}
                    onclick={() => {
                      if (firing.conversationId !== undefined && runsTrigger !== null)
                        void onRunAgain(runsTrigger, firing.conversationId);
                    }}
                  >{retryingIds.has(runsTrigger.id) ? "retrying…" : "run again"}</button>
                {/if}
                <span class="run-when">{fmtWhen(firing.firedAtMs)}</span>
              </div>
            {/each}
          {/if}
        </div>
        <p class="runs-note">
          Runs are ordinary conversations — they also live in your chat rail under the Triggers
          filter.
        </p>
      </div>
    </div>
  {:else}
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
            use:tip={"Paste shared Trigger JSON — it prefills the wizard for review, never saves directly"}
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
            onopenruns={onOpenRuns}
            onrunagain={(trigger, conversationId) => void onRunAgain(trigger, conversationId)}
            {retryingIds}
            onsetupprovider={onSetupProvider}
          />
        {:else}
          <p class="loading" aria-live="polite">loading triggers…</p>
        {/if}
      </div>
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

  .loading {
    margin: 0;
    font-size: 11.5px;
    color: var(--app-text-subtle);
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

  /* ── Runs view (DESIGN.md Screen 2) ─────────────────────────────────────── */
  .crumb {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 11.5px;
    color: var(--app-text-muted);
    margin-bottom: 14px;
  }
  .crumb .back {
    cursor: pointer;
    color: var(--app-text-muted);
    background: none;
    border: 0;
    font: inherit;
    padding: 0;
  }
  .crumb .back:hover {
    color: var(--app-text-strong);
  }
  .crumb .sep {
    color: var(--app-text-faint);
  }
  .crumb .here {
    color: var(--app-text-strong);
  }
  .runs-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 4px;
  }
  .runs-head h1 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .cond-echo {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .cond-echo .cond-icon {
    display: inline-flex;
    color: var(--app-accent-strong);
  }
  .cond-echo .cond-icon :global(svg) {
    width: 11px;
    height: 11px;
  }
  .runs-sub {
    margin: 0 0 18px;
    font-size: 11px;
    color: var(--app-text-subtle);
  }
  .run-rows {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    overflow: hidden;
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 8px 13px;
    min-height: 38px;
  }
  .run-row + .run-row {
    border-top: 1px solid var(--app-border);
  }
  .run-row:hover {
    background: var(--app-surface-subtle);
  }
  .run-row .spacer {
    flex: 1 1 auto;
  }
  .run-status {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    font-size: 11px;
    white-space: nowrap;
    flex: 0 0 auto;
  }
  .run-status .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    align-self: center;
    flex: 0 0 auto;
  }
  .st-ok .dot,
  .st-ok .word {
    color: var(--app-accent-strong);
  }
  .st-skip .dot,
  .st-skip .word {
    color: var(--app-neutral-text);
  }
  .st-fail .dot,
  .st-fail .word {
    color: var(--app-danger-text);
  }
  .run-open {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font: inherit;
    font-size: 11px;
    background: none;
    border: 0;
    padding: 0;
    color: var(--app-text);
    text-decoration: underline dotted;
    text-underline-offset: 2px;
    cursor: pointer;
    white-space: nowrap;
  }
  .run-open:hover {
    color: var(--app-accent);
  }
  .run-open .open-ind {
    display: inline-flex;
    color: var(--app-text-subtle);
  }
  .run-open:hover .open-ind {
    color: var(--app-accent);
  }
  .run-open .open-ind :global(svg) {
    width: 10px;
    height: 10px;
  }
  .run-reason {
    font-size: 11.5px;
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
  }
  .run-when {
    font-size: 11px;
    color: var(--app-text-subtle);
    white-space: nowrap;
    flex: 0 0 auto;
  }
  .run-again {
    font: inherit;
    font-size: 10.5px;
    background: none;
    border: 0;
    padding: 0;
    color: var(--app-danger-text);
    text-decoration: underline;
    text-underline-offset: 2px;
    white-space: nowrap;
    cursor: pointer;
    flex: 0 0 auto;
  }
  .run-again:hover:not(:disabled) {
    color: var(--app-text-strong);
  }
  .run-again:disabled {
    color: var(--app-text-subtle);
    text-decoration: none;
    cursor: default;
  }
  .runs-empty {
    margin: 0;
    padding: 18px 14px;
    font-size: 11.5px;
    color: var(--app-text-subtle);
  }
  .runs-note {
    margin: 14px 0 0;
    font-size: 10.5px;
    color: var(--app-text-faint);
  }

  /* shared keyboard-focus affordance (B5) */
  .btn:focus-visible,
  .crumb .back:focus-visible,
  .run-open:focus-visible,
  .run-again:focus-visible,
  .import-error .dismiss:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }
</style>
