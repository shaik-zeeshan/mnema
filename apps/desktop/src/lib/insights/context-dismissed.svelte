<script lang="ts">
  // The dismissed archive, expanded — the negative space of the inferred
  // dossier. "Restore" is NOT undelete: dismissing deleted the conclusion row
  // and recorded a veto, so restoring only lifts the veto and the belief comes
  // back solely if a later derivation pass still finds evidence for it. The
  // copy says exactly that.
  import type { DismissedView } from "$lib/types/recording";
  import { contextAgo } from "$lib/insights/context-time";

  interface Props {
    rows: DismissedView[] | null;
    error: string | null;
    onrestore: (d: DismissedView) => Promise<void>;
  }

  let { rows, error, onrestore }: Props = $props();

  let busy = $state<string | null>(null);
  const keyOf = (d: DismissedView) => `${d.subject}\0${d.statement}`;

  async function restore(d: DismissedView): Promise<void> {
    if (busy) return;
    busy = keyOf(d);
    try {
      await onrestore(d);
    } finally {
      busy = null;
    }
  }
</script>

<div class="plate box">
  <p class="note">
    Beliefs you removed from your dossier. Restoring lets one form again — only if your
    activity still supports it.
  </p>
  {#if error}
    <p class="err">{error}</p>
  {/if}
  {#each rows ?? [] as d (keyOf(d))}
    <div class="stmt">
      <span class="stmt__t">
        <span class="x">{d.statement}</span>
        <span class="stmt__m">
          <span class="topic">{d.subject}</span>
          <span class="t-meta subtle">dismissed {contextAgo(d.dismissedAtMs)}</span>
        </span>
      </span>
      <button
        type="button"
        class="btn btn--ghost btn--sm"
        disabled={busy === keyOf(d)}
        aria-busy={busy === keyOf(d)}
        onclick={() => void restore(d)}
      >
        {busy === keyOf(d) ? "Restoring…" : "Restore"}
      </button>
    </div>
  {/each}
</div>

<style>
  .box {
    border-radius: var(--r-panel);
    padding: 6px 12px 8px;
  }
  .note,
  .err {
    margin: 8px 0 2px;
    max-width: 70ch;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .err {
    color: var(--app-danger);
  }

  .stmt {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 0;
    min-height: 36px;
  }
  .stmt + .stmt {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .stmt__t {
    flex: 1;
    min-width: 0;
  }
  .stmt__t .x {
    display: block;
    max-width: 62ch;
    font: var(--w-medium) var(--t-ui) / 1.45 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .stmt__m {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
    margin-top: 5px;
  }
  .subtle {
    color: var(--app-text-subtle);
  }
  .topic {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--r-sm);
    background: var(--glass-tint);
    color: var(--app-text-muted);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .topic::before {
    content: "[";
  }
  .topic::after {
    content: "]";
  }
</style>
