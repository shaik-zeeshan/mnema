<!--
  Screen 1 / 8 — Welcome (issue #195, slice 10; redesign ON2).

  One button, one sentence, and NO "use recommended defaults" — every downstream
  choice is already answered by the resolver.

  Motion lives here and on the Finale, nowhere else, and it plays ONCE: a
  filmstrip of captured screens rewinds under a fixed playhead and settles on
  one frame, the timecode counting back with it, and that frame gives up the
  words it was carrying. Reduced motion lands on the settled frame directly.
  Skin of record: `docs/mockups/redesign/onboarding.html` (Welcome).

  The timecode is derived from the real configured capture interval, so "one
  frame back" is the actual default rate.
-->
<script lang="ts">
  import { formatClock } from "$lib/time-format";
  import { fpsToIntervalS } from "$lib/components/capture-rate";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";
  import IconArrow from "~icons/lucide/arrow-right";

  let {
    flow,
    onContinue,
  }: { flow: OnboardingFlow; onContinue: () => void } = $props();

  // Eight wireframe tiles, repeated along a 30-frame strip; frame HIT is the
  // one the rewind settles on.
  const TILES: readonly (readonly number[])[] = [
    [71, 60, 53, 73, 90],
    [55, 37, 66, 74, 90],
    [46, 90, 38, 51, 46],
    [90, 60, 86, 39, 78],
    [55, 76, 59, 81, 44],
    [66, 55, 43, 53, 36],
    [48, 65, 77, 48, 85],
    [41, 34, 37, 90, 79],
  ];
  const FRAMES = Array.from({ length: 30 }, (_, k) => TILES[k % TILES.length]!);
  const HIT = 19;
  const PITCH = 144; // 132px tile + 12px gap
  const TRAVEL = 9; // frames rewound

  const intervalMs = $derived(fpsToIntervalS(flow.controller.draftFrameRate) * 1000);
  const anchor = Date.now();

  let film = $state<HTMLDivElement | null>(null);
  let strip = $state<HTMLDivElement | null>(null);
  let x = $state(0);
  let end = $state(0);
  let settled = $state(false);

  // Frames behind the hit, from the strip's offset: the clock runs backwards
  // as the strip rewinds and reads the hit frame's time once it settles.
  const timecode = $derived(
    end === 0
      ? "--:--:--"
      : formatClock(anchor - (8 - Math.round((end - x) / PITCH)) * intervalMs, { seconds: true }),
  );

  $effect(() => {
    if (!film || !strip) return;
    end = film.clientWidth / 2 - (HIT * PITCH + 66);
    const settle = () => {
      x = end;
      settled = true;
    };
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      settle();
      return;
    }
    const start = end - TRAVEL * PITCH;
    x = start;
    const anim = strip.animate(
      [{ transform: `translateX(${start}px)` }, { transform: `translateX(${end}px)` }],
      { duration: 2600, easing: "cubic-bezier(0.16, 1, 0.3, 1)", fill: "forwards", delay: 250 },
    );
    let raf = 0;
    const loop = () => {
      if (strip) x = new DOMMatrix(getComputedStyle(strip).transform).m41;
      if (anim.playState !== "finished") raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    anim.finished.then(settle, () => {});
    return () => {
      cancelAnimationFrame(raf);
      anim.cancel();
    };
  });
</script>

<div class="ob-main welcome" class:settled>
  <div class="hero">
    <span class="mx-kicker">Mnema</span>
    <h1 class="hero__title">Your memory,<br /><span>on rewind.</span></h1>
    <p class="mx-prose">
      Records your screen so you can scrub back to anything you have seen. Every byte stays on this
      Mac.
    </p>
    <div class="ob-acts">
      <button class="mx-btn mx-btn--primary" onclick={onContinue} disabled={flow.busy}>
        Begin setup<IconArrow class="ob-btn-arrow" width="14" height="14" />
      </button>
      <span class="ob-fine">About a minute. Nothing is recorded until you finish.</span>
    </div>
  </div>

  <div>
    <div class="film" bind:this={film} aria-hidden="true">
      <div class="film__strip" bind:this={strip} style:transform="translateX({end === 0 ? 0 : x}px)">
        {#each FRAMES as widths, k (k)}
          <div class="fr" class:hit={k === HIT}>
            <div class="fr__bar"><i></i><i></i><i></i></div>
            <div class="fr__body">
              {#each widths as width, w (w)}<i style:width="{width}%"></i>{/each}
            </div>
          </div>
        {/each}
      </div>
      <span class="film__head"></span>
    </div>
    <div class="film__rail">
      <span class="film__tag">◂◂ rewinding</span>
      <time class="film__tc">{timecode}</time>
      <span class="film__read">read off that frame — <b>“Quarterly review — draft agenda”</b></span>
    </div>
  </div>
</div>

<style>
  .welcome {
    align-content: space-between;
    padding: 88px 0 var(--s-6);
    gap: var(--s-6);
  }
  .hero {
    display: grid;
    justify-items: start;
    gap: var(--s-5);
    padding: 0 var(--s-8);
  }
  .hero__title {
    margin: 0;
    font: 600 76px / 0.98 var(--font-sans);
    letter-spacing: -0.045em;
    color: var(--app-text-strong);
  }
  .hero__title span {
    color: var(--app-text-subtle);
  }
  .hero .mx-prose {
    font-size: 16px;
    max-width: 46ch;
  }
  .hero .ob-acts {
    margin-top: var(--s-2);
  }

  .film {
    position: relative;
    height: 104px;
    overflow: hidden;
    -webkit-mask-image: linear-gradient(90deg, transparent, black 14%, black 86%, transparent);
    mask-image: linear-gradient(90deg, transparent, black 14%, black 86%, transparent);
  }
  .film__strip {
    position: absolute;
    left: 0;
    top: 6px;
    display: flex;
    gap: 12px;
  }
  .fr {
    flex: none;
    width: 132px;
    height: 88px;
    display: grid;
    grid-template-rows: 10px 1fr;
    overflow: hidden;
    border-radius: var(--r-sm);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    transition:
      border-color var(--t-slow) var(--ease-quart),
      background-color var(--t-slow) var(--ease-quart);
  }
  .fr__bar {
    display: flex;
    gap: 3px;
    align-items: center;
    padding: 0 6px;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .fr__bar i {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--chart-grey-2);
  }
  .fr__body {
    display: grid;
    align-content: start;
    gap: 6px;
    padding: 9px 10px;
  }
  .fr__body i {
    display: block;
    height: 4px;
    border-radius: 2px;
    background: var(--chart-grey-1);
    transition: background-color var(--t-slow) var(--ease-quart);
  }
  .settled .fr.hit {
    border-color: var(--app-accent-border);
    background: color-mix(in srgb, var(--app-accent) 5%, var(--app-surface));
  }
  .settled .fr.hit .fr__body i:nth-child(2) {
    background: var(--app-accent);
  }
  .film__head {
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 0;
    width: 1.5px;
    margin-left: -0.75px;
    background: var(--app-accent);
  }
  .film__head::before {
    content: "";
    position: absolute;
    top: 0;
    left: -4px;
    width: 9.5px;
    height: 6px;
    background: var(--app-accent);
    clip-path: polygon(0 0, 100% 0, 50% 100%);
  }
  .film__rail {
    display: flex;
    align-items: baseline;
    gap: var(--s-4);
    padding: var(--s-3) var(--s-8) 0;
  }
  .film__tag {
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .film__tc {
    font: 500 var(--text-md) / 1 var(--font-mono);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .film__read {
    font: 400 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
    opacity: 0;
    transition: opacity var(--t-slow) var(--ease-quart);
  }
  .film__read b {
    font-weight: 500;
    color: var(--app-text-strong);
  }
  .settled .film__read {
    opacity: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .fr,
    .fr__body i,
    .film__read {
      transition: none;
    }
  }
</style>
