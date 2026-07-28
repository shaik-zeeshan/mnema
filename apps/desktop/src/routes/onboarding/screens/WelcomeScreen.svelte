<!--
  Screen 1 / 8 — Welcome (issue #195, slice 10).

  One button, one sentence, and NO "use recommended defaults" — every downstream
  choice is already answered by the resolver.

  Motion lives here and on the Finale, nowhere else: a filmstrip of captured
  screens travelling backwards under a fixed playhead (slow creep, hard snap
  back six frames, settle), two ghost strips lagging by 100/200 ms so the snap
  smears, a timecode reel falling with it, and the frame under the playhead
  giving up the words it was carrying. CSS only, no script, one 10 s cycle.
  Ported from `docs/onboarding/mockups/chosen-cinematic-rewind.html` 754-805.

  The timecode is derived from the real configured capture interval, not the
  mockup's hardcoded reel — "two seconds per frame" is the actual default rate.
-->
<script lang="ts">
  import { fpsToIntervalS } from "$lib/components/capture-rate";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

  let {
    flow,
    onContinue,
  }: { flow: OnboardingFlow; onContinue: () => void } = $props();

  // Eight distinct wireframe tiles; index 3 is the one under the playhead when
  // the strip settles (the "hit" that gives up its words).
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
  const HIT_INDEX = 3;
  // Three identical groups so the strip can travel exactly one group per cycle.
  const GROUPS = [0, 1, 2];

  const intervalS = $derived(fpsToIntervalS(flow.controller.draftFrameRate));

  // The reel falls with the strip: settle · one frame back · the snap · settle.
  // Multiples of the real interval, anchored to the clock at mount.
  const anchor = new Date();
  const reel = $derived(
    [0, 1, 4, 5].map((frames) =>
      new Date(anchor.getTime() - frames * intervalS * 1000).toLocaleTimeString("en-GB", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
      }),
    ),
  );
</script>

