<!--
  Onboarding's capture rows (issue #195, slice 6; redesign ON2).

  Skin of record: `docs/mockups/redesign/onboarding.html` (Capture & Storage) —
  the old sentence-with-dials became spec rows on kit controls:
    Take a snapshot   11-stop `mx-seg--mono` over the capture-rate ladder
    Keep              `mx-seg`, Everything first
    Saves to          the path + Choose…
    (caller rows)     `rows` — the screen slots "Never recorded" in here
  and the verdict panel under them (plan line, verdict, repair buttons).

  The arithmetic and every string the verdict prints live in
  `$lib/onboarding/capture-sentence.ts` (`sentenceVerdict`), kept as is so
  `bun test` pins them; this file is the control surface only. Each rate stop's
  tooltip carries its daily cost, which is what the old hover preview showed.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import Segmented from "$lib/components/Segmented.svelte";
  import {
    CAPTURE_INTERVAL_LADDER_S,
    captureIntervalPhrase,
    intervalSToFps,
    nearestLadderIndex,
  } from "$lib/components/capture-rate";
  import {
    DEFAULT_FOLDER_LABEL,
    RETENTION_STOPS,
    dailyBytes,
    retentionIndex,
    sentencePath,
    sentenceVerdict,
    type ProbeState,
    type RepairAct,
  } from "$lib/onboarding/capture-sentence";
  import type { StorageProbe } from "$lib/onboarding/gates";
  import { describeError, formatBytes } from "$lib/settings/state/format";
  import type { RetentionPolicy } from "$lib/types";
  import IconFolder from "~icons/lucide/folder";

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
    videoPixels = undefined,
    semanticSearchOn = false,
    onDisableSemanticSearch,
    onError,
    rows,
  }: {
    /** Wire format stays fps; the rows never say "fps". */
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
    /** `flow.videoPixels` — pixels per frame at the draft resolution. */
    videoPixels?: number;
    semanticSearchOn?: boolean;
    /** Omit and the "turn Semantic Search off" escape is not offered. */
    onDisableSemanticSearch?: () => void;
    onError?: (message: string) => void;
    /** More `.ob-row`s, drawn after Saves to and before the verdict. */
    rows?: Snippet;
  } = $props();

  const LADDER = CAPTURE_INTERVAL_LADDER_S;
  const rateIndex = $derived(nearestLadderIndex(frameRate));
  const intervalS = $derived(LADDER[rateIndex]!);
  const keepIndex = $derived(retentionIndex(retention));

  const shortRate = (s: number) => (s < 1 ? `${Math.round(1 / s)}/s` : `${s}s`);
  const rateOptions = $derived(
    LADDER.map((s, i) => ({
      value: String(i),
      label: shortRate(s),
      ariaLabel: `${captureIntervalPhrase(s)} — ${formatBytes(dailyBytes(s, videoPixels))} a day`,
    })),
  );
  const keepOptions = RETENTION_STOPS.map((stop) => ({
    value: stop.id,
    label: stop.days === null ? "Everything" : `Last ${stop.days} days`,
  }));

  const shownPath = $derived(saveDirectory.trim() || probedPath);
  const verdict = $derived(
    sentenceVerdict({
      intervalSeconds: intervalS,
      videoPixels,
      retention,
      path: shownPath,
      probe,
      probeState,
      requiredBytes,
      semanticSearchOn,
    }),
  );
  const tone = $derived(
    verdict.tone === "bad" ? "danger" : verdict.tone === "" ? undefined : verdict.tone,
  );
  const phrase = $derived(captureIntervalPhrase(intervalS));

  // ── The folder ────────────────────────────────────────────────────────────
  let browsing = $state(false);
  /** Compare what is SHOWN, not `saveDirectory !== ""`: the settings round
   *  trip fills the draft with the resolved default path. */
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
  // `nosemantic` is dropped when the caller has nothing to turn off, so a dead
  // button is never rendered.
  const repairs = $derived(
    verdict.repairs.filter(
      (r) => r.act !== "nosemantic" || (semanticSearchOn && onDisableSemanticSearch),
    ),
  );

  function act(action: RepairAct) {
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

<div class="ob-rows">
  <div class="ob-row">
    <span class="mx-label">Take a snapshot</span>
    <div class="ob-row__ctl">
      <Segmented
        mono
        options={rateOptions}
        value={String(rateIndex)}
        ariaLabel="How often to take a snapshot"
        onValueChange={(v) => onFrameRateChange(intervalSToFps(LADDER[Number(v)]!))}
      />
      <span class="ob-fine">{phrase}</span>
    </div>
  </div>
  <div class="ob-row">
    <span class="mx-label">Keep</span>
    <div class="ob-row__ctl">
      <Segmented
        options={keepOptions}
        value={retention}
        ariaLabel="How long to keep it"
        onValueChange={(v) => onRetentionChange(v as RetentionPolicy)}
      />
    </div>
  </div>
  <div class="ob-row">
    <span class="mx-label">Saves to</span>
    <div class="ob-row__ctl">
      <span class="ob-path" class:bad={verdict.clause !== null} title={shownPath}>
        {sentencePath(shownPath) || "…"}
      </span>
      {#if verdict.clause}<span class="ob-clause">— {verdict.clause}</span>{/if}
      <button class="mx-btn mx-btn--sm" disabled={browsing} onclick={browse}>
        <IconFolder width="13" height="13" />Choose…
      </button>
      {#if movedAway && verdict.clause === null}
        <button class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => onSaveDirectoryChange("")}>
          Use {DEFAULT_FOLDER_LABEL}
        </button>
      {/if}
    </div>
  </div>
  {@render rows?.()}
</div>

<div class="ob-verdict" data-tone={tone} aria-live="polite">
  <div class="ob-verdict__plan">
    {phrase[0]!.toUpperCase()}{phrase.slice(1)} ·
    {#each verdict.plan as seg, i (i)}{#if seg.kind}<b>{seg.text}</b>{:else}{seg.text}{/if}{/each}
  </div>
  <div class="ob-verdict__text">
    {#if verdict.probing}
      <span class="mx-inline"><span class="mx-spin mx-spin--sm"></span>Checking the folder…</span>
    {:else}
      {#each verdict.verdict as seg, i (i)}{#if seg.kind === "fig"}<b class="fig">{seg.text}</b
          >{:else if seg.kind === "verdict"}<b>{seg.text}</b>{:else}{seg.text}{/if}{/each}
    {/if}
  </div>
  {#if repairs.length > 0}
    <div class="ob-acts">
      {#each repairs as r (r.act)}
        <!-- One green primary per view: the footer's. Repairs stay plain. -->
        <button class="mx-btn mx-btn--sm" onclick={() => act(r.act)}>
          {r.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Spec rows: label column + one control line. `:global` because the caller
     slots its own `.ob-row`s in through `rows`. */
  .ob-rows {
    display: grid;
    border-top: 1px solid var(--mx-hairline);
  }
  .ob-rows :global(.ob-row) {
    display: grid;
    grid-template-columns: 136px minmax(0, 1fr);
    align-items: center;
    gap: var(--s-4);
    min-height: 52px;
    padding: 9px 0;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .ob-rows :global(.ob-row > .mx-label) {
    font-size: var(--text-md);
    color: var(--app-text-muted);
  }
  .ob-rows :global(.ob-row__ctl) {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    flex-wrap: wrap;
  }
  .ob-path {
    display: inline-flex;
    align-items: center;
    height: var(--h-sm);
    padding: 0 var(--s-2);
    border-radius: var(--r-sm);
    font: 450 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-strong);
    background: var(--mx-wash);
    box-shadow: inset 0 0 0 1px var(--mx-hairline);
    white-space: nowrap;
  }
  .ob-path.bad {
    color: var(--app-danger-text);
    box-shadow: inset 0 0 0 1px var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .ob-clause {
    font: 500 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-danger-text);
    white-space: nowrap;
  }

  .ob-verdict {
    display: grid;
    gap: 6px;
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-lg);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
  }
  .ob-verdict__plan {
    font: 400 var(--text-base) / 1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ob-verdict__plan b,
  .ob-verdict__text b.fig {
    font: 500 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .ob-verdict__text {
    font: 400 var(--text-md) / 1.45 var(--font-sans);
    color: var(--app-text);
  }
  .ob-verdict__text b {
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .ob-verdict[data-tone="ok"] .ob-verdict__text b:not(.fig) {
    color: var(--app-accent);
    font-weight: 500;
  }
  .ob-verdict:is([data-tone="warn"], [data-tone="danger"]) {
    border-color: var(--tone-bd);
    background: var(--tone-bg);
  }
  .ob-verdict:is([data-tone="warn"], [data-tone="danger"]) .ob-verdict__text b:not(.fig) {
    color: var(--tone);
  }
  .ob-verdict .ob-acts {
    margin-top: 4px;
  }
</style>
