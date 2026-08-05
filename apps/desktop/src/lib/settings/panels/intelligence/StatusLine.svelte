<script lang="ts">
  // A model / runtime status line — the QUIET half of this direction.
  //
  // It replaces the bordered `.model-status` block that every Intelligence
  // panel used to draw. Six bordered cards on one page blows direction 05's
  // ceiling (one bordered container per window — the window ring), and a card
  // around a status sentence is exactly the "depth as a container border" this
  // direction bans. Here depth is nothing at all: a label, a machine sub-line,
  // and one chip. State is carried by the chip, not by a tinted frame.
  //
  // The chip is accent when things are working and neutral otherwise — warn
  // and danger are reserved for the model picker's fit verdicts, which are the
  // only place on this page where a real limit is being crossed.

  interface Props {
    title: string;
    /** The machine sub-line: counts, model ids, last-run times. */
    meta?: string;
    ok?: boolean;
    okLabel?: string;
    offLabel?: string;
  }

  let {
    title,
    meta,
    ok = false,
    okLabel = "available",
    offLabel = "unavailable",
  }: Props = $props();
</script>

<div class="statusline">
  <span class="statusline__txt">
    <span class="ti-grow__lbl">{title}</span>
    {#if meta}<span class="ti-grow__sub">{meta}</span>{/if}
  </span>
  <span class="ti-chip" class:ti-chip--acc={ok}>{ok ? okLabel : offLabel}</span>
</div>

<style>
  .statusline {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    width: 100%;
    min-width: 0;
  }

  .statusline__txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
</style>
