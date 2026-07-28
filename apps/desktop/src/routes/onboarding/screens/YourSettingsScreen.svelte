<!--
  Screen 4 / 8 — Your settings (issue #195, slice 8).

  A READ-ONLY manifest: eight rows of name + one short value, one sentence of
  context, two actions. No row expands into an editor and nothing here mutates
  settings — every change is a round trip to *Change settings*, which is also
  where provider names, model identifiers and per-row byte sizes live. That
  demotion is the whole reason this screen is light.

  Ported from `docs/onboarding/mockups/chosen-cinematic-rewind.html` 1140-1204
  (`chosen-shots/s06-dark.png`), in `var(--app-*)` tokens.
-->
<script lang="ts">
  import { systemAudioNeedsRequest } from "$lib/onboarding/feature-rules";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
  import { workListBytes } from "$lib/onboarding/resolve-setup";
  import { retentionToDays } from "$lib/components/retention";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

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
    { name: "AI features", value: f.aiFeatures ? "on" : "off — needs a provider", on: f.aiFeatures },
  ]);

  // ── The one foot line: downloads · location · retention · daily figure ────
  // `flow.workList` is live: it tracks both the feature state (returning from
  // Change settings with the last audio source off drops Whisper and speakrs)
  // and the model picks made there.
  const workList = $derived(flow.workList);
  // Any total containing nomic is approximate — its Rust figure is
  // `approx_download_bytes` (see `resolve-setup.ts`).
  const approximate = $derived(workList.some((item) => item.subsystem === "semanticSearch"));
  const downloads = $derived(
    workList.length === 0
      ? "Nothing to download"
      : `Downloads ${approximate ? "about " : ""}${formatSize(workListBytes(workList))}`,
  );

  const saveDirectory = $derived(homeRelative(flow.controller.draftSaveDirectory));
  const retention = $derived.by(() => {
    const days = retentionToDays(flow.controller.draftRetentionPolicy);
    return days === null ? "keeps everything" : `keeps ${days} days`;
  });
  const daily = $derived.by(() => {
    const fps = flow.controller.draftFrameRate;
    const mb = estimateDailyStorageMb(fps > 0 ? 1 / fps : 0);
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

<h1 class="ob-disp mid centred">Already answered.</h1>

<div class="manifest">
  {#each rows as row (row.name)}
    <div class="row" class:off={!row.on} class:warned={row.warned}>
      <span class="tick" aria-hidden="true">{row.on ? "✓" : "○"}</span>
      <span class="nm">{row.name}</span>
      <span class="val">
        {row.value}
        {#if row.grant}
          <button class="grant" type="button" onclick={() => flow.goTo("permissions")}>
            Grant ▸
          </button>
        {/if}
      </span>
    </div>
  {/each}
</div>

<div class="ob-foot">
  <p class="context">
    <span class="ob-num ob-strong">{downloads}</span>
    · saves to <span class="ob-strong">{saveDirectory}</span>
    · {retention}
    · about <span class="ob-num ob-strong">{daily}</span> a day
  </p>
  <hr class="ob-rule" />
  <div class="ob-acts">
    <button class="ob-btn ghost spacer" onclick={onBack}>← Back</button>
    <button class="ob-btn" onclick={onChangeSettings}>Change settings</button>
    <button class="ob-btn primary" onclick={onContinue}>Start&nbsp; →</button>
  </div>
</div>

<style>
  .centred {
    text-align: center;
  }

  /* Eight rows, 600px wide, centred — name and one short value, nothing else. */
  .manifest {
    width: 600px;
    max-width: 100%;
    margin: 38px auto 0;
    display: flex;
    flex-direction: column;
  }
  .row {
    display: grid;
    grid-template-columns: 16px 1fr auto;
    gap: 16px;
    align-items: baseline;
    padding: 11px 0;
    border-bottom: 1px solid var(--app-border);
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row .nm {
    font-size: var(--text-md);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .row .val {
    font-size: var(--text-md);
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .row .tick {
    color: var(--app-accent);
  }
  .row.off .tick {
    color: var(--app-text-subtle);
  }
  .row.off .nm {
    color: var(--app-text-muted);
  }
  .row.off .val {
    color: var(--app-text-subtle);
  }
  .row.warned .val {
    color: var(--app-warn);
  }

  /* The way back for a missing permission. Not an editor — a link to the
     Permissions screen, which is the only place a grant can be requested. */
  .grant {
    font: inherit;
    font-size: var(--text-sm);
    color: var(--app-text);
    background: transparent;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    padding: 3px 9px;
    margin-left: 10px;
    cursor: pointer;
  }
  .grant:hover {
    background: var(--app-surface-hover);
  }
  .grant:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .context {
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-text);
    text-align: center;
    margin: 0;
  }
</style>
