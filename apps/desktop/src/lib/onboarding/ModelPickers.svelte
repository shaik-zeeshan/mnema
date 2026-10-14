<!--
  Onboarding model pickers (issue #195, slice 10).

  Ported from `docs/onboarding/mockups/input-components/parts/models.part.html`
  — that mockup is the design of record; behaviour and copy come from it.

  Skin of record: `docs/mockups/redesign/onboarding.html` (Change settings →
  Models): `mx-seg` groups, a three-cell spec strip, and a read-only budget
  (total, stacked bar with the free-space marker, legend) as the section footer.

  Three things it changes about the shipping surface:
   · The two `<Select>`s become a second `Segmented`. A toggle group over quality
     OUTCOMES was rejected: four segments over seven transcription models makes
     Whisper Small and both Parakeet builds unreachable.
   · The variant row is height-reserved, so switching family never moves the
     strip or the footer. A family with one build fills the row with a fact.
   · The budget bar is the ONLY place the 419 MB speaker model is ever visible —
     it has no picker anywhere and is spent silently today.

  Semantic Search's group answers only WHICH language coverage. On/off belongs to
  the switch chain, so with the feature off no segment is active and the strip
  reads the off state. `onSemanticEnabledChange` remains for the budget escape.

  Motion: none.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { formatBytes } from "$lib/settings/state/format";
  import { RESERVE_FLOOR_BYTES } from "$lib/onboarding/gates";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
  import IconDisk from "~icons/lucide/hard-drive";
  import {
    FAMILY_NOTES,
    OS_MANAGED_VALUE,
    SEMANTIC_FAMILIES,
    SEMANTIC_OFF,
    diskVerdict,
    downloadBudget,
    pickBytes,
    semanticPicks,
    totalLabel,
    transcriptionPicks,
    type PickerModel,
  } from "$lib/onboarding/model-budget";
  import type {
    AudioTranscriptionModelStatus,
    SemanticSearchModelStatus,
    SemanticSearchSupportedModel,
  } from "$lib/types";

  interface Props {
    // ── Transcription ────────────────────────────────────────────────────
    /** The family group — `transcriptionProviderOptions`, Deepgram already
     *  filtered. Pass `[]` where another control already owns the engine choice
     *  (the onboarding screen renders `Providers.svelte` above this): the group
     *  is then not drawn, and the family still arrives via
     *  `transcriptionFamily`. */
    transcriptionFamilies: { value: string; label: string }[];
    /** `draftTranscriptionProvider`. */
    transcriptionFamily: string;
    /** `selectedTranscriptionModels` — the picked provider's builds. */
    transcriptionModels: readonly AudioTranscriptionModelStatus[];
    /** `draftTranscriptionModelId`; `null` for an OS-managed model. */
    transcriptionModelId: string | null;
    /** `features.transcription` — an off feature downloads nothing. */
    transcriptionEnabled: boolean;
    /** `chooseTranscriptionProvider`. */
    onTranscriptionFamilyChange: (value: string) => void;
    /** `chooseTranscriptionModel` — receives `OS_MANAGED_VALUE` for Apple Speech. */
    onTranscriptionModelChange: (value: string) => void;

    // ── Semantic Search ──────────────────────────────────────────────────
    /** `semanticSearchModelStatus?.models ?? []`. */
    semanticStatus: readonly SemanticSearchModelStatus[];
    /** `semanticSearchSupportedModels` — carries the multilingual flag. */
    semanticCatalog: readonly SemanticSearchSupportedModel[];
    /** `draftSemanticSearchModelId`. */
    semanticModelId: string | null;
    /** `features.semanticSearch` — off means no active segment. */
    semanticEnabled: boolean;
    /** `chooseSemanticSearchModel`. */
    onSemanticModelChange: (value: string) => void;
    /** Flip `features.semanticSearch` (via `flow.toggleFeature`) — on when a
     *  family is picked while off, off from the budget escape. */
    onSemanticEnabledChange: (on: boolean) => void;

    // ── Who's speaking — no picker, footer only ──────────────────────────
    speakerEnabled: boolean;
    speakerInstalled: boolean;
    /** `selectedSpeakerModel?.download?.byteSize ?? SPEAKRS_BYTES`. */
    speakerBytes: number;

    // ── Disk ─────────────────────────────────────────────────────────────
    /** Free bytes on the capture volume; `null` when unmeasured (never blocks). */
    freeBytes: number | null;
    /** Seconds between snapshots — the gate's capture term needs it. */
    captureIntervalSeconds: number;
    /** Pixels per frame at the draft resolution (`flow.videoPixels`). */
    videoPixels?: number;
    /** Context printed beside the Transcription kicker (the chosen engine). */
    transcriptionNote?: Snippet;
  }

  let {
    transcriptionFamilies,
    transcriptionFamily,
    transcriptionModels,
    transcriptionModelId,
    transcriptionEnabled,
    onTranscriptionFamilyChange,
    onTranscriptionModelChange,
    semanticStatus,
    semanticCatalog,
    semanticModelId,
    semanticEnabled,
    onSemanticModelChange,
    onSemanticEnabledChange,
    speakerEnabled,
    speakerInstalled,
    speakerBytes,
    freeBytes,
    captureIntervalSeconds,
    videoPixels = undefined,
    transcriptionNote,
  }: Props = $props();

  // Last build chosen inside each family, so leaving Whisper Medium for Parakeet
  // and coming back does not silently reset a deliberate choice. Plain object,
  // not $state: nothing renders from it.
  const lastInFamily: Record<string, string> = {};

  // ── Transcription ─────────────────────────────────────────────────────────
  const tModels = $derived(transcriptionPicks(transcriptionModels));
  const tPicked = $derived(
    tModels.find((m) => m.id === (transcriptionModelId ?? OS_MANAGED_VALUE)) ??
      tModels[0] ??
      null,
  );
  const tVariants = $derived(
    tModels.map((m) => ({ value: m.id, label: m.short, ariaLabel: m.name })),
  );

  function pickTranscriptionFamily(family: string): void {
    onTranscriptionFamilyChange(family);
    // The parent resolves the family's default model; a remembered build wins.
    const remembered = lastInFamily[family];
    if (remembered) onTranscriptionModelChange(remembered);
  }

  function pickTranscriptionModel(id: string): void {
    lastInFamily[transcriptionFamily] = id;
    onTranscriptionModelChange(id);
  }

  // ── Semantic Search ───────────────────────────────────────────────────────
  const sModels = $derived(semanticPicks(semanticStatus, semanticCatalog));
  const sSelected = $derived(sModels.find((m) => m.id === semanticModelId) ?? null);
  const sPicked = $derived(semanticEnabled ? sSelected : null);
  const sFamily = $derived(sPicked ? sPicked.family : SEMANTIC_OFF);
  const sVariants = $derived(
    sModels
      .filter((m) => m.family === sFamily)
      .map((m) => ({ value: m.id, label: m.short, ariaLabel: m.name })),
  );

  function pickSemanticFamily(family: string): void {
    const next =
      lastInFamily[family] ?? sModels.find((m) => m.family === family)?.id ?? null;
    if (next && next !== semanticModelId) onSemanticModelChange(next);
    if (!semanticEnabled) onSemanticEnabledChange(true);
  }

  function pickSemanticModel(id: string): void {
    lastInFamily[sFamily] = id;
    onSemanticModelChange(id);
  }

  // ── The budget ────────────────────────────────────────────────────────────
  const budget = $derived(
    downloadBudget({
      speakerBytes: speakerEnabled && !speakerInstalled ? speakerBytes : 0,
      transcriptionBytes: pickBytes(tPicked, transcriptionEnabled),
      semanticBytes: pickBytes(sPicked, semanticEnabled),
      semanticApprox: sPicked?.approx ?? false,
    }),
  );

  // "Download anyway" dismisses the shortfall until something changes it.
  let dismissed = $state(false);
  const verdict = $derived(
    diskVerdict({ budget, freeBytes, captureIntervalSeconds, videoPixels }),
  );
  // A new figure is a new decision — an earlier dismissal must not suppress a
  // fresh shortfall.
  $effect(() => {
    void budget.bytes;
    void freeBytes;
    void captureIntervalSeconds;
    void videoPixels;
    dismissed = false;
  });

  /** What is left for downloads once the reserve and a day of capture are set
   *  aside — the same arithmetic the Capture & Storage gate applies. */
  const roomForDownloads = $derived(
    freeBytes === null
      ? null
      : Math.max(
          0,
          freeBytes -
            RESERVE_FLOOR_BYTES -
            estimateDailyStorageMb(captureIntervalSeconds, videoPixels) * 1e6,
        ),
  );
  // One axis for bytes AND room, so "it doesn't fit" is a picture. 6% headroom
  // keeps the marker off the right edge when it fits.
  const axis = $derived(
    Math.max(budget.bytes, roomForDownloads ?? 0, 1) * 1.06,
  );
  const pct = (bytes: number): number => (bytes / axis) * 100;
  const roomPct = $derived(
    roomForDownloads === null ? 0 : Math.min(100, pct(roomForDownloads)),
  );
  // The label is centred on the marker, except near either end, where centring
  // would push it off the track and clip it.
  const labelShift = $derived(roomPct > 78 ? -100 : roomPct < 12 ? 0 : -50);

  function sizeCell(model: PickerModel | null, prefix = ""): string {
    if (!model) return "none";
    if (model.osManaged) return "none — OS managed";
    if (model.installed) return "already on this Mac";
    if (model.bytes <= 0) return "none";
    return `${prefix}${formatBytes(model.bytes)}`;
  }
</script>

<div class="ob-models">
  <!-- ── Transcription ─────────────────────────────────────────────────── -->
  <div class="ob-picker">
    <div class="ob-pane__head">
      <span class="mx-kicker">Transcription</span>
      {#if transcriptionNote}<span class="ob-fine">{@render transcriptionNote()}</span>{/if}
    </div>
    {#if transcriptionFamilies.length > 1}
      <Segmented
        value={transcriptionFamily}
        options={transcriptionFamilies}
        ariaLabel="Transcription engine"
        onValueChange={pickTranscriptionFamily}
      />
    {/if}
    <!-- Height-reserved: the build group appears only for a family with more
         than one build, but the row is always this tall, so nothing below moves. -->
    <div class="ob-picker__ctl">
      {#if tVariants.length > 1}
        <Segmented
          value={tPicked?.id ?? ""}
          options={tVariants}
          ariaLabel="Transcription model"
          onValueChange={pickTranscriptionModel}
        />
      {/if}
      <span class="ob-fine">{FAMILY_NOTES[transcriptionFamily] ?? ""}</span>
    </div>
    <div class="ob-strip">
      <div><span>Model</span><b>{tPicked?.name ?? "None"}</b></div>
      <div><span>Download</span><b class="num">{sizeCell(tPicked)}</b></div>
      <div><span>Memory</span><b class="num">{tPicked?.detail ?? "—"}</b></div>
    </div>
  </div>

  <hr class="mx-rule" />

  <!-- ── Semantic Search ───────────────────────────────────────────────── -->
  <div class="ob-picker">
    <div class="ob-pane__head"><span class="mx-kicker">Semantic Search</span></div>
    <div class="ob-picker__ctl">
      <Segmented
        value={sFamily}
        options={SEMANTIC_FAMILIES}
        ariaLabel="Semantic Search languages"
        onValueChange={pickSemanticFamily}
      />
      {#if sVariants.length > 1}
        <Segmented
          value={sPicked?.id ?? ""}
          options={sVariants}
          ariaLabel="Semantic Search model"
          onValueChange={pickSemanticModel}
        />
      {/if}
      <span class="ob-fine">{FAMILY_NOTES[sFamily] ?? ""}</span>
    </div>
    <div class="ob-strip">
      <div><span>Model</span><b>{sPicked?.name ?? "Off"}</b></div>
      <div><span>Download</span><b class="num">{sizeCell(sPicked, "about ")}</b></div>
      <div><span>Languages</span><b>{sPicked?.detail ?? "keyword search only"}</b></div>
    </div>
  </div>

  <!-- ── Read-only footer: the whole download ──────────────────────────── -->
  <div class="ob-totals">
    <div class="ob-total">
      <b class:over={verdict && !dismissed}>{totalLabel(budget.bytes, budget.approx)}</b>
      <span class="ob-fine">to download before any of this works</span>
    </div>
    <div aria-hidden="true">
      <div class="ob-bar-stack">
        {#if budget.speakerBytes > 0}<i style:--c="var(--app-source-mic)" style:width="{pct(budget.speakerBytes)}%"></i>{/if}
        {#if budget.transcriptionBytes > 0}<i style:--c="var(--chart-grey-4)" style:width="{pct(budget.transcriptionBytes)}%"></i>{/if}
        {#if budget.semanticBytes > 0}<i style:--c="var(--app-info)" style:width="{pct(budget.semanticBytes)}%"></i>{/if}
        {#if verdict}<span class="ob-budget-hatch" style:left="{roomPct}%"></span>{/if}
      </div>
      {#if roomForDownloads !== null}
        <div class="ob-room">
          <i style:left="{roomPct}%"></i>
          <span style:left="{roomPct}%" style:transform="translateX({labelShift}%)">
            {roomForDownloads > 0
              ? `${formatBytes(roomForDownloads)} left for downloads`
              : "no room left for downloads"}
          </span>
        </div>
      {/if}
    </div>
    <ul class="ob-legend">
      <li style:--c="var(--app-source-mic)">
        Who's speaking
        <b>
          {#if !speakerEnabled}off{:else if speakerInstalled}already on this Mac{:else}{formatBytes(
              budget.speakerBytes,
            )}{/if}
        </b>
      </li>
      <li style:--c="var(--chart-grey-4)">
        Transcription <b>{#if !transcriptionEnabled}off{:else}{sizeCell(tPicked)}{/if}</b>
      </li>
      <li style:--c="var(--app-info)">
        Semantic Search <b>{#if !semanticEnabled}off{:else}{sizeCell(sPicked, "about ")}{/if}</b>
      </li>
    </ul>
  </div>

  {#if verdict && !dismissed}
    <div class="mx-notice" data-tone="danger">
      <span class="mx-notice__icon"><IconDisk width="15" height="15" /></span>
      <p class="mx-notice__text">{verdict.message}</p>
      <div class="mx-notice__acts">
        {#if verdict.escapeSavingBytes !== null}
          <button class="mx-btn mx-btn--sm" type="button" onclick={() => onSemanticEnabledChange(false)}>
            Turn Semantic Search off
          </button>
        {/if}
        <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => (dismissed = true)}>
          Download anyway
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .ob-models {
    display: grid;
    gap: var(--s-4);
  }
  .ob-picker {
    display: grid;
    gap: var(--s-3);
  }
  .ob-pane__head {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
  }
  .ob-pane__head :global(b) {
    font-weight: 600;
    color: var(--app-text);
  }
  .ob-picker__ctl {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    flex-wrap: wrap;
    min-height: var(--h-sm);
  }
  .ob-strip {
    display: grid;
    grid-template-columns: 1.6fr 1fr 1fr;
    border: 1px solid var(--app-border);
    border-radius: var(--r-md);
    background: var(--app-surface);
  }
  .ob-strip div {
    display: grid;
    align-content: start;
    gap: 6px;
    padding: 10px var(--s-3);
    min-width: 0;
  }
  .ob-strip div + div {
    border-left: 1px solid var(--mx-hairline);
  }
  .ob-strip span {
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .ob-strip b {
    font: 500 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
    overflow-wrap: break-word;
  }
  .ob-strip b.num {
    font-family: var(--font-mono);
    font-weight: 450;
  }
  .ob-totals {
    display: grid;
    gap: var(--s-2);
  }
  .ob-total {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
  }
  .ob-total b {
    font: 400 22px / 1 var(--font-mono);
    letter-spacing: -0.04em;
    color: var(--app-text-strong);
  }
  .ob-total b.over {
    color: var(--app-danger-text);
  }
  .ob-bar-stack {
    position: relative;
    display: flex;
    gap: 2px;
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
    background: var(--app-surface-hover);
  }
  .ob-bar-stack i {
    display: block;
    height: 100%;
    background: var(--c);
  }
  /* Past the marker the downloads do not fit: that stretch is hatched. */
  .ob-budget-hatch {
    position: absolute;
    inset: 0 0 0 auto;
    background: repeating-linear-gradient(
      135deg,
      var(--app-danger-border) 0 3px,
      transparent 3px 6px
    );
  }
  /* The free-space marker crosses the bar; its label sits under it. */
  .ob-room {
    position: relative;
    height: 14px;
    margin-top: 6px;
    font: 400 var(--text-sm) / 14px var(--font-mono);
    color: var(--app-text-subtle);
  }
  .ob-room i {
    position: absolute;
    top: -16px;
    width: 2px;
    height: 16px;
    margin-left: -1px;
    border-radius: 1px;
    background: var(--app-text-muted);
  }
  .ob-room span {
    position: absolute;
    top: 2px;
    white-space: nowrap;
  }
  .ob-legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-5);
    margin: 0;
    padding: 0;
    list-style: none;
    font: 400 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-muted);
  }
  .ob-legend li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .ob-legend li::before {
    content: "";
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  .ob-legend b {
    font: 500 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-strong);
  }
</style>