{#snippet strip()}
  {#each GROUPS as group (group)}
    {#each TILES as widths, i (i)}
      <div class="fr" class:hit={i === HIT_INDEX}>
        <span class="fbar"></span>
        <div class="fbody">
          {#each widths as width, w (w)}
            <i class:hl={i === HIT_INDEX && w === 1} style="width:{width}%"></i>
          {/each}
        </div>
      </div>
    {/each}
  {/each}
{/snippet}

<div class="hero">
  <span class="ob-m">Mnema</span>
  <h1 class="ob-disp wl-t" style="margin-top:14px">
    <span class="wl-echo a" aria-hidden="true">Your memory,<br />on rewind.</span>
    <span class="wl-echo b" aria-hidden="true">Your memory,<br />on rewind.</span>
    Your memory,<br />on rewind.
  </h1>
  <p class="ob-lead" style="margin-top:26px">
    Records your screen so you can scrub back to anything you have seen. Every byte stays on
    this Mac.
  </p>
  <div class="ob-acts" style="margin-top:38px">
    <button class="ob-btn primary ring" onclick={onContinue} disabled={flow.busy}>
      Begin setup&nbsp; →
    </button>
    <span class="ob-fine">About a minute. Nothing is recorded until you finish.</span>
  </div>
</div>

<div class="rw">
  <div class="rw-band" aria-hidden="true">
    <div class="rw-strip e2">{@render strip()}</div>
    <div class="rw-strip e1">{@render strip()}</div>
    <div class="rw-strip">{@render strip()}</div>
    <span class="rw-head"></span>
  </div>
  <div class="rw-rail">
    <span class="rw-tag">◂◂ rewinding</span>
    <div class="rw-tc" aria-hidden="true">
      <div class="rw-reel">
        {#each reel as stamp, i (i)}<span>{stamp}</span>{/each}
      </div>
    </div>
    <span class="rw-read">
      read off that frame — <b>“Quarterly review — draft agenda”</b>
    </span>
  </div>
</div>

<style>
  .hero {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    position: relative;
  }

  /* The band bleeds to the window edges and sits on the stage's bottom padding.
     Cancel that padding with the shell's own tokens, never a literal: the
     hardcoded `0 -48px -40px` was tuned to a stage padding of `46px 48px 40px`,
     and when the shell tightened to `28px 44px 20px` the band kept eating 40px
     of a 20px gutter — 20px of stage overflow, at every window size. */
  .rw {
    position: relative;
    flex: none;
    margin: 0 calc(-1 * var(--ob-pad-x)) calc(-1 * var(--ob-pad-b));
    padding-bottom: 20px;
  }
  .rw-band {
    position: relative;
    height: 96px;
    overflow: hidden;
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 12%, #000 88%, transparent);
    mask-image: linear-gradient(90deg, transparent, #000 12%, #000 88%, transparent);
  }
  .rw-band::after {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 2;
    background: linear-gradient(to bottom, var(--app-bg), transparent 42%);
  }

  /* Geometry: tile 128px + 10px gap = 138px pitch; the strip is three identical
     8-tile groups and travels exactly one group (1104px) per cycle, so the loop
     is seamless. */
  .rw-strip {
    position: absolute;
    top: 6px;
    left: -1104px;
    display: flex;
    gap: 10px;
    animation: rw-scrub 10s linear infinite;
    will-change: transform;
  }
  .rw-strip.e1 {
    opacity: 0;
    --gop: 0.62;
    animation:
      rw-scrub 10s linear infinite,
      rw-ghost 10s linear infinite;
    animation-delay: 0.1s;
  }
  .rw-strip.e2 {
    opacity: 0;
    --gop: 0.34;
    animation:
      rw-scrub 10s linear infinite,
      rw-ghost 10s linear infinite;
    animation-delay: 0.2s;
  }
  /* The trail has to be brighter than the live strip or it vanishes into the tile. */
  .rw-strip.e1 .fr,
  .rw-strip.e2 .fr {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .rw-strip.e1 .fbody i,
  .rw-strip.e2 .fbody i {
    background: var(--app-border-strong);
  }
  @keyframes rw-scrub {
    0% {
      transform: translateX(0);
    }
    62% {
      transform: translateX(138px); /* creep — one frame in 6.2s */
    }
    70% {
      transform: translateX(966px); /* the snap — six frames back */
    }
    100% {
      transform: translateX(1104px); /* settle — one more frame */
    }
  }
  @keyframes rw-ghost {
    0%,
    60% {
      opacity: 0;
    }
    63%,
    69% {
      opacity: var(--gop, 0.3);
    }
    72%,
    100% {
      opacity: 0;
    }
  }

  .fr {
    width: 128px;
    height: 84px;
    flex: none;
    overflow: hidden;
    border-radius: 4px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
  }
  .fr.hit {
    border-color: var(--app-accent-border);
  }
  .fbar {
    display: block;
    height: 9px;
    background: var(--app-surface-raised);
    border-bottom: 1px solid var(--app-border);
  }
  .fbody {
    padding: 9px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .fbody i {
    display: block;
    height: 4px;
    border-radius: 1px;
    background: var(--app-border-hover);
  }
  .fbody i.hl {
    background: var(--app-accent-bg);
    box-shadow: 0 0 0 1px var(--app-accent-border);
  }

  .rw-head {
    position: absolute;
    left: 34%;
    top: 0;
    bottom: 0;
    width: 1px;
    z-index: 3;
    background: var(--app-accent);
    box-shadow: 0 0 12px 2px var(--app-accent-glow);
  }
  .rw-head::before {
    content: "";
    position: absolute;
    top: 0;
    left: -3px;
    width: 7px;
    height: 5px;
    background: var(--app-accent);
    clip-path: polygon(0 0, 100% 0, 50% 100%);
  }

  /* The rail is inset back to the content column, so it must undo exactly what
     `.rw` bled — same token, or the reel drifts off the playhead. */
  .rw-rail {
    position: relative;
    height: 34px;
    margin: 0 var(--ob-pad-x);
  }
  .rw-rail::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 1px;
    background: repeating-linear-gradient(
      90deg,
      var(--app-border-hover) 0 1px,
      transparent 1px 23px
    );
  }
  .rw-tc {
    position: absolute;
    left: calc(34% - var(--ob-pad-x));
    top: 9px;
    transform: translateX(-50%);
    height: 16px;
    overflow: hidden;
    font-size: var(--text-sm);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.04em;
  }
  .rw-reel {
    display: flex;
    flex-direction: column;
    animation: rw-reel 10s steps(1, end) infinite;
  }
  .rw-reel span {
    height: 16px;
    line-height: 16px;
    flex: none;
  }
  @keyframes rw-reel {
    0%,
    49.9% {
      transform: translateY(0);
    }
    50%,
    61.9% {
      transform: translateY(-16px);
    }
    62%,
    87.9% {
      transform: translateY(-32px);
    }
    88%,
    100% {
      transform: translateY(-48px);
    }
  }
  .rw-tag {
    position: absolute;
    left: 0;
    top: 11px;
    font-size: var(--text-xs);
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--app-accent);
    opacity: 0;
    animation: rw-tag 10s linear infinite;
  }
  @keyframes rw-tag {
    0%,
    60% {
      opacity: 0;
    }
    63%,
    69% {
      opacity: 1;
    }
    73%,
    100% {
      opacity: 0;
    }
  }
  .rw-read {
    position: absolute;
    right: 0;
    top: 11px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    opacity: 0;
    animation: rw-read 10s ease infinite;
    white-space: nowrap;
  }
  .rw-read b {
    font-weight: 400;
    color: var(--app-accent);
  }
  @keyframes rw-read {
    0%,
    72% {
      opacity: 0;
    }
    76%,
    90% {
      opacity: 1;
    }
    94%,
    100% {
      opacity: 0;
    }
  }

  /* The hero title smears backwards on the snap, then settles. */
  .wl-t {
    position: relative;
  }
  .wl-echo {
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0;
    color: var(--app-text-strong);
    animation: wl-echo 10s cubic-bezier(0.2, 0.8, 0.3, 1) infinite;
  }
  .wl-echo.b {
    animation-delay: 0.07s;
  }
  @keyframes wl-echo {
    0%,
    61% {
      opacity: 0;
      transform: translateX(0);
    }
    63.5% {
      opacity: 0.26;
      transform: translateX(-34px);
    }
    67% {
      opacity: 0.11;
      transform: translateX(-70px);
    }
    70.5%,
    100% {
      opacity: 0;
      transform: translateX(0);
    }
  }

  .ob-btn.ring {
    box-shadow: var(--app-ring);
  }

  /* Nothing is switched off — every meaning survives, only movement goes. */
  @media (prefers-reduced-motion: reduce) {
    .rw-strip {
      animation: none;
      transform: translateX(1000px);
    }
    .rw-strip.e1,
    .rw-strip.e2 {
      display: none;
    }
    .wl-echo {
      display: none;
    }
    .rw-reel {
      animation: none;
      transform: translateY(-32px);
    }
    .rw-tag {
      animation: none;
      opacity: 0.45;
    }
    .rw-read {
      animation: none;
      opacity: 1;
    }
  }
</style>
