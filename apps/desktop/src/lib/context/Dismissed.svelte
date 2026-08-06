<script lang="ts">
  // The dismissed archive — the page's one concession to inferred data.
  //
  // These are beliefs Mnema CONCLUDED and the user removed. Restore is careful
  // on purpose: it clears the dismissal so the belief is ALLOWED to form again
  // on a later derivation pass. It does not put the old conclusion back, and if
  // the activity no longer supports it nothing returns — which is correct.
  import type { DismissedView } from "$lib/types/recording";
  import { dismissedKey, relativeTime, type ContextData } from "./context-data.svelte";

  interface Props {
    data: ContextData;
  }

  let { data }: Props = $props();

  const focusedKey = $derived(
    data.focus.kind === "dismissed" ? dismissedKey(data.focus.item) : null,
  );

  function focus(d: DismissedView): void {
    data.focus = { kind: "dismissed", item: d };
  }
</script>

<div class="chd">
  <span class="chd__n">Dismissed</span>
  <span class="t-meta note">
    {data.showDismissed
      ? "Restoring lets one form again — only if your activity still supports it"
      : "Beliefs you removed from your dossier"}
  </span>
  <span class="chd__c">
    {#if data.dismissedCount !== null}<span class="is-mono">{data.dismissedCount}</span>{/if}
    <button
      type="button"
      class="btn btn--sm btn--ghost"
      aria-expanded={data.showDismissed}
      onclick={() => (data.showDismissed = !data.showDismissed)}
      >{data.showDismissed ? "Hide" : "Show"}</button
    >
  </span>
</div>

{#if data.showDismissed}
  {#if data.dismissedError && (data.dismissed?.length ?? 0) === 0}
    <p class="quiet">{data.dismissedError}</p>
  {:else if (data.dismissed?.length ?? 0) === 0}
    <p class="quiet">You haven't removed any beliefs.</p>
  {:else}
    <div class="ss-grp grp">
      {#each data.dismissed ?? [] as d (dismissedKey(d))}
        <div
          class="arow"
          class:is-sel={focusedKey === dismissedKey(d)}
          role="button"
          tabindex="0"
          onclick={() => focus(d)}
          onfocusin={() => focus(d)}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              focus(d);
            }
          }}
        >
          <div class="arow__t">
            <p class="arow__x">{d.statement}</p>
            <div class="arow__m">
              {#if d.subject}<span class="topicchip">{d.subject}</span>{/if}
              <span class="t-meta age is-mono">dismissed {relativeTime(d.dismissedAtMs)}</span>
            </div>
          </div>
          <div class="arow__a">
            <button
              type="button"
              class="btn btn--sm"
              disabled={data.restoringKey === dismissedKey(d)}
              onclick={() => void data.restore(d)}>Restore</button
            >
          </div>
        </div>
      {/each}
    </div>
    {#if data.dismissedError}
      <p class="quiet err" role="alert">{data.dismissedError}</p>
    {/if}
  {/if}
{/if}

<style>
  .chd {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    height: 26px;
    padding: 0 var(--s-16);
    margin-top: 14px;
    background: var(--app-bg);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .chd__n {
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }

  .note {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chd__c {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  .grp {
    margin: var(--s-8) var(--s-16) 0;
  }

  .quiet {
    margin: var(--s-10) var(--s-16) 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .err {
    color: var(--app-danger-strong);
  }

  .arow {
    display: flex;
    align-items: flex-start;
    gap: var(--s-10);
    padding: var(--s-8) var(--s-10);
    border-top: var(--hairline) solid var(--app-border);
  }

  .arow:first-child {
    border-top: 0;
  }

  .arow:hover {
    background: var(--app-surface-hover);
  }

  .arow:focus-visible {
    outline: none;
  }

  .arow.is-sel {
    background: var(--app-surface-active);
    box-shadow: inset 2px 0 0 var(--app-accent);
  }

  .arow__t {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  /* One notch smaller than an authored line: this is not what you wrote. */
  .arow__x {
    margin: 0;
    max-width: 72ch;
    font: var(--w-regular) var(--t-ui) / 1.5 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .arow__m {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  .arow__a {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
  }

  .age {
    color: var(--app-text-subtle);
  }

  .topicchip {
    display: inline-flex;
    align-items: center;
    height: 17px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
  }
</style>
