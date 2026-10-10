<!--
  DateInput — the kit's scrubbable date capsule + calendar popover (SHELL.md › Date input).
  mode "day": one day · "period": one whole day | week | month with a D·W·M switch
  (Insights Overview) · "range": presets + drag-select (Chat scope only).
  Step: drag the capsule (right = earlier), wheel, ←/→, Home/End = latest; click/Enter = popover.
  `value` is an ISO day (`2026-10-09`; any day inside the period) or `start..end` for range.
  `density` maps ISO day → 0..3 captured (see `densityFromHeatmap`); missing = nothing captured.
  `onchange({start, end, unit})` fires on every step / commit / unit switch.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { tip } from "./tooltip";
  import {
    addDays, cap, capsuleLabel, daysBetween, fmt, isoWeek, latestSpan, monday, monthOf, parseIso,
    periodOf, periodsAgo, presets, relativeLabel, resetLabel, spanLabel, stepSpan, switchUnit, toIso,
    unitOf, type DateRange, type Mode, type PeriodUnit, type Unit,
  } from "./date-input";

  interface Props {
    mode?: Mode;
    /** Period mode's starting unit. */
    unit?: PeriodUnit;
    value?: string;
    /** Capture start; default today − 120 days. */
    min?: string;
    /** Default today: nothing steps past it. */
    max?: string;
    today?: string;
    /** "end" right-aligns the popover under the capsule. */
    align?: "start" | "end";
    density?: Record<string, number>;
    onchange?: (range: DateRange) => void;
  }
  let {
    mode = "day", unit: unitProp = "week", value, min: minProp, max: maxProp, today: todayProp,
    align = "start", density, onchange,
  }: Props = $props();

  const UNITS: PeriodUnit[] = ["day", "week", "month"];
  const PITCH = 7; // ruler: tick 2px + gap 5px
  const WEEKDAYS = ["M", "T", "W", "T", "F", "S", "S"];

  const today = $derived(parseIso(todayProp ?? toIso(new Date())));
  const min = $derived(minProp ? parseIso(minProp) : addDays(today, -120));
  const max = $derived(maxProp ? parseIso(maxProp) : today);
  const period = $derived(mode === "period");
  const range = $derived(mode === "range");

  function seed(): [Date, Date, Unit] {
    const [s, e = s] = (value ?? toIso(today)).split("..").map(parseIso);
    if (mode === "period") return [...periodOf(s, unitProp), unitProp];
    return mode === "range" ? [s, e, unitOf(s, e)] : [s, s, "day"];
  }
  const seeded = untrack(seed);
  let a = $state(seeded[0]);
  let b = $state(seeded[1]);
  let unit = $state(seeded[2]);
  // Re-seed when the consumer hands in a new value / unit / mode.
  $effect.pre(() => {
    void [value, unitProp, mode];
    untrack(() => ([a, b, unit] = seed()));
  });

  // ── density
  const heat = (d: Date) => (d < min || d > max ? -1 : (density?.[toIso(d)] ?? 0));
  function periodHeat(s: Date, u: Unit): number { // mean density of the captured days in the period
    let t = 0, k = 0;
    for (let d = s, e = periodOf(s, u)[1]; d <= e; d = addDays(d, 1)) {
      const h = heat(d);
      if (h >= 0) { t += h; k++; }
    }
    return k ? Math.round(t / k) : -1;
  }

  // ── ruler: one tick per day (per period in period mode), height = density
  const tickUnit = $derived(period ? unit : "day");
  const starts = $derived.by(() => {
    const out: Date[] = [];
    for (let d = periodOf(min, tickUnit)[0]; d <= max; d = tickUnit === "month" ? monthOf(d, 1)[0] : addDays(d, tickUnit === "week" ? 7 : 1)) out.push(d);
    return out;
  });
  const ticks = $derived(starts.map((s) => (period ? periodHeat(s, tickUnit) : heat(s))));
  const sel = $derived.by(() => {
    if (!period) return [daysBetween(min, a), daysBetween(min, b)];
    const k = starts.findIndex((s) => +s === +a);
    return [k, k];
  });
  let rulerW = $state(84);
  const tapeX = $derived((rulerW || 84) / 2 - (((sel[0] + sel[1]) / 2) * PITCH + 1));

  const label = $derived(capsuleLabel(mode, unit, a, b, today));
  const rel = $derived(relativeLabel(mode, unit, a, b, today));
  const atNow = $derived(period && periodsAgo(a, unit, today) === 0);

  // ── stepping
  function apply(span: [Date, Date] | null, u: Unit = unit): boolean {
    if (!span) return false;
    [a, b] = span;
    unit = u;
    if (open) view = monthOf(a)[0];
    onchange?.({ start: toIso(a), end: toIso(b), unit });
    return true;
  }
  const step = (k: number) => apply(stepSpan(a, b, unit, k, min, max));
  const latest = () => apply(latestSpan(a, b, unit, max));
  function setUnit(u: PeriodUnit) {
    if (period && u !== unit) apply(switchUnit(a, unit, u, today, min), u);
  }

  // ── drag / wheel / keys on the capsule
  let btn: HTMLButtonElement;
  let host: HTMLDivElement;
  let dragging = $state(false);
  let drag: { x: number; applied: number } | null = null;
  let dragged = false;
  const stepPx = (u: Unit) => (period || u === "day" ? PITCH : u === "month" ? 36 : 24);

  function onPointerDown(e: PointerEvent) {
    if (e.button) return;
    drag = { x: e.clientX, applied: 0 };
    dragged = false;
    btn.setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.x;
    if (!dragged && Math.abs(dx) > 3) { dragged = dragging = true; open = false; }
    const want = -Math.trunc(dx / stepPx(unit)); // dragging the tape right pulls earlier days under the notch
    while (drag.applied !== want) {
      const dir = Math.sign(want - drag.applied);
      if (!step(dir)) { drag.x = e.clientX + drag.applied * stepPx(unit); break; } // pinned at an edge: no debt
      drag.applied += dir;
    }
  }
  function endDrag() { drag = null; dragging = false; }
  function onCapsuleClick() {
    if (dragged) { dragged = false; return; } // a drag is not a click
    if (!open) { view = monthOf(a)[0]; anchor = null; }
    open = !open;
  }
  let wheelAcc = 0;
  function onWheel(e: WheelEvent) {
    if (pop.contains(e.target as Node)) return;
    e.preventDefault();
    wheelAcc += Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    if (Math.abs(wheelAcc) >= 40) { step(Math.sign(wheelAcc)); wheelAcc = 0; }
  }
  function onKey(e: KeyboardEvent) {
    const k = ({ ArrowLeft: -1, ArrowRight: 1, ArrowDown: -1, ArrowUp: 1 } as Record<string, number>)[e.key];
    if (k && !e.metaKey) { e.preventDefault(); step(k); }
    else if (e.key === "Home" || e.key === "End") { e.preventDefault(); latest(); }
  }

  // ── popover
  let pop: HTMLDivElement;
  let open = $state(false);
  let view = $state(monthOf(untrack(() => a))[0]);
  let anchor = $state<Date | null>(null);
  let hover = $state<Date | null>(null);
  let selecting = false;
  const weeks = $derived(period && unit === "week");
  const months = $derived(period && unit === "month");
  const pending = $derived(anchor ? ([anchor, hover ?? anchor].sort((x, y) => +x - +y) as [Date, Date]) : [a, b]);
  const cells = $derived.by(() => {
    const start = monday(view);
    const n = Math.ceil((daysBetween(start, view) + monthOf(view)[1].getDate()) / 7) * 7; // only weeks touching the month
    return Array.from({ length: n }, (_, i) => addDays(start, i));
  });
  const rows = $derived(Array.from({ length: cells.length / 7 }, (_, r) => cells.slice(r * 7, r * 7 + 7)));
  const yearMonths = $derived(Array.from({ length: 12 }, (_, m) => new Date(view.getFullYear(), m, 1, 12)));

  function commit(na: Date, nb: Date, u: Unit) {
    anchor = null;
    selecting = false;
    open = false;
    apply([na, nb], u);
    btn.focus();
  }
  function nav(k: number) {
    view = months ? new Date(view.getFullYear() + k, view.getMonth(), 1, 12) : monthOf(view, k)[0];
  }
  function rangeDown(d: Date) {
    if (anchor && !selecting) {
      const [lo, hi] = [anchor, d].sort((x, y) => +x - +y);
      return commit(lo, hi, unitOf(lo, hi));
    }
    anchor = hover = d;
    selecting = true;
  }
  function onWindowPointerUp() {
    endDrag();
    if (!selecting) return;
    selecting = false;
    if (anchor && hover && +hover !== +anchor) commit(pending[0], pending[1], unitOf(pending[0], pending[1]));
  }
  function onWindowPointerDown(e: PointerEvent) {
    if (open && !host.contains(e.target as Node)) open = false;
  }
  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) { open = false; anchor = null; }
  }
  const dayAttrs = (d: Date) => ({
    "data-out": d.getMonth() !== view.getMonth() || undefined,
    "data-today": +d === +today || undefined,
  });
  const inPending = (d: Date) => range && +d >= +pending[0] && +d <= +pending[1];
