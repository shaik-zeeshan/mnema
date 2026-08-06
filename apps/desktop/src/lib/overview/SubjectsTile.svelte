<script lang="ts">
  // Subjects the engine currently believes something about. Subjects are derived
  // client-side from `list_user_context_conclusions` — the same shape the
  // Insights Subjects surface uses — so this tile adds no query of its own.
  //
  // The five dots are the belief's confidence, which is a stored number. Nothing
  // here is a trend, a rank, or a score Mnema does not hold.
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";
  import type { LoadState, Selection } from "./overview-data.svelte";
  import TileShell from "./TileShell.svelte";

  interface Props {
    conclusions: LoadState<Conclusion[]>;
    status: LoadState<UserContextStatus | null>;
    selectedKey: string | null;
    /** The 800px floor: two single-line rows, which is what actually fits. */
    compact?: boolean;
    onselect: (selection: Selection) => void;
    /** The tile is the Subjects destination's door (page 09). */
    onopen?: () => void;
  }

  let { conclusions, status, selectedKey, compact = false, onselect, onopen }: Props = $props();

  const all = $derived(conclusions.status === "ok" ? conclusions.value : []);

  // One row per Subject: its most recently supported belief speaks for it.
  const rows = $derived.by(() => {
    const bySubject = new Map<string, Conclusion>();
    for (const c of all) {
      const key = c.subject.toLowerCase();
      const held = bySubject.get(key);
      if (!held || c.lastSupportedAtMs > held.lastSupportedAtMs) bySubject.set(key, c);
    }
    return [...bySubject.values()]
      .sort((a, b) => b.lastSupportedAtMs - a.lastSupportedAtMs)
      .slice(0, compact ? 2 : 3);
  });

  const subjectCount = $derived(
    status.status === "ok" ? (status.value?.subjectCount ?? null) : null,
  );
  // With a door, the header note is the opener — "12 active ›" when the count
  // is real on this machine, a bare "Open" otherwise (G8: no invented number).
  const more = $derived(
    subjectCount === null ? (onopen ? "Open" : undefined) : `${subjectCount} active`,
  );

  const quiet = $derived(
    conclusions.status === "failed"
      ? "Couldn't read what Mnema has concluded."
      : conclusions.status === "loading"
        ? null
        : rows.length === 0
          ? "Nothing concluded yet."
          : null,
  );

  function keyOf(c: Conclusion): string {
    return `subject:${c.id}`;
  }

  /** Confidence as filled dots out of five — the stored 0–1 value, rounded up so
   *  a held belief never draws as zero conviction. */
  function dots(confidence: number): boolean[] {
    const filled = Math.min(5, Math.max(1, Math.ceil(confidence * 5)));
    return Array.from({ length: 5 }, (_, i) => i < filled);
  }

  function toSelection(c: Conclusion): Selection {
    const evidence = c.evidence.length;
    return {
      key: keyOf(c),
      source: "Subjects",
      title: c.subject,
      lede: c.statement,
      sections: [
        {
          label: "Belief",
          rows: [
            { k: "Confidence", v: `${Math.round(c.confidence * 100)}%`, mono: true },
            { k: "Formed", v: new Date(c.formedAtMs).toLocaleDateString(), mono: true },
            { k: "Supported", v: new Date(c.lastSupportedAtMs).toLocaleDateString(), mono: true },
            { k: "Evidence", v: `${evidence} ${evidence === 1 ? "activity" : "activities"}`, mono: true },
            ...(c.replacedStatement ? [{ k: "Replaced", v: c.replacedStatement }] : []),
          ],
        },
      ],
    };
  }
</script>

<TileShell
  label="Subjects"
  {more}
  {onopen}
  span="ss-tile--2 ov-half"
  {quiet}
  selected={rows.some((c) => keyOf(c) === selectedKey)}
>
  {#each rows as c (c.id)}
    <button
      type="button"
      class="ss-row row"
      class:ss-row--sel={selectedKey === keyOf(c)}
      onclick={() => onselect(toSelection(c))}
    >
      <span class="ss-row__txt">
        <span class="ss-row__lbl">{c.subject}</span>
        {#if !compact}<span class="ss-row__sub">{c.statement}</span>{/if}
      </span>
      <span class="ss-row__val">
        <span class="conv" aria-label="{Math.round(c.confidence * 100)}% confidence">
          {#each dots(c.confidence) as on}<i class:on></i>{/each}
        </span>
      </span>
    </button>
  {/each}
</TileShell>

<style>
  .row {
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: default;
    min-height: 0;
    padding: 0 var(--s-10);
  }

  .conv {
    display: inline-flex;
    gap: 2px;
  }

  .conv i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }

  .conv i.on {
    background: var(--app-accent);
  }
</style>
