<!--
  Onboarding's capture line (issue #195, slice 6) — "the sentence".

  Ported from `docs/onboarding/mockups/input-components/parts/sentence.part.html`,
  which is the design of record; behaviour and copy come from it. The arithmetic
  and every string it prints live in `$lib/onboarding/capture-sentence.ts` so
  `bun test` can pin them; this file is the control surface only.

  ONE line replaces three rows and their two duplicates: the capture-rate
  slider, the storage field and the retention group on *Capture & Storage*, plus
  the `RetentionPicker` and rate `Select` on *Change settings*. A user who has
  read the sentence never meets the same two settings again in a different
  control.

  The ghost dial: each token prints its own alternatives faintly around it, as
  many as its ladder can carry — retention prints all four stops (so "everything"
  reads as one of four rather than as a statement), the rate prints a 5-wide
  window clamped at both ends, and the folder prints one, the default you moved
  away from, riding at the very END of the sentence so the full stop still lands
  on the folder.

  Keyboard parity: ghosts are `aria-hidden` and out of the tab order — three
  extra tab stops per token is the wrong trade — but every value a ghost reaches
  is reachable from the token itself. Tab between tokens, ← → step, Page↑/Page↓
  jump ±2 (the outer ghost), Home/End go to the ends, Enter on the folder opens
  the picker. Each token is a `role="spinbutton"` carrying `aria-valuetext`, so a
  screen reader hears the word, not the index.

  Motion: none ambient. The one animation is the probe's indeterminate bar,
  which is real work in flight, and it is dropped under prefers-reduced-motion.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
  } from "$lib/components/capture-rate";
  import {
    DEFAULT_FOLDER_LABEL,
    RATE_GHOST_WIDTH,
    RETENTION_STOPS,
    dailyBytes,
    ghostWindow,
    retentionIndex,
    sentencePath,
    sentenceVerdict,
    type ProbeState,
    type RepairAct,
  } from "$lib/onboarding/capture-sentence";
  import type { StorageProbe } from "$lib/onboarding/gates";
  import { describeError, formatBytes } from "$lib/settings/state/format";
  import type { RetentionPolicy } from "$lib/types";

  let {
    frameRate,
    onFrameRateChange,
    retention,
    onRetentionChange,
    saveDirectory,
    onSaveDirectoryChange,
    probedPath = "",
    probe,
    probeState,
    onRecheck,
    requiredBytes,
    semanticSearchOn = false,
    onDisableSemanticSearch,
    onError,
  }: {
    /** Wire format stays fps; the sentence never says "fps". */
    frameRate: number;
    onFrameRateChange: (fps: number) => void;
    retention: RetentionPolicy;
    onRetentionChange: (policy: RetentionPolicy) => void;
    /** The draft directory. `""` means "the folder the backend resolves". */
    saveDirectory: string;
    onSaveDirectoryChange: (path: string) => void;
    /** The path `probe_storage_path` actually measured, so a blank draft still
     *  renders a real folder. */
    probedPath?: string;
    probe: StorageProbe | null;
    /** "the probe failed" renders differently from "not probed yet". */
    probeState: ProbeState;
    onRecheck: () => void;
    /** `flow.downloadBytes` — what the downloads will fetch. */
    requiredBytes: number;
    semanticSearchOn?: boolean;
    /** Omit and the "turn Semantic Search off" escape is not offered. */
    onDisableSemanticSearch?: () => void;
    onError?: (message: string) => void;
  } = $props();

  // ── The two ladders ───────────────────────────────────────────────────────
  const LADDER = CAPTURE_INTERVAL_LADDER_S;
  const rateIndex = $derived(nearestLadderIndex(frameRate));
  const intervalS = $derived(LADDER[rateIndex]!);
  const keepIndex = $derived(retentionIndex(retention));

  // ── Hover preview ─────────────────────────────────────────────────────────
  // Hovering a ghost moves the consequence before you commit to it. Mouse only:
  // ghosts are not focusable, and a keyboard step commits outright.
  let peek = $state<{ rate?: number; keep?: RetentionPolicy } | null>(null);
  const viewInterval = $derived(
    peek?.rate === undefined ? intervalS : LADDER[peek.rate]!,
  );
  const viewRetention = $derived(peek?.keep ?? retention);

  const shownPath = $derived(saveDirectory.trim() || probedPath);
  const verdict = $derived(
    sentenceVerdict({
      intervalSeconds: viewInterval,
      retention: viewRetention,
      path: shownPath,
      probe,
      probeState,
      requiredBytes,
      semanticSearchOn,
    }),
  );

  // ── The dials ─────────────────────────────────────────────────────────────
  interface DialSpec {
    kind: "rate" | "keep";
    label: string;
    aria: string;
    index: number;
    count: number;
    width: number;
    textAt: (i: number) => string;
    titleAt: (i: number) => string;
    peekAt: (i: number) => { rate?: number; keep?: RetentionPolicy };
    commit: (i: number) => void;
  }

  const shortRate = (s: number) => (s < 1 ? `${Math.round(1 / s)}/s` : `${s}s`);

  const rateDial = $derived<DialSpec>({
    kind: "rate",
    label: captureIntervalPhrase(intervalS),
    aria: "How often to take a snapshot",
    index: rateIndex,
    count: LADDER.length,
    width: RATE_GHOST_WIDTH,
    textAt: (i) => shortRate(LADDER[i]!),
    titleAt: (i) =>
      `${captureIntervalPhrase(LADDER[i]!)} — ${formatBytes(dailyBytes(LADDER[i]!))} a day`,
    peekAt: (i) => ({ rate: i }),
    commit: (i) => onFrameRateChange(intervalSToFps(LADDER[i]!)),
  });

  const keepDial = $derived<DialSpec>({
    kind: "keep",
    label: RETENTION_STOPS[keepIndex]!.word,
    aria: "How long to keep it",
    index: keepIndex,
    count: RETENTION_STOPS.length,
    width: RETENTION_STOPS.length,
    textAt: (i) => RETENTION_STOPS[i]!.short,
    titleAt: (i) => `keep ${RETENTION_STOPS[i]!.word}`,
    peekAt: (i) => ({ keep: RETENTION_STOPS[i]!.id }),
    commit: (i) => onRetentionChange(RETENTION_STOPS[i]!.id),
  });

  /**
   * The ghosts on either side of the token. Split rather than one list with the
   * token inside it: stepping shifts the window, and if the token were an entry
   * in a keyed `{#each}` it would be destroyed and recreated on every step, so
   * keyboard focus would fall out of the control after one arrow press.
   */
  function ghosts(d: DialSpec, side: "before" | "after"): number[] {
    const win = ghostWindow(d.index, d.count, d.width);
    const from = side === "before" ? win.start : d.index + 1;
    const to = side === "before" ? d.index : win.end;
    const out: number[] = [];
    for (let i = from; i < to; i++) out.push(i);
    return out;
  }

  function stepKey(d: DialSpec, event: KeyboardEvent) {
    const last = d.count - 1;
    const clamp = (i: number) => Math.min(Math.max(i, 0), last);
    const to =
      event.key === "ArrowRight" || event.key === "ArrowUp"
        ? clamp(d.index + 1)
        : event.key === "ArrowLeft" || event.key === "ArrowDown"
          ? clamp(d.index - 1)
          : event.key === "PageUp"
            ? clamp(d.index + 2)
            : event.key === "PageDown"
              ? clamp(d.index - 2)
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? last
                  : null;
    if (to === null) return;
    event.preventDefault();
    peek = null;
    d.commit(to);
  }

  // ── The folder ────────────────────────────────────────────────────────────
  let browsing = $state(false);
  /**
   * Whether the undo ghost has anywhere to go. NOT `saveDirectory !== ""`: the
   * settings round-trip fills the draft with the RESOLVED default path, so a
   * user who has moved nowhere still carries a non-empty draft and the ghost
   * would print "↩ ~/.mnema" immediately after "in ~/.mnema.". Compare what the
   * sentence actually shows instead — that is the claim the undo makes.
   */
  const movedAway = $derived(sentencePath(shownPath) !== DEFAULT_FOLDER_LABEL);

  async function browse() {
    if (browsing) return;
    browsing = true;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose where Mnema stores captures",
        defaultPath: saveDirectory.trim() || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        onSaveDirectoryChange(picked);
      }
    } catch (err) {
      onError?.(`Couldn't open the folder picker: ${describeError(err)}`);
    } finally {
      browsing = false;
    }
  }

  // ── Repairs ───────────────────────────────────────────────────────────────
  // The escape offered under each failure. `nosemantic` is dropped when the
  // caller has nothing to turn off, so a dead button is never rendered.
  const repairs = $derived(
    verdict.repairs.filter(
      (r) => r.act !== "nosemantic" || (semanticSearchOn && onDisableSemanticSearch),
    ),
  );

  function act(action: RepairAct) {
    peek = null;
    if (action === "default") return onSaveDirectoryChange("");
    if (action === "pick") return void browse();
    if (action === "recheck") return onRecheck();
    if (action === "nosemantic") return onDisableSemanticSearch?.();
    if (action === "slower") {
      return onFrameRateChange(
        intervalSToFps(LADDER[Math.min(rateIndex + 2, LADDER.length - 1)]!),
      );
    }
    if (action === "tighten") {
      return onRetentionChange(
        RETENTION_STOPS[Math.min(keepIndex + 1, RETENTION_STOPS.length - 1)]!.id,
      );
    }
    onRetentionChange("days_30");
  }