</script>

<svelte:window onpointerdown={onWindowPointerDown} onpointerup={onWindowPointerUp} onkeydown={onWindowKey} />

<div class="mx-date" bind:this={host} data-mode={mode} data-align={align} data-dragging={dragging || undefined} onwheel={onWheel}>
  <button
    type="button" class="mx-date__btn" bind:this={btn} aria-haspopup="dialog" aria-expanded={open}
    aria-label="{range ? 'Range' : period ? cap(unit) : 'Day'}: {label}, {rel}. Arrow keys or drag to step, Enter for calendar."
    onpointerdown={onPointerDown} onpointermove={onPointerMove} onpointercancel={endDrag} onclick={onCapsuleClick} onkeydown={onKey}
  >
    <svg class="mx-date__icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M16 3v4M8 3v4M3 10h18" /></svg>
    <span class="mx-date__text"><b>{label}</b><small>{rel}</small></span>
    <span class="mx-date__ruler" aria-hidden="true" bind:clientWidth={rulerW}>
      <span class="mx-date__tape" style:transform="translateX({tapeX}px)">
        {#each ticks as h, i (i)}<i data-h={h} data-sel={(i >= sel[0] && i <= sel[1]) || undefined}></i>{/each}
      </span>
    </span>
  </button>
  {#if period}
    <span class="mx-date__units" role="group" aria-label="Period">
      {#each UNITS as u (u)}
        <button type="button" aria-pressed={u === unit} aria-label={cap(u)} use:tip={cap(u)} onclick={() => setUnit(u)}>{u[0].toUpperCase()}</button>
      {/each}
    </span>
  {/if}

  <div class="mx-pop mx-date__pop" bind:this={pop} data-open={open || undefined} inert={!open} role="dialog" aria-label="Choose {range ? 'a range' : period ? 'a period' : 'a day'}">
    {#if range}
      <div class="mx-date__presets">
        {#each presets(today) as [l, s, e, u] (l)}
          <button type="button" aria-pressed={+s === +a && +e === +b} onclick={() => commit(s, e, u)}><span>{l}</span><small>{spanLabel(s, e)}</small></button>
        {/each}
      </div>
    {/if}
    {#if period}
      <div class="mx-date__units" role="group" aria-label="Period">
        {#each UNITS as u (u)}<button type="button" aria-pressed={u === unit} onclick={() => setUnit(u)}>{cap(u)}</button>{/each}
      </div>
    {/if}
    <div class="mx-cal" class:mx-cal--weeks={weeks}>
      <div class="mx-cal__head">
        {#if months}
          <span class="mx-cal__month">{view.getFullYear()}</span>
          {@render navBtn(-1, view.getFullYear() <= min.getFullYear())}
          {@render navBtn(1, view.getFullYear() >= max.getFullYear())}
        {:else}
          <span class="mx-cal__month">{fmt(view, { month: "long", year: "numeric" })}</span>
          {@render navBtn(-1, view <= monthOf(min)[0])}
          {@render navBtn(1, view >= monthOf(max)[0])}
        {/if}
      </div>
      {#if months}
        <div class="mx-cal__months">
          {#each yearMonths as m0 (+m0)}
            {@const h = periodHeat(m0, "month")}
            <button
              type="button" class="mx-cal__day mx-cal__mo" disabled={h < 0} data-sel={+m0 === +a || undefined}
              data-today={(m0.getMonth() === today.getMonth() && m0.getFullYear() === today.getFullYear()) || undefined}
              aria-label={fmt(m0, { month: "long", year: "numeric" })} onclick={() => commit(...monthOf(m0), "month")}
            ><span>{fmt(m0, { month: "short" })}</span><i data-h={Math.max(h, 0)}></i></button>
          {/each}
        </div>
      {:else}
        <div class="mx-cal__grid">
          {#if weeks}<span class="mx-cal__wd">wk</span>{/if}
          {#each WEEKDAYS as w, i (i)}<span class="mx-cal__wd">{w}</span>{/each}
          {#if weeks}
            {#each rows as row (+row[0])}
              <button
                type="button" class="mx-cal__week" data-sel={+row[0] === +a || undefined} disabled={row[0] > max || row[6] < min}
                aria-label="Week {isoWeek(row[0])}, {spanLabel(row[0], row[6])}" onclick={() => commit(...periodOf(row[0], "week"), "week")}
              >
                <span class="mx-cal__wn">{isoWeek(row[0])}</span>
                {#each row as d (+d)}
                  {@const h = heat(d)}
                  <span class="mx-cal__day" {...dayAttrs(d)} data-off={h < 0 || undefined}><span>{d.getDate()}</span><i data-h={Math.max(h, 0)}></i></span>
                {/each}
              </button>
            {/each}
          {:else}
            {#each cells as d (+d)}
              {@const h = heat(d)}
              <button
                type="button" class="mx-cal__day" {...dayAttrs(d)} disabled={h < 0}
                data-sel={(!range && +d === +a) || undefined} data-in={inPending(d) || undefined}
                data-start={(range && +d === +pending[0]) || undefined} data-end={(range && +d === +pending[1]) || undefined}
                aria-label={fmt(d, { weekday: "long", month: "long", day: "numeric" })}
                onclick={() => !range && commit(d, d, "day")}
                onpointerdown={() => range && h >= 0 && rangeDown(d)}
                onpointerover={() => { if (range && anchor && h >= 0) hover = d; }}
              ><span>{d.getDate()}</span><i data-h={Math.max(h, 0)}></i></button>
            {/each}
          {/if}
        </div>
      {/if}
      <div class="mx-cal__foot">
        <span class="mx-cal__legend"><i></i><i></i><i></i>captured</span>
        {#if range}
          <span>{anchor ? "pick an end day" : "drag across days"}</span>
        {:else}
          <button
            type="button" class="mx-btn mx-btn--ghost mx-btn--sm" disabled={atNow}
            onclick={() => (period ? commit(...periodOf(max, unit), unit) : commit(max, max, "day"))}
          >{period ? resetLabel(unit) : "Today"}</button>
        {/if}
      </div>
    </div>
  </div>
</div>

{#snippet navBtn(k: number, disabled: boolean)}
  <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" {disabled} aria-label="{k < 0 ? 'Previous' : 'Next'} {months ? 'year' : 'month'}" onclick={() => nav(k)}>
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d={k < 0 ? "m15 6-6 6 6 6" : "m9 6 6 6-6 6"} /></svg>
  </button>
{/snippet}
