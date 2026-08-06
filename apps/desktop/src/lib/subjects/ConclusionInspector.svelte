<script lang="ts">
  // The inspector on an opened subject: the SELECTED belief's own record, so the
  // story column never has to explain itself twice.
  //
  // "How it decays" states only constants the frontend actually holds. The
  // engine's fade HALF-LIFE lives in Rust (`confidence.rs FADE_HALF_LIFE_DAYS`)
  // and is not exposed to the UI, so that row is absent rather than guessed —
  // G8, the same rule the status strip follows.
  import IconPanel from "~icons/lucide/panel-right";
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";
  import { ago, conf, pct, shortDate } from "./format";
  import type { SubjectDetailData } from "./subject-detail-data.svelte";

  interface Props {
    data: SubjectDetailData;
  }

  let { data }: Props = $props();

  const c = $derived(data.selected);
  const history = $derived(c ? data.historyOf(c.id) : []);
  const formedAt = $derived(history.length > 0 ? history[0] : (c?.confidence ?? 0));
</script>

<aside class="ss-insp" aria-label="Inspector">
  <div class="ss-insp__h">
    <span class="ic" aria-hidden="true"><IconPanel /></span>
    <span>Inspector</span>
  </div>

  <div class="ss-insp__b">
    {#if !c}
      <p class="ss-insp__empty">Select a conclusion to see its record here.</p>
    {:else}
      <div class="ss-insp__sec">
        <span>Selection</span>
        <span class="pos"
          >Conclusion {data.selectedIndex + 1} of {data.conclusionCount}</span
        >
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Confidence</span>
        <span class="ss-kv__v is-mono">{conf(c.confidence)} · {pct(c.confidence)}%</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Status</span>
        <span class="ss-kv__v"
          >{c.status === "faded" ? "Below floor" : "Visible"}{c.pinned ? " · pinned" : ""}</span
        >
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Snapshots</span>
        <span class="ss-kv__v is-mono">{data.snapshotCount(c.id)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Formed</span>
        <span class="ss-kv__v is-mono"
          >{shortDate(c.formedAtMs)} · at {pct(formedAt)}%</span
        >
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Last support</span>
        <span class="ss-kv__v is-mono">{ago(c.lastSupportedAtMs)}</span>
      </div>

      <div class="ss-insp__sec"><span>Evidence</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">Supports</span>
        <span class="ss-kv__v is-mono"
          >{data.evidenceCounts.supports}
          {data.evidenceCounts.supports === 1 ? "activity" : "activities"}</span
        >
      </div>
      {#if data.evidenceCounts.contradicts > 0}
        <div class="ss-kv">
          <span class="ss-kv__k">Contradicts</span>
          <span class="ss-kv__v is-mono"
            >{data.evidenceCounts.contradicts}
            {data.evidenceCounts.contradicts === 1 ? "activity" : "activities"}</span
          >
        </div>
      {/if}
      {#if c.replacedStatement}
        <div class="ss-kv">
          <span class="ss-kv__k">Replaced</span>
          <span class="ss-kv__v is-mono">1 earlier take</span>
        </div>
      {/if}

      <div class="ss-insp__sec"><span>How it decays</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">Floor</span>
        <span class="ss-kv__v is-mono">{DISPLAY_FLOOR.toFixed(2)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Pinned</span>
        <span class="ss-kv__v">{c.pinned ? "Exempt from decay" : "Not pinned — decays"}</span>
      </div>
      <p class="note">
        Pinning is the only way to hold a belief still. Dismissing removes it from the
        dossier; it can form again if your activity still supports it.
      </p>
    {/if}
  </div>
</aside>

<style>
  .ic {
    display: flex;
    font-size: 11px;
  }

  .pos {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    text-transform: none;
    letter-spacing: 0;
    color: var(--app-text-muted);
  }

  .note {
    margin: var(--gap-label) 0 0;
    padding: 0 var(--s-10);
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
  }
</style>
