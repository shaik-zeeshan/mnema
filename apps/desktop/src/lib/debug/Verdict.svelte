<!--
  The verdict line: one sentence over the health rollup + a tone dot, and on the
  right (mono) the paused time, or recording duration and app version.
-->
<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { formatClock } from "$lib/time-format";
  import { getDebugController } from "./state/controller.svelte";
  import { formatRunning, verdict } from "./verdict";

  const controller = getDebugController();
  const { health, capture } = controller;

  let version = $state("");
  $effect(() => {
    getVersion().then((v) => (version = v)).catch(() => {});
  });

  const v = $derived.by(() => {
    if (health.entries.length > 0) return verdict(health.entries);
    if (health.error) return { tone: "danger" as const, text: "Can’t read health" };
    return { tone: "off" as const, text: "Checking…" };
  });

  // Earliest per-source start = when this recording began. Ticks with the
  // health poll, so it stops counting while paused (the right side says so).
  const startedAtMs = $derived.by(() => {
    const sessions = capture.session?.sourceSessions;
    const starts = [sessions?.screen, sessions?.microphone, sessions?.systemAudio]
      .map((s) => s?.startedAtUnixMs)
      .filter((ms): ms is number => ms != null);
    return starts.length > 0 ? Math.min(...starts) : null;
  });

  const right = $derived.by(() => {
    if (controller.pausedAtMs != null) return `paused at ${formatClock(controller.pausedAtMs, { seconds: true })}`;
    const tail = version ? ` · ${version}` : "";
    if (capture.isCapturing && startedAtMs != null) {
      return `recording ${formatRunning((health.fetchedAtMs ?? Date.now()) - startedAtMs)}${tail}`;
    }
    return `${capture.isCapturing ? "recording" : "not recording"}${tail}`;
  });
</script>

<div class="dv-verdict" data-tone={v.tone} aria-live="polite">
  <span class="dv-dot"></span><b>{v.text}</b><span class="num">{right}</span>
</div>
