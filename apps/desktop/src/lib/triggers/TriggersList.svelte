<script lang="ts">
  // The /triggers list (Screen 1 of docs/triggers/mockups/final/DESIGN.md):
  // triggers grouped by condition, each row = enable switch · name (opens the
  // per-trigger Runs view) · condition detail · honest last-run status, hover
  // actions edit · share · delete, and a dashed ghost add-row per section.
  // Pure presentation — the route page owns the data and the handlers.
  import IconArrowUpRight from "~icons/lucide/arrow-up-right";
  import IconCheck from "~icons/lucide/check";
  import IconPlus from "~icons/lucide/plus";
  import IconTriangleAlert from "~icons/lucide/triangle-alert";
  import { tip } from "$lib/components/tooltip";
  import {
    CONDITION_SECTIONS,
    conditionDetail,
    fmtWhen,
    type ConditionType,
    type TriggerDefinition,
    type TriggerStatus,
  } from "$lib/triggers/api";
  import { CONDITION_ICON } from "$lib/triggers/condition-icons";

  interface Props {
    triggers: TriggerDefinition[];
    statuses: Map<string, TriggerStatus>;
    /** Provider Gate: false dims every row into "needs an AI provider". */
    providerReady: boolean;
    /** Row to flash accent-green (just created/saved). */
    flashId: string | null;
    ontoggle: (trigger: TriggerDefinition) => void;
    onedit: (trigger: TriggerDefinition) => void;
    /** Copies Trigger JSON; the "copied" flash waits for this to resolve. */
    onshare: (trigger: TriggerDefinition) => Promise<void>;
    ondelete: (trigger: TriggerDefinition) => void;
    onadd: (cond: ConditionType) => void;
    onopenrun: (conversationId: string) => void;
    /** Name click → the trigger's Runs view (DESIGN.md Screen 2). */
    onopenruns: (trigger: TriggerDefinition) => void;
    onrunagain: (trigger: TriggerDefinition, conversationId: string) => void;
    /** Trigger ids with a Run Again retry in flight (shows "retrying…"). */
    retryingIds: ReadonlySet<string>;
    onsetupprovider: () => void;
  }

  let {
    triggers,
    statuses,
    providerReady,
    flashId,
    ontoggle,
    onedit,
    onshare,
    ondelete,
    onadd,
    onopenrun,
    onopenruns,
    onrunagain,
    retryingIds,
    onsetupprovider,
  }: Props = $props();

  // "copied" / "copy failed" flash per row — success only once the clipboard
  // write actually resolved.
  let copiedId = $state<string | null>(null);
  let copyFailedId = $state<string | null>(null);
  async function share(trigger: TriggerDefinition): Promise<void> {
    const id = trigger.id;
    try {
      await onshare(trigger);
      copiedId = id;
      copyFailedId = null;
    } catch {
      copyFailedId = id;
      copiedId = null;
    }
    setTimeout(() => {
      if (copiedId === id) copiedId = null;
      if (copyFailedId === id) copyFailedId = null;
    }, 1500);
  }

  function sectionTriggers(cond: ConditionType): TriggerDefinition[] {
    return triggers.filter((t) => t.condition.type === cond);
  }

  interface RowStatus {
    kind: "ok" | "skip" | "fail" | "none" | "running";
    word: string;
    rest: string;
    title: string;
    conversationId: string | null;
  }

  function rowStatus(trigger: TriggerDefinition): RowStatus {
    const status = statuses.get(trigger.id);
    // Running / Readiness Wait — the sixth lifecycle state, outranks the
    // last-firing display while a firing is in flight.
    if (status?.runningSinceMs !== undefined) {
      return {
        kind: "running",
        word: "running",
        rest: `· ${fmtWhen(status.runningSinceMs)}`,
        title: "waiting for transcription… can take up to ~15 min",
        conversationId: null,
      };
    }
    const firing = status?.lastFiring;
    if (!firing) {
      return {
        kind: "none",
        word: "no runs yet",
        rest: "",
        title: "This trigger hasn't fired yet — you'll get a notification when a run completes",
        conversationId: null,
      };
    }
    const when = fmtWhen(firing.firedAtMs);
    if (firing.outcome === "completed") {
      return {
        kind: "ok",
        word: "completed",
        rest: `· ${when}`,
        title: firing.conversationId
          ? "The last run produced a document — click to read it"
          : "The last run completed",
        conversationId: firing.conversationId ?? null,
      };
    }
    if (firing.outcome === "skipped") {
      return {
        kind: "skip",
        word: "skipped",
        rest: `${firing.reason ? `— ${firing.reason} ` : ""}· ${when}`,
        title:
          "The condition fired but there was nothing to work with — no document, no notification",
        conversationId: null,
      };
    }
    return {
      kind: "fail",
      word: "failed",
      rest: `${firing.reason ? `— ${firing.reason} ` : ""}· ${when}`,
      title: firing.conversationId
        ? "The run started but did not complete — run again retries this exact firing"
        : "The run started but did not complete — it will try again next time the condition fires",
      conversationId: firing.conversationId ?? null,
    };
  }
