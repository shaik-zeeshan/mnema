<script lang="ts">
  // Subjects lede (insights.html renderSubjects `.in-read`): dossier kicker, a
  // headline templated over the four conviction tiers (no LLM pass), and one
  // stat tile per tier. The cuts live in subjectsTiers.ts.
  import { subjectsHeadline, type ConvictionTier } from "$lib/insights/subjectsTiers";

  interface Props {
    subjectCount: number;
    conclusionCount: number;
    names: Record<ConvictionTier, string[]>;
  }
  let { subjectCount, conclusionCount, names }: Props = $props();

  const head = $derived(subjectsHeadline(names));
  const TILES: [ConvictionTier, string, string][] = [
    ["strong", "Strongly held", "lead ≥ 68%"],
    ["forming", "Forming", "30 – 68%"],
    ["shaping", "Just taking shape", "under 30%"],
    ["fading", "Fading", "kept for history"],
  ];
</script>

<div class="in-read">
  <div class="mx-reveal" style="--i:1">
    <span class="mx-kicker">Your dossier · {subjectCount} subjects · {conclusionCount} conclusions</span>
    <h1 class="mx-display">{head.lead}{#if head.tail}{" "}<em>{head.tail}</em>{/if}</h1>
    <p class="mx-prose">
      What Mnema has come to believe about you. Each subject is scored by its strongest conclusion;
      confidence rises with fresh evidence, halves every 30 days without it, and drops faster when
      activity contradicts it.
    </p>
  </div>
  <div class="in-lede-stats mx-reveal" style="--i:2">
    {#each TILES as [id, label, note] (id)}
      <div class="mx-stat">
        <span class="mx-stat__label">{label}</span>
        <span class="mx-stat__value">{names[id].length}</span>
        <span class="mx-stat__delta">{note}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  /* insights.html `.in-read` / `.in-lede-stats`. */
  .in-read {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-7);
    align-items: end;
    padding: var(--s-4) 0 var(--s-6);
    margin-bottom: var(--s-6);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .in-read .mx-display {
    margin: var(--s-4) 0;
    max-width: 24ch;
  }
  .in-read .mx-display em {
    font-style: normal;
    font-weight: 500;
    color: var(--app-text-subtle);
  }
  .in-lede-stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(132px, auto));
    gap: var(--s-5) var(--s-6);
    padding-bottom: 4px;
  }
</style>
