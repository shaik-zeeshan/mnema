<!--
  Screen 4 / 8 — Your settings (issue #195, slice 8).

  A READ-ONLY manifest: a 2x4 grid of tick + name + one short value, a mono
  facts row, two actions. No row expands into an editor and nothing here mutates
  settings — every change is a round trip to *Change settings*, which is also
  where provider names, model identifiers and per-row byte sizes live. That
  demotion is the whole reason this screen is light.

  Skin of record: `docs/mockups/redesign/onboarding.html` (Your settings).
-->
<script lang="ts">
  import { systemAudioNeedsRequest } from "$lib/onboarding/feature-rules";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
  import { workListBytes } from "$lib/onboarding/resolve-setup";
  import { retentionToDays } from "$lib/components/retention";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconCheck from "~icons/lucide/check";
  import IconHalf from "~icons/lucide/circle-dashed";
  import IconRing from "~icons/lucide/circle";

  let {
    flow,
    onContinue,
    onBack,
    onChangeSettings,
  }: {
    flow: OnboardingFlow;
    onContinue: () => void;
    onBack: () => void;
    onChangeSettings: () => void;
  } = $props();

  const f = $derived(flow.features);
  const perms = $derived(flow.features.permissions);

  interface Row {
    name: string;
    value: string;
    on: boolean;
    /** Renders the value in the warn tone (a missing permission). */
    warned?: boolean;
    /** Offers the way back to the Permissions screen. */
    grant?: boolean;
    /** On, but nothing can confirm it works (system audio, ADR 0052). */
    unconfirmed?: boolean;
  }

  // One line per row, one short value, nothing else. A row whose permission is
  // missing SAYS SO and stays listed — it never vanishes and never blocks Start.
  const rows = $derived<Row[]>([
    perms.screen
      ? { name: "Screen capture", value: f.screen ? "on" : "off", on: f.screen }
      : { name: "Screen capture", value: "not granted", on: false, warned: true, grant: true },
    { name: "Read on-screen text", value: f.ocr ? "on" : "off", on: f.ocr },
    perms.microphone
      ? { name: "Microphone", value: f.microphone ? "on" : "off", on: f.microphone }
      : { name: "Microphone", value: "not granted", on: false, warned: true, grant: true },
    {
      name: "System audio",
      // macOS exposes no API to read this grant (ADR 0052), so the row states
      // that plainly rather than claiming a confirmation we cannot have.
      value: !f.systemAudio
        ? "off"
        : systemAudioNeedsRequest(f)
          ? "on · macOS can't confirm the grant"
          : "on",
      on: f.systemAudio,
      unconfirmed: f.systemAudio,
    },
    {
      name: "Transcription",
      value: f.transcription ? "on" : f.microphone || f.systemAudio ? "off" : "off — no audio source",
      on: f.transcription,
    },
    {
      name: "Who's speaking",
      value: f.speakerSeparation ? "on" : f.transcription ? "off" : "off — needs transcription",
      on: f.speakerSeparation,
    },
    { name: "Semantic Search", value: f.semanticSearch ? "on" : "off", on: f.semanticSearch },
    // Never pre-ticked: consent is an affirmative act, made on Change settings.
    // On with nothing connected is allowed (a soft warning, not a hold) — say so.
    {
      name: "AI features",
      value: !f.aiFeatures
        ? "off — needs a provider"
        : flow.controller.ai.aiConfigReady
          ? "on"
          : "on · nothing connected yet",
      on: f.aiFeatures,
    },
  ]);

  // ── The foot facts: downloads · location · retention · daily figure ───────
  // `flow.workList` is live: it tracks both the feature state (returning from
  // Change settings with the last audio source off drops Whisper and speakrs)
  // and the model picks made there.
  const workList = $derived(flow.workList);
  // Any total containing nomic is approximate — its Rust figure is
  // `approx_download_bytes` (see `resolve-setup.ts`).
  const approximate = $derived(workList.some((item) => item.subsystem === "semanticSearch"));
  const downloads = $derived(
    workList.length === 0
      ? "Nothing"
      : `${approximate ? "~" : ""}${formatSize(workListBytes(workList))}`,
  );

  const saveDirectory = $derived(homeRelative(flow.controller.draftSaveDirectory));
  const retention = $derived.by(() => {
    const days = retentionToDays(flow.controller.draftRetentionPolicy);
    return days === null ? "Everything" : `${days} days`;
  });
  const daily = $derived.by(() => {
    const fps = flow.controller.draftFrameRate;
    const mb = estimateDailyStorageMb(fps > 0 ? 1 / fps : 0, flow.videoPixels);
    return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${Math.round(mb)} MB`;
  });

  // ponytail: decimal MB/GB, not the 1024-based `formatBytes` — every figure in
  // the plan and the model manifests is SI (Whisper base 147,951,465 = 148 MB).
  function formatSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
    return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
  }

  function homeRelative(path: string): string {
    const trimmed = path.trim();
    if (trimmed.length === 0) return "the default folder";
    return trimmed.replace(/^\/Users\/[^/]+/, "~");
  }
</script>

<div class="ob-main">
  <div class="ob-col">
    <div class="ob-head">
      <span class="mx-kicker">Your settings</span>
      <h1 class="mx-display">Already answered.</h1>
    </div>

    <!-- Two columns of four, capture | processing: rows 0-3 left, 4-7 right. -->
    <div class="ob-manifest">
      {#each [0, 4, 1, 5, 2, 6, 3, 7] as k (k)}
        {@const row = rows[k]!}
        <div class="ob-man" class:off={!row.on} class:warn={row.warned}>
          {#if row.unconfirmed}
            <IconHalf class="ob-mark" />
          {:else if row.on}
            <IconCheck class="ob-mark ob-mark--on" />
          {:else}
            <IconRing class="ob-mark" />
          {/if}
          <span class="ob-man__name">{row.name}</span>
          <span class="ob-man__val">
            {row.value}
            {#if row.grant}
              <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => flow.goTo("permissions")}>
                Grant
              </button>
            {/if}
          </span>
        </div>
      {/each}
    </div>

    <dl class="ob-facts">
      <div><dt>Download</dt><dd>{downloads}</dd></div>
      <div><dt>Saves to</dt><dd class="path" title={flow.controller.draftSaveDirectory}>{saveDirectory}</dd></div>
      <div><dt>Keeps</dt><dd>{retention}</dd></div>
      <div><dt>Per day</dt><dd>~{daily}</dd></div>
    </dl>
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note">Every line here stays changeable in Settings.</p>
  <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
  <button class="mx-btn" onclick={onChangeSettings}>Change settings</button>
  <button class="mx-btn mx-btn--primary" onclick={onContinue}>
    Start<IconArrow class="ob-btn-arrow" width="14" height="14" />
  </button>
</footer>

<style>
  .ob-manifest {
    display: grid;
    grid-template-columns: 1fr 1fr;
    column-gap: var(--s-6);
    border-top: 1px solid var(--mx-hairline);
  }
  .ob-man {
    display: grid;
    grid-template-columns: 14px auto minmax(0, 1fr);
    align-items: center;
    gap: var(--s-3);
    height: 42px;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .ob-man__name {
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .ob-man__val {
    justify-self: end;
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    font: 400 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-muted);
    white-space: nowrap;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ob-man.off .ob-man__name,
  .ob-man.off .ob-man__val {
    color: var(--app-text-subtle);
  }
  .ob-man.warn .ob-man__val {
    color: var(--app-warn);
  }

  .ob-facts {
    display: grid;
    grid-template-columns: repeat(4, auto);
    justify-content: start;
    column-gap: var(--s-7);
    margin: 0;
  }
  .ob-facts div {
    display: grid;
    gap: 8px;
    min-width: 0;
  }
  .ob-facts dt {
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .ob-facts dd {
    margin: 0;
    font: 450 var(--text-lg) / 1 var(--font-mono);
    color: var(--app-text-strong);
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  /* A long save path is the one value that can outrun its column. */
  .ob-facts dd.path {
    max-width: 34ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
