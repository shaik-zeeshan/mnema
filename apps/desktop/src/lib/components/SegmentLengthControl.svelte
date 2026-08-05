<script lang="ts">
  // ══ INSTRUMENT 3 of 6 — SEGMENT LENGTH ════════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". This setting earns an instrument
  // because it passes the rule: the physical quantity is *how long a segment
  // runs before it is finalised*, and the consequence is exact and real — a
  // segment is only readable once it has been finalised, so an unexpected quit
  // costs you up to one segment of the most recent recording. The denominator
  // is the segment itself; no measurement is needed, which is why this readout
  // is honest even on a machine that has never captured anything.
  //
  // Anatomy: header (name + live value) → WELL (the ladder) → READOUT.
  //
  // The 5-minute cap is a hard app invariant (Capture Segment Duration is
  // capped at 5 minutes), so the ladder draws the next stop up DISABLED rather
  // than omitting it — a ceiling you can see is a ceiling you stop asking about.

  interface Props {
    /** Segment duration in seconds (bindable), as the backend stores it. */
    value: number;
    disabled?: boolean;
  }

  let { value = $bindable(), disabled = false }: Props = $props();

  // Seconds. The last entry is above the invariant cap and is drawn disabled.
  interface Stop {
    s: number;
    label: string;
    /** Above the 5-minute invariant cap: shown, never selectable. */
    over?: boolean;
  }
  const STOPS: Stop[] = [
    { s: 30, label: "30 s" },
    { s: 60, label: "1 min" },
    { s: 120, label: "2 min" },
    { s: 300, label: "5 min" },
    { s: 600, label: "10 min", over: true },
  ];

  // A persisted value need not be a ladder stop (the control this replaced was
  // a free 10–300s slider). Show the nearest stop rather than nothing — but
  // never write on mount, so an existing setting is not silently rounded.
  const activeIdx = $derived(
    STOPS.reduce(
      (best, stop, i) =>
        Math.abs(stop.s - value) < Math.abs(STOPS[best].s - value) ? i : best,
      0,
    ),
  );
  const shown = $derived(STOPS[activeIdx]);

  function select(i: number) {
    const stop = STOPS[i];
    if (disabled || stop.over) return;
    value = stop.s;
  }

  function onKeydown(event: KeyboardEvent, index: number) {
    let next: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") next = index + 1;
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = index - 1;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = STOPS.findIndex((s) => s.over) - 1;
    if (next === null) return;
    event.preventDefault();
    // Clamp inside the selectable range — the over-cap stop is not reachable.
    const last = STOPS.findIndex((s) => s.over) - 1;
    const target = Math.min(Math.max(next, 0), last);
    select(target);
    document.getElementById(`segment-stop-${target}`)?.focus();
  }
</script>

<div class="ti-instr ti-instr--bare segment-instr">
  <div class="ti-instr__hd">
    <span class="ti-instr__name">Segment length</span>
    <span class="ti-instr__sub">how much an unexpected quit can cost you</span>
    <span class="ti-instr__v">
      {shown.s >= 60 ? shown.s / 60 : shown.s}<em>{shown.s >= 60 ? "min" : "sec"}</em>
    </span>
  </div>

  <div class="ti-well ti-ladder" role="radiogroup" aria-label="Segment length">
    <div class="ti-ladder__stops">
      {#each STOPS as stop, i (stop.s)}
        <button
          type="button"
          id="segment-stop-{i}"
          class="ti-ladder__s"
          class:is-on={i === activeIdx && !stop.over}
          role="radio"
          aria-checked={i === activeIdx && !stop.over}
          disabled={disabled || stop.over}
          tabindex={i === activeIdx ? 0 : -1}
          onclick={() => select(i)}
          onkeydown={(e) => onKeydown(e, i)}
        >
          {stop.label}
        </button>
      {/each}
    </div>
  </div>

  <div class="ti-instr__out">
    A segment is only readable once it is finalised, so an unexpected quit or a
    power loss can cost you the <b>{shown.label}</b> in progress — and nothing
    older. Longer segments mean fewer files; shorter ones mean less to lose.
    Mnema caps this at <b>5 min</b>.
  </div>
</div>

<style>
  .segment-instr {
    width: 100%;
    min-width: 0;
  }
</style>
