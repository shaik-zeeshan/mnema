// Reactivity harness driver — mirrors TimelineJumper.svelte's load effect
// (L186-189) verbatim so we exercise the REAL rune adapter's reactive wiring:
//   $effect(() => { if (!open) return; void cache.load(placeholder); });
// Compiled to plain JS by build.mjs (bun test can't run runes directly).
import { createJumperCache } from "./jumper-cache";

export function makeDriver() {
  const cache = createJumperCache();
  let open = $state(false);
  // Fixed placeholder (same month for the whole test): the ONLY thing that
  // should re-drive load() after the first fetch is the cache marking the
  // month stale via invalidate*, exactly as the head poll does mid-open.
  const placeholder = { year: 2026, month: 6, day: 15 };
  const stop = $effect.root(() => {
    $effect(() => {
      if (!open) return;
      void cache.load(placeholder);
    });
  });
  return {
    setOpen(v: boolean) {
      open = v;
    },
    invalidate(frames: { capturedAt: string }[]) {
      cache.invalidateMonthsForFrames(frames);
    },
    stop,
  };
}
