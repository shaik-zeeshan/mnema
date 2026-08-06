<script lang="ts">
  // The 256px inspector on the Subjects list: the selected subject's record.
  //
  // It is why the row can stay one line — the whole record lives here, so no row
  // ever grows a second column. Every figure is read off the loaded data or the
  // row is absent (G8); "grounded in" only lists evidence whose Activity
  // actually resolved.
  import IconPanel from "~icons/lucide/panel-right";
  import IconChevron from "~icons/lucide/chevron-right";
  import IconEye from "~icons/lucide/eye";
  import { convictionTierId } from "$lib/insights/subjectsTiers";
  import { ago, conf, pct } from "./format";
  import { openEvidenceRef } from "./open-evidence";
  import type { SubjectsData } from "./subjects-data.svelte";

  interface Props {
    data: SubjectsData;
    onopen: (subject: string) => void;
  }

  let { data, onopen }: Props = $props();

  const row = $derived(data.selectedRow);

  // The tier is the engine's, so it is asked for rather than re-derived here.
  const TIER_NAME: Record<string, string> = {
    strong: "Strongly held",
    forming: "Forming",
    shaping: "Just taking shape",
    fading: "Fading",
  };
  const tierName = $derived(row ? TIER_NAME[convictionTierId(row)] : "");

  const trendName = $derived(
    row?.trend === "up" ? "Warming" : row?.trend === "down" ? "Cooling" : "Steady",
  );

  // "View frame" is drawn only when a resolved piece of evidence actually
  // points at a frame — never as a button that would land on the Timeline top.
  const frameEvidence = $derived(data.grounding.find((g) => g.frameId !== null));

  function viewFrame(): void {
    const g = frameEvidence;
    if (!g || g.frameId === null) return;
    void openEvidenceRef({ subjectType: "frame", subjectId: g.frameId, isHeadline: false });
  }
</script>

<aside class="ss-insp" aria-label="Inspector">
  <div class="ss-insp__h">
    <span class="ic" aria-hidden="true"><IconPanel /></span>
    <span>Inspector</span>
  </div>

  <div class="ss-insp__b">
    {#if !row}
      <p class="ss-insp__empty">Select a subject to see its record here.</p>
    {:else}
      <div class="ss-insp__sec">
        <span>Subject</span>
        <span class="tier">{tierName}</span>
      </div>
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Name</span>
        <span class="ss-kv__v name">{row.subject}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Top</span>
        <span class="ss-kv__v is-mono">{conf(row.topConfidence)} · {pct(row.topConfidence)}%</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Trend</span>
        <span class="ss-kv__v">{trendName}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Moved</span>
        <span class="ss-kv__v is-mono">{ago(row.lastMovedAtMs)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Pinned</span>
        <span class="ss-kv__v"
          >{row.pinned ? "Yes — protected from decay" : "No"}</span
        >
      </div>

      <div class="ss-insp__sec"><span>Conclusions · {row.conclusionCount}</span></div>
      <div class="cs">
        {#each row.conclusions as c (c.id)}
          <div class="c" class:is-faded={c.status === "faded"}>
            <p class="c__st">{c.statement}</p>
            <div class="c__m">
              <span class="cbar"
                ><i
                  class:is-dim={c.status === "faded"}
                  style="width:{pct(c.confidence)}%"
                ></i></span
              >
              <span class="c__p">{pct(c.confidence)}%</span>
            </div>
          </div>
        {/each}
      </div>

      {#if data.grounding.length > 0}
        <div class="ss-insp__sec"><span>Grounded in</span></div>
        {#each data.grounding as g (g.activityId)}
          <div class="ss-kv">
            <span class="ss-kv__k">{g.source}</span>
            <span class="ss-kv__v is-mono">{ago(g.atMs)} · {g.title}</span>
          </div>
        {/each}
      {/if}

      <div class="ss-insp__sec"><span>Actions</span></div>
      <div class="acts">
        <button
          type="button"
          class="btn btn--sm btn--primary"
          onclick={() => onopen(row.subject)}
        >
          <span class="ic" aria-hidden="true"><IconChevron /></span>
          Open subject
          <span class="kbd">⏎</span>
        </button>
        {#if frameEvidence}
          <button type="button" class="btn btn--sm" onclick={viewFrame}>
            <span class="ic" aria-hidden="true"><IconEye /></span>
            View frame
          </button>
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .ic {
    display: flex;
    font-size: 11px;
  }

  .tier {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    text-transform: none;
    letter-spacing: 0;
    color: var(--app-text-muted);
  }

  .name {
    font: var(--w-semi) 15px / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-title);
  }

  .cs {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 4px var(--s-10) 0;
  }

  .c.is-faded {
    opacity: 0.55;
  }

  .c__st {
    margin: 0 0 3px;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .c__m {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  .cbar {
    position: relative;
    flex: 1 1 auto;
    height: 4px;
    border-radius: 2px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }

  .cbar i {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--app-accent);
  }

  .cbar i.is-dim {
    background: var(--app-text-faint);
  }

  .c__p {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }

  .acts {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px var(--s-10) 0;
  }

  .acts .btn {
    width: 100%;
    justify-content: flex-start;
  }

  .acts .kbd {
    margin-left: auto;
    background: transparent;
  }
</style>
