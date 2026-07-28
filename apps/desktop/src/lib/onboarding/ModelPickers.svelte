<!--
  Onboarding model pickers (issue #195, slice 10).

  Ported from `docs/onboarding/mockups/input-components/parts/models.part.html`
  — that mockup is the design of record; behaviour and copy come from it.

  Structure: a toggle group over model FAMILIES, a variant sub-group only where a
  family has more than one build, a fixed three-cell detail strip, and a
  read-only download budget bar as the section footer.

  Three things it changes about the shipping surface:
   · The two `<Select>`s become a second `Segmented`. A toggle group over quality
     OUTCOMES was rejected: four segments over seven transcription models makes
     Whisper Small and both Parakeet builds unreachable.
   · The variant row is height-reserved, so switching family never moves the
     strip or the footer. A family with one build fills the row with a fact.
   · The budget bar is the ONLY place the 419 MB speaker model is ever visible —
     it has no picker anywhere and is spent silently today.

  Semantic Search's `Off` segment IS the row's switch, folded in, because Off is
  the only real saving on that row (every model is 488 MB or more).

  Motion: none ambient. The bar's widths transition because the user just moved
  them; that is the only movement, and `prefers-reduced-motion` drops it.
-->
<script lang="ts">
  import Segmented from "$lib/components/Segmented.svelte";
  import { formatBytes } from "$lib/settings/state/format";
  import { RESERVE_FLOOR_BYTES } from "$lib/onboarding/gates";
  import { estimateDailyStorageMb } from "$lib/onboarding/disk-estimate";
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
    /** `features.semanticSearch` — drives the `Off` segment. */
    semanticEnabled: boolean;
    /** `chooseSemanticSearchModel`. */
    onSemanticModelChange: (value: string) => void;
    /** Flip `features.semanticSearch` (via `flow.toggleFeature`) to `on`. */
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
    if (family === SEMANTIC_OFF) {
      onSemanticEnabledChange(false);
      return;
    }
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
    diskVerdict({ budget, freeBytes, captureIntervalSeconds }),
  );
  // A new figure is a new decision — an earlier dismissal must not suppress a
  // fresh shortfall.
  $effect(() => {
    void budget.bytes;
    void freeBytes;
    void captureIntervalSeconds;
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
            estimateDailyStorageMb(captureIntervalSeconds) * 1e6,
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

<div class="mp">
  <!-- ── Transcription ─────────────────────────────────────────────────── -->
  <div class="mp-picker">
    <span class="ob-m">Transcription</span>
    {#if transcriptionFamilies.length > 1}
      <Segmented
        value={transcriptionFamily}
        options={transcriptionFamilies}
        ariaLabel="Transcription engine"
        onValueChange={pickTranscriptionFamily}
      />
    {/if}
    <!-- Height-reserved: the sub-group appears only for a family with more than
         one build, but the row is always this tall, so nothing below it moves. -->
    <div class="sub">
      {#if tVariants.length > 1}
        <Segmented
          value={tPicked?.id ?? ""}
          options={tVariants}
          ariaLabel="Transcription model"
          onValueChange={pickTranscriptionModel}
        />
      {/if}
      <span class="sub-note">{FAMILY_NOTES[transcriptionFamily] ?? ""}</span>
    </div>
    <div class="strip">
      <div>
        <span class="k">Model</span>
        <span class="v">{tPicked?.name ?? "None"}</span>
      </div>
      <div>
        <span class="k">Download</span>
        <span class="v">{sizeCell(tPicked)}</span>
      </div>
      <div>
        <span class="k">Memory</span>
        <span class="v dim">{tPicked?.detail ?? "—"}</span>
      </div>
    </div>
  </div>

  <hr class="ob-rule" />

  <!-- ── Semantic Search ───────────────────────────────────────────────── -->
  <div class="mp-picker">
    <span class="ob-m">Semantic Search</span>
    <Segmented
      value={sFamily}
      options={SEMANTIC_FAMILIES}
      ariaLabel="Semantic Search languages"
      onValueChange={pickSemanticFamily}
    />
    <div class="sub">
      {#if sVariants.length > 1}
        <Segmented
          value={sPicked?.id ?? ""}
          options={sVariants}
          ariaLabel="Semantic Search model"
          onValueChange={pickSemanticModel}
        />
      {/if}
      <span class="sub-note">{FAMILY_NOTES[sFamily] ?? ""}</span>
    </div>
    <div class="strip">
      <div>
        <span class="k">Model</span>
        <span class="v">{sPicked?.name ?? "Off"}</span>
      </div>
      <div>
        <span class="k">Download</span>
        <span class="v">{sizeCell(sPicked, "about ")}</span>
      </div>
      <div>
        <span class="k">Languages</span>
        <span class="v dim">{sPicked?.detail ?? "keyword search only"}</span>
      </div>
    </div>
  </div>

  <!-- ── Read-only footer: the whole download ──────────────────────────── -->
  <div class="foot">
    <div class="budget" aria-hidden="true">
      <div class="budget-track">
        {#if budget.speakerBytes > 0}
          <i class="b-speaker" style="width:{pct(budget.speakerBytes)}%"></i>
        {/if}
        {#if budget.transcriptionBytes > 0}
          <i class="b-transcription" style="width:{pct(budget.transcriptionBytes)}%"></i>
        {/if}
        {#if budget.semanticBytes > 0}
          <i class="b-semantic" style="width:{pct(budget.semanticBytes)}%"></i>
        {/if}
        <i class="b-rest"></i>
        {#if verdict}
          <div class="budget-hatch" style="left:{roomPct}%"></div>
        {/if}
      </div>
      {#if roomForDownloads !== null}
        <div class="disk-line">
          <i style="left:{roomPct}%"></i>
          <span style="left:{roomPct}%; transform:translateX({labelShift}%)">
            {roomForDownloads > 0
              ? `${formatBytes(roomForDownloads)} left for downloads`
              : "no room left for downloads"}
          </span>
        </div>
      {/if}
    </div>

    <div class="tot-line">
      <span class="tot-hero" class:over={verdict && !dismissed}>
        {totalLabel(budget.bytes, budget.approx)}
      </span>
      <span class="tot-cap">to download before any of this works</span>
    </div>
    <div class="tot-parts">
      <span>
        <em class="b-speaker"></em>Who's speaking
        <b>
          {#if !speakerEnabled}off{:else if speakerInstalled}already on this Mac{:else}{formatBytes(
              budget.speakerBytes,
            )} · no picker{/if}
        </b>
      </span>
      <span>
        <em class="b-transcription"></em>Transcription
        <b>
          {#if !transcriptionEnabled}off{:else}{sizeCell(tPicked)}{/if}
        </b>
      </span>
      <span>
        <em class="b-semantic"></em>Semantic Search
        <b>
          {#if !semanticEnabled}off{:else}{sizeCell(sPicked, "about ")}{/if}
        </b>
      </span>
    </div>

    {#if verdict && !dismissed}
      <div class="escape">
        <span class="msg">{verdict.message}</span>
        {#if verdict.escapeSavingBytes !== null}
          <button
            class="ob-btn sm"
            type="button"
            onclick={() => onSemanticEnabledChange(false)}
          >
            Turn Semantic Search off
          </button>
        {/if}
        <button class="ob-btn sm" type="button" onclick={() => (dismissed = true)}>
          Download anyway
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .mp :global(.ob-m) {
    display: block;
    margin-bottom: 8px;
  }
  .mp :global(.ob-rule) {
    margin: 18px 0;
  }

  /* The variant row is ALWAYS this tall: a `Segmented`'s own height — 12px label
     at line-height 1 + 5px×2 segment padding + 2px×2 group padding + 1px×2
     border. A family with one build leaves the strip and the footer exactly
     where the multi-build families put them. */
  .sub {
    margin-top: 9px;
    min-height: 28px;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .sub-note {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    line-height: 1.5;
  }

  /* Three cells, two lines of value reserved in each, so the longest string in
     the manifest wraps instead of clipping and the block still never changes
     height. */
  .strip {
    margin-top: 10px;
    display: grid;
    grid-template-columns: 1.35fr 1fr 1.15fr;
    gap: 1px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-border);
    overflow: hidden;
  }
  .strip > div {
    box-sizing: border-box;
    background: var(--app-surface-subtle);
    padding: 8px 11px;
    min-height: 64px;
    min-width: 0;
  }
  .strip .k {
    display: block;
    font-size: var(--text-xs);
    line-height: 1.1;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    margin-bottom: 4px;
  }
  .strip .v {
    display: block;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    overflow-wrap: break-word;
  }
  .strip .v.dim {
    color: var(--app-text-muted);
  }

  /* ── Section footer: read-only budget bar + the total of record ───────── */
  .foot {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--app-border);
  }
  .budget-track {
    position: relative;
    display: flex;
    height: 22px;
    gap: 1px;
    border-radius: 6px;
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 1px var(--app-border);
  }
  .budget-track i {
    display: block;
    height: 100%;
    transition: width 0.22s cubic-bezier(0.2, 0.8, 0.3, 1);
  }
  .b-speaker {
    background: var(--app-accent-strong);
  }
  .b-transcription {
    background: var(--app-accent);
  }
  .b-semantic {
    background: var(--app-info);
  }
  .b-rest {
    background: transparent;
    flex: 1 1 auto;
  }
  .budget-hatch {
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    background: repeating-linear-gradient(
      135deg,
      var(--app-danger-bg) 0 5px,
      transparent 5px 10px
    );
    box-shadow: inset 1px 0 0 var(--app-danger-border);
    pointer-events: none;
  }
  .disk-line {
    position: relative;
    height: 14px;
    margin-top: 2px;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }
  .disk-line i {
    position: absolute;
    top: -26px;
    width: 2px;
    height: 30px;
    background: var(--app-text-muted);
    border-radius: 1px;
    transition: left 0.22s cubic-bezier(0.2, 0.8, 0.3, 1);
  }
  .disk-line span {
    position: absolute;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    transition: left 0.22s cubic-bezier(0.2, 0.8, 0.3, 1);
  }
  .tot-line {
    margin-top: 16px;
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  .tot-hero {
    font-size: var(--text-xl);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.01em;
  }
  .tot-hero.over {
    color: var(--app-danger);
  }
  .tot-cap {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
  }
  .tot-parts {
    margin-top: 7px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .tot-parts span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .tot-parts em {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  .tot-parts b {
    color: var(--app-text-muted);
    font-weight: 400;
  }

  .escape {
    margin-top: 12px;
    padding: 10px 12px;
    border: 1px solid var(--app-danger-border);
    border-radius: 8px;
    background: var(--app-danger-bg);
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .escape .msg {
    font-size: var(--text-sm);
    color: var(--app-danger);
    line-height: 1.6;
    flex: 1 1 240px;
  }
  .escape :global(.ob-btn) {
    border-color: var(--app-danger-border);
    color: var(--app-text);
  }

  @media (prefers-reduced-motion: reduce) {
    .budget-track i,
    .disk-line i,
    .disk-line span {
      transition: none;
    }
  }
</style>