</script>

<div class="sections">
  {#each CONDITION_SECTIONS as section (section.cond)}
    {@const rows = sectionTriggers(section.cond)}
    {@const SectionIcon = CONDITION_ICON[section.cond]}
    <section class="cond-section">
      <div class="cond-section-head">
        <div class="cond-section-title">
          <span class="glyph" aria-hidden="true"><SectionIcon /></span>
          <span>{section.title}</span>
        </div>
        <p class="cond-section-blurb">{section.blurb}</p>
      </div>
      <div class="trigger-rows">
        {#if rows.length === 0}
          <p class="sec-empty">Nothing here yet.</p>
        {/if}
        {#each rows as trigger (trigger.id)}
          {@const gated = !providerReady}
          {@const status = rowStatus(trigger)}
          {@const detail = conditionDetail(trigger.condition)}
          <div
            class="trow"
            class:trow--gated={gated}
            class:trow--off={!trigger.enabled && !gated}
            class:trow--new={flashId === trigger.id}
          >
            <button
              type="button"
              class="switch"
              class:on={trigger.enabled}
              role="switch"
              aria-checked={trigger.enabled}
              aria-label={`Enable ${trigger.name}`}
              disabled={gated}
              onclick={() => ontoggle(trigger)}
            ></button>
            <button
              type="button"
              class="trow-name"
              use:tip={"See every run of this trigger, including skips and failures"}
              onclick={() => onopenruns(trigger)}
            >{trigger.name}</button>
            {#if detail}
              <span class="trow-detail">{detail}</span>
            {/if}
            <span class="spacer"></span>
            {#if gated}
              <span
                class="gate-chip"
                use:tip={"This trigger can't run until an AI provider is configured — its condition is ignored until then"}
              >
                <IconTriangleAlert aria-hidden="true" />
                needs an AI provider
              </span>
              <button type="button" class="gate-link" onclick={onsetupprovider}>
                Set up provider
              </button>
            {:else if status.conversationId !== null}
              <button
                type="button"
                class="trow-status st-{status.kind} trow-status--openable"
                use:tip={status.title}
                onclick={() => {
                  if (status.conversationId !== null) onopenrun(status.conversationId);
                }}
              >
                <span class="dot" aria-hidden="true"></span>
                <span class="word">{status.word}</span>
                <span class="rest">{status.rest}</span>
                <span class="open-ind" aria-hidden="true"><IconArrowUpRight /></span>
              </button>
              {#if status.kind === "fail"}
                <button
                  type="button"
                  class="run-again"
                  disabled={retryingIds.has(trigger.id)}
                  use:tip={"Retry this exact firing — same meeting or window, a fresh attempt"}
                  onclick={() => {
                    if (status.conversationId !== null) onrunagain(trigger, status.conversationId);
                  }}
                >{retryingIds.has(trigger.id) ? "retrying…" : "run again"}</button>
              {/if}
            {:else}
              <span class="trow-status st-{status.kind}" use:tip={status.title}>
                <span class="dot" aria-hidden="true"></span>
                <span class="word">{status.word}</span>
                <span class="rest">{status.rest}</span>
              </span>
            {/if}
            <span class="trow-actions">
              <button type="button" class="row-act" onclick={() => onedit(trigger)}>edit</button>
              <button
                type="button"
                class="row-act"
                class:row-act--fail={copyFailedId === trigger.id}
                use:tip={"Copies this trigger as JSON — never carries provider or model config"}
                onclick={() => void share(trigger)}
              >
                {#if copiedId === trigger.id}
                  copied <IconCheck aria-hidden="true" />
                {:else if copyFailedId === trigger.id}
                  copy failed
                {:else}
                  share
                {/if}
              </button>
              <button
                type="button"
                class="row-act row-act--danger"
                onclick={() => ondelete(trigger)}
              >delete</button>
            </span>
          </div>
        {/each}
        <button type="button" class="add-row" onclick={() => onadd(section.cond)}>
          <span class="plus" aria-hidden="true"><IconPlus /></span>
          <span>{section.addLabel}</span>
        </button>
      </div>
    </section>
  {/each}
</div>

<style>
  /* Visual vocabulary from docs/triggers/mockups/final/triggers-ui.html —
     the mockup wins on visuals; tokens are the app-global --app-* set. */
  .cond-section {
    margin-bottom: 24px;
  }
  .cond-section-head {
    margin-bottom: 6px;
  }
  .cond-section-title {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 12px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text);
  }
  .cond-section-title .glyph {
    display: inline-flex;
    color: var(--app-accent-strong);
  }
  .cond-section-title .glyph :global(svg) {
    width: 12px;
    height: 12px;
  }
  .cond-section-blurb {
    margin: 1px 0 0 21px;
    font-size: 11px;
    line-height: 1.55;
    color: var(--app-text-subtle);
  }

  .trigger-rows {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    overflow: hidden;
  }
  .trow {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 8px 13px;
    min-height: 38px;
    transition: background 0.12s ease;
  }
  .trow + .trow {
    border-top: 1px solid var(--app-border);
  }
  .trow:hover {
    background: var(--app-surface-subtle);
  }
  .trow-name {
    font: inherit;
    font-size: 12.5px;
    background: none;
    border: 0;
    padding: 0;
    text-align: left;
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 0 auto;
    max-width: 50%;
    cursor: pointer;
  }
  .trow-name:hover {
    color: var(--app-accent);
  }
  .trow-detail {
    font-size: 10.5px;
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 0 1 auto;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .trow-status {
    font-size: 11px;
    white-space: nowrap;
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    background: none;
    border: 0;
    padding: 0;
    font-family: inherit;
  }
  .trow-status .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    align-self: center;
    flex: 0 0 auto;
  }
  .trow-status .rest {
    color: var(--app-text-muted);
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
  .st-none .dot,
  .st-none .word {
    color: var(--app-text-subtle);
  }
  .st-running .dot,
  .st-running .word {
    color: var(--app-accent-strong);
  }
  .st-running .dot {
    animation: pulseDot 1.2s ease-in-out infinite;
  }
  @keyframes pulseDot {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.25;
    }
  }
  .trow-status--openable {
    cursor: pointer;
  }
  /* rest-state affordance: the openable status reads as a link */
  .trow-status--openable .word {
    text-decoration: underline dotted;
    text-underline-offset: 2px;
  }
  .trow-status--openable .open-ind {
    display: inline-flex;
    align-self: center;
    color: var(--app-text-subtle);
  }
  .trow-status--openable .open-ind :global(svg) {
    width: 10px;
    height: 10px;
  }
  .trow-status--openable:hover .open-ind {
    color: var(--app-text-strong);
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
  }
  .run-again:hover:not(:disabled) {
    color: var(--app-text-strong);
  }
  .run-again:disabled {
    color: var(--app-text-subtle);
    text-decoration: none;
    cursor: default;
  }
  .trow-status--openable:hover .rest {
    color: var(--app-text-strong);
  }
  .trow--off .trow-name,
  .trow--off .trow-detail,
  .trow--off .trow-status {
    opacity: 0.45;
  }

  /* hover row actions */
  .trow-actions {
    display: none;
    align-items: center;
    gap: 2px;
    flex: 0 0 auto;
  }
  .trow:hover .trow-actions,
  .trow:focus-within .trow-actions {
    display: inline-flex;
  }
  .row-act {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font: inherit;
    font-size: 10.5px;
    background: none;
    border: 0;
    padding: 2px 6px;
    border-radius: 4px;
    color: var(--app-text-subtle);
    cursor: pointer;
    transition: color 0.12s ease, background 0.12s ease;
  }
  .row-act :global(svg) {
    width: 10px;
    height: 10px;
  }
  .row-act:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }
  .row-act--danger:hover {
    color: var(--app-danger-text);
    background: var(--app-danger-bg);
  }
  .row-act--fail,
  .row-act--fail:hover {
    color: var(--app-danger-text);
  }

  /* shared keyboard-focus affordance (B5) */
  .row-act:focus-visible,
  .run-again:focus-visible,
  .gate-link:focus-visible,
  .trow-status--openable:focus-visible,
  .trow-name:focus-visible,
  .add-row:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }

  /* newly created/saved row flash (success feedback) */
  .trow--new {
    animation: rowFlash 1.8s ease;
  }
  @keyframes rowFlash {
    0%,
    35% {
      background: var(--app-accent-bg);
    }
    100% {
      background: transparent;
    }
  }

  .sec-empty {
    margin: 0;
    padding: 10px 13px;
    font-size: 11px;
    color: var(--app-text-subtle);
  }

  /* provider-gated row */
  .trow--gated .trow-name,
  .trow--gated .trow-detail {
    opacity: 0.5;
  }
  .gate-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    padding: 1px 8px;
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
    background: var(--app-warn-bg);
    color: var(--app-warn);
    white-space: nowrap;
  }
  .gate-chip :global(svg) {
    width: 10px;
    height: 10px;
    flex: 0 0 auto;
  }
  .gate-link {
    font: inherit;
    font-size: 11px;
    background: none;
    border: 0;
    padding: 0;
    color: var(--app-warn);
    text-decoration: underline;
    text-underline-offset: 2px;
    white-space: nowrap;
    cursor: pointer;
  }
  .gate-link:hover {
    color: var(--app-text-strong);
  }

  /* ghost add row */
  .add-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    font: inherit;
    font-size: 11.5px;
    padding: 7px 13px;
    border: 0;
    border-top: 1px dashed var(--app-border);
    background: transparent;
    color: var(--app-text-subtle);
    cursor: pointer;
    text-align: left;
    transition: color 0.12s ease, background 0.12s ease;
  }
  .trigger-rows > .add-row:first-child {
    border-top: 0;
  }
  .add-row:hover {
    color: var(--app-accent);
    background: var(--app-surface-subtle);
  }
  .add-row .plus {
    display: inline-flex;
    color: var(--app-text-faint);
    transition: color 0.12s ease;
  }
  .add-row .plus :global(svg) {
    width: 11px;
    height: 11px;
  }
  .add-row:hover .plus {
    color: var(--app-accent);
  }

  /* enable switch (mockup .switch) */
  .switch {
    position: relative;
    flex: 0 0 auto;
    width: 30px;
    height: 17px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    background: var(--app-surface-hover);
    cursor: pointer;
    padding: 0;
    transition: background 0.12s ease, border-color 0.12s ease;
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--app-text-muted);
    transition: left 0.12s ease, background 0.12s ease;
  }
  .switch.on {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  .switch.on::after {
    left: 15px;
    background: var(--app-accent);
  }
  .switch:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .switch:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }
</style>