</script>

<!-- Decoration to a screen reader on purpose: every value a ghost reaches is
     also reachable from the token with ← → / Page↑ Page↓ / Home / End. -->
{#snippet ghost(d: DialSpec, i: number)}
  <button
    type="button"
    class="ghost"
    class:near={Math.abs(i - d.index) === 1}
    aria-hidden="true"
    tabindex="-1"
    title={d.titleAt(i)}
    onclick={(event) => {
      event.stopPropagation();
      peek = null;
      d.commit(i);
    }}
    onmouseenter={() => (peek = d.peekAt(i))}
    onmouseleave={() => (peek = null)}
  >
    {d.textAt(i)}
  </button>
{/snippet}

{#snippet dial(d: DialSpec)}
  <span class="dial">
    {#each ghosts(d, "before") as i (i)}{@render ghost(d, i)}{/each}
    <button
      type="button"
      class="tok"
      role="spinbutton"
      aria-label={d.aria}
      aria-valuemin={1}
      aria-valuemax={d.count}
      aria-valuenow={d.index + 1}
      aria-valuetext={d.label}
      onclick={() => {
        peek = null;
        d.commit((d.index + 1) % d.count);
      }}
      onkeydown={(event) => stepKey(d, event)}
    >
      {d.label}
    </button>
    {#each ghosts(d, "after") as i (i)}{@render ghost(d, i)}{/each}
  </span>
{/snippet}

<p class="sentence">
  Take a snapshot <span class="nb">{@render dial(rateDial)},</span>
  keep <span class="nb">{@render dial(keepDial)},</span>
  in <span class="nb"
    ><button
      type="button"
      class="tok place"
      class:bad={verdict.clause !== null}
      title={shownPath}
      aria-label="Where it lives: {shownPath}. Opens a folder picker."
      disabled={browsing}
      onclick={browse}>{sentencePath(shownPath) || "…"}</button
    >{#if verdict.probing}…{:else if verdict.clause}<span class="dash">{" —"}</span
      >{:else}.{/if}</span
  >{#if verdict.clause}<span class="brk">{` ${verdict.clause}`}</span>{/if}
  {#if movedAway}
    <!-- The one place ghost: the default you moved away from. It rides at the
         END of the sentence — it is an undo, not a ladder neighbour, and a
         broken sentence gets to finish its clause before the undo appears.
         ponytail: it commits instead of previewing; a preview would need a
         probe of a path you have not chosen. -->
    <button
      type="button"
      class="ghost undo"
      aria-hidden="true"
      tabindex="-1"
      title="Back to the default folder"
      onclick={() => onSaveDirectoryChange("")}
    >
      {DEFAULT_FOLDER_LABEL}
    </button>
  {/if}
</p>

<div class="outcome {verdict.tone}" class:peek={peek !== null} aria-live="polite">
  <span class="plan">
    {#if peek}<span class="peek-tag">if</span>{/if}
    {#each verdict.plan as seg, i (i)}<span class={seg.kind ?? ""}>{seg.text}</span
      >{/each}
  </span>
  {#if verdict.probing}
    <div class="probing"><i></i></div>
  {:else}
    {#each verdict.verdict as seg, i (i)}<span class={seg.kind ?? ""}>{seg.text}</span
      >{/each}
  {/if}
</div>

{#if !peek && repairs.length > 0}
  <div class="repair">
    {#each repairs as r (r.act)}
      <button class="ob-btn sm" class:primary={r.primary} onclick={() => act(r.act)}>
        {r.label}
      </button>
    {/each}
  </div>
{/if}

<p class="hint">
  click a word to step · click a faint word to jump · ← → step · Home / End ends ·
  Enter on the folder opens the picker
</p>

<style>
  .sentence {
    margin: 0;
    font-size: var(--text-xl);
    line-height: 2.1;
    color: var(--app-text);
    overflow-wrap: break-word;
  }
  /* Each token is glued to the punctuation after it, so a wrap never starts a
     line with an orphan comma; the break clause is free to wrap. */
  .nb {
    white-space: nowrap;
  }
  .brk {
    color: var(--app-danger);
  }
  .dash {
    color: var(--app-text-faint);
  }

  /* Every token is the same word-shaped thing, whatever it edits. */
  .tok {
    font: inherit;
    font-size: inherit;
    line-height: inherit;
    color: var(--app-text-strong);
    background: transparent;
    border: 0;
    border-bottom: 1px solid var(--app-border-hover);
    padding: 0 2px 1px;
    margin: 0;
    cursor: pointer;
    border-radius: 0;
    -webkit-appearance: none;
    appearance: none;
  }
  .tok:hover {
    border-color: var(--app-accent);
  }
  .tok:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-color: var(--app-accent);
  }
  .tok.bad {
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }
  /* The folder mark keeps the place token from reading as a word-menu. */
  .tok.place::before {
    content: "▸";
    color: var(--app-text-faint);
    font-size: 0.62em;
    vertical-align: 0.22em;
    margin-right: 6px;
  }
  .tok.place:hover::before,
  .tok.place:focus-visible::before {
    color: var(--app-accent);
  }

  /* ---- the ghost dial ---- */
  .dial {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 5px;
    vertical-align: baseline;
  }
  .ghost {
    font: inherit;
    font-size: var(--text-md);
    color: var(--app-text-faint);
    background: transparent;
    border: 0;
    padding: 0 1px;
    cursor: pointer;
    white-space: nowrap;
    border-radius: 4px;
  }
  .ghost:hover {
    color: var(--app-text-subtle);
  }
  .ghost.near {
    color: var(--app-text-subtle);
  }
  .ghost.near:hover {
    color: var(--app-text-muted);
  }
  .ghost.undo {
    color: var(--app-text-subtle);
    margin-left: 10px;
  }
  .ghost.undo::before {
    content: "↩ ";
    color: var(--app-text-faint);
  }

  /* ---- the consequence ---- */
  .outcome {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--app-border);
    font-size: var(--text-md);
    line-height: 1.85;
    color: var(--app-text-muted);
  }
  .outcome :global(.fig) {
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .outcome :global(.verdict) {
    color: var(--app-text-strong);
  }
  .outcome.ok :global(.verdict) {
    color: var(--app-accent);
  }
  .outcome.warn :global(.verdict) {
    color: var(--app-warn);
  }
  .outcome.bad :global(.verdict) {
    color: var(--app-danger);
  }
  .outcome .plan {
    display: block;
  }
  .outcome.peek {
    border-left: 2px solid var(--app-accent-border);
    margin-left: -14px;
    padding-left: 12px;
  }
  .outcome.peek .plan {
    color: var(--app-text);
  }
  .peek-tag {
    color: var(--app-accent);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    font-size: var(--text-xs);
    margin-right: 8px;
  }
  .repair {
    margin-top: 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .hint {
    margin: 14px 0 0;
    font-size: var(--text-xs);
    color: var(--app-text-faint);
    letter-spacing: 0.06em;
  }

  /* Real work in flight, not ambient motion — it exists only while a probe is
     outstanding, and reduced motion drops it to a static bar. */
  .probing {
    height: 2px;
    background: var(--app-border);
    border-radius: 2px;
    overflow: hidden;
    max-width: 220px;
    margin-top: 10px;
  }
  .probing i {
    display: block;
    height: 100%;
    width: 35%;
    background: var(--app-accent-strong);
    border-radius: 2px;
    animation: capture-sentence-slide 900ms linear infinite;
  }
  @keyframes capture-sentence-slide {
    from {
      transform: translateX(-110%);
    }
    to {
      transform: translateX(320%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .probing i {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
  }
</style>
