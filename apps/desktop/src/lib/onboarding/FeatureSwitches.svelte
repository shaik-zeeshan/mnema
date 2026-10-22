<!--
  Feature switches — the dependency graph drawn as a chain (issue #195, slice 8).

  Ported from `docs/onboarding/mockups/input-components/parts/switches.part.html`,
  which is the specification for behaviour and copy.

  Replaces the eight `<Switch>` rows scattered across three sections of
  `routes/onboarding/screens/ChangeSettingsScreen.svelte` (Screen capture,
  Microphone, System audio, Read on-screen text, Transcription, Who's speaking,
  Semantic Search, AI features) with ONE ordered chain, so a parent and its
  children are never in different scroll positions. Model pickers and provider
  Segmenteds stay where they are — only the on/off column moves here.

  Skin of record: `docs/mockups/redesign/onboarding.html` (Change settings →
  What Mnema will do): `mx-switch` rows in source colours, tree elbows for
  dependent rows (no left bars), a stacked disk bar.

  Three rules it enforces:
   · Indentation means exactly ONE thing: child of the row above (Transcription's
     "either audio source" parentage is in its description and lock fix).
   · Hover or keyboard-focus previews the cascade BEFORE it commits: the branch
     goes dashed, every row that would move is tagged `goes off`, both totals
     print their would-be value, and the same sentence is announced on
     `aria-live`. Keyboard focus reaches parity with hover.
   · A locked row renders its FIX, never a dead switch (`lockFix` walks up to the
     first ancestor that can actually be flipped).

  All logic is imported: `feature-rules.ts` owns the graph, `feature-cost.ts`
  owns the arithmetic. This file owns pixels, copy and the two announcements.
-->
<script lang="ts">
  import { formatBytes } from "$lib/settings/state/format";
  import { costDelta, featureCost } from "./feature-cost";
  import {
    FEATURE_LABELS,
    featureNote,
    lockFix,
    preview,
    type FeatureId,
    type FeatureState,
    type TogglePreview,
  } from "./feature-rules";
  import type { ModelInventory, ModelSelections } from "./resolve-setup";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInfo from "~icons/lucide/info";

  interface Props {
    /** The live feature state — `flow.features`. */
    features: FeatureState;
    /** `flow.toggleFeature`: returns the PRE-toggle state, or null when refused. */
    onToggle: (id: FeatureId) => FeatureState | null;
    /** Undo: assign the snapshot back (`(s) => (flow.features = s)`). */
    onRestore: (snapshot: FeatureState) => void;
    /** Clear an OS permission lock — only ever called with `"microphone"`. */
    onGrant: (id: FeatureId) => void;
    /** Whether a provider + default model are actually behind the AI row. */
    aiConfigured: boolean;
    /** Jump to the AI provider setup — the soft gate's primary action. */
    onConnectAi: () => void;
    /** Real reason AI is not ready, when there is one (`ai.aiConfigMissing`). */
    aiNote?: string | null;
    /** Live model drafts, so a row's download figure is the real work-list item. */
    models?: ModelSelections | null;
    /** Live facts for each subsystem's selected model — an installed one costs 0. */
    installed?: Partial<ModelInventory> | null;
    /** Chosen capture rate, so the disk figures track the sentence's. */
    captureIntervalSeconds?: number;
    /** Pixels per frame at the draft resolution (`flow.videoPixels`). */
    videoPixels?: number;
  }

  let {
    features,
    onToggle,
    onRestore,
    onGrant,
    aiConfigured,
    onConnectAi,
    aiNote = null,
    models = null,
    installed = null,
    captureIntervalSeconds,
    videoPixels = undefined,
  }: Props = $props();

  const costCtx = $derived({ models, installed, captureIntervalSeconds, videoPixels });
  const cost = $derived(featureCost(features, costCtx));

  // ── Preview ───────────────────────────────────────────────────────────────
  /** The row the pointer or focus is on, or null. */
  let peek = $state<FeatureId | null>(null);
  /** The one polite announcement. Set explicitly (never derived) so a commit's
   *  sentence is not immediately overwritten by the preview of undoing it. */
  let live = $state("");
  /** Snapshot of the last flip that moved a row the user did not touch. Scoped
   *  to this component's lifetime, which is the round trip: the only other
   *  writer of `features` is `flow.resolve()`, and that runs while this screen
   *  is unmounted. */
  let undo = $state<{ snapshot: FeatureState; message: string } | null>(null);

  const peeked = $derived(peek ? { id: peek, p: preview(features, peek) } : null);
  const peekedAfter = $derived(peeked && !peeked.p.noop ? featureCost(peeked.p.after, costCtx) : null);

  function movesWith(id: FeatureId): boolean {
    return peeked !== null && peeked.id !== id && peeked.p.cascade.includes(id);
  }

  function setPeek(id: FeatureId | null): void {
    if (peek === id) return;
    peek = id;
    if (id === null) {
      live = "";
      return;
    }
    const p = preview(features, id);
    live = p.noop
      ? `${FEATURE_LABELS[id]} is locked — ${(p.lockReason ?? "").toLowerCase()}.`
      : sentence(id, p, "will");
  }

  // ── Actions ───────────────────────────────────────────────────────────────
  function commit(id: FeatureId): void {
    const p = preview(features, id);
    if (p.noop) {
      live = `${FEATURE_LABELS[id]} can't go on yet — ${(p.lockReason ?? "").toLowerCase()}.`;
      return;
    }
    // Both strings are built BEFORE the flip: `features` is the live state, and
    // the delta is measured from where we are to where the preview lands.
    const message = sentence(id, p, "did", false);
    const announcement = sentence(id, p, "did");
    const snapshot = onToggle(id);
    // One undo chip, for the last flip that moved a row you did not touch. A
    // later cascade-free flip CLEARS it: an undo that silently reverts two
    // flips while describing one would be worse than no undo at all.
    undo = snapshot && p.cascade.length > 0 ? { snapshot, message } : null;
    live = announcement;
    // The preview described the state we just left; drop it rather than
    // instantly re-announcing the flip back.
    peek = null;
  }

  function restore(): void {
    if (!undo) return;
    onRestore(undo.snapshot);
    undo = null;
    peek = null;
    live = "Undone.";
  }

  function grant(id: FeatureId): void {
    onGrant(id);
    peek = null;
    live = `${FEATURE_LABELS[id]} permission requested. Nothing is turned on for you.`;
  }

  function connect(): void {
    onConnectAi();
    peek = null;
    live = "Add a provider to give AI features something to answer with.";
  }

  // ── Copy ──────────────────────────────────────────────────────────────────
  /** Row copy lives in `feature-rules.ts`, beside the state it describes. */
  const describe = (id: FeatureId): string =>
    featureNote(features, id, { configured: aiConfigured, note: aiNote });

  /** Why a row is off when the user did not turn it off. */
  function cutBy(id: FeatureId): string | null {
    const noAudio = !features.microphone && !features.systemAudio;
    if (id === "ocr" && !features.screen) return "screen capture";
    if (id === "transcription" && noAudio) return "audio";
    if (id === "speakerSeparation" && !features.transcription) {
      return noAudio ? "audio" : "transcription";
    }
    return null;
  }

  function deltaWords(before: FeatureState, after: FeatureState): string {
    const d = costDelta(before, after, costCtx);
    const bits: string[] = [];
    const disk = Math.round(d.diskMbPerDay);
    if (disk !== 0) {
      bits.push(`${disk > 0 ? "costs" : "frees"} ${Math.abs(disk)} MB/day`);
    }
    if (d.downloadBytes !== 0) {
      const size = formatBytes(Math.abs(d.downloadBytes));
      bits.push(`${d.downloadBytes > 0 ? "adds" : "saves"} ${size} of download`);
    }
    return bits.join(" · ");
  }

  /** The one sentence, used for the preview, the commit and the undo chip. */
  function sentence(
    id: FeatureId,
    p: TogglePreview,
    tense: "will" | "did",
    withDelta = true,
  ): string {
    const head = `${FEATURE_LABELS[id]} ${tense === "will" ? "goes" : "went"} ${
      p.after[id] ? "on" : "off"
    }`;
    const kids = p.cascade.map(
      (f) =>
        FEATURE_LABELS[f].toLowerCase() +
        (p.after[f] === p.after[id] ? "" : ` ${p.after[f] ? "on" : "off"}`),
    );
    const goes = tense === "did" ? "went" : kids.length > 1 ? "go" : "goes";
    const tail = kids.length ? ` — and ${kids.join(" + ")} ${goes} with it.` : ".";
    const d = withDelta ? deltaWords(features, p.after) : "";
    return head + tail + (d ? ` ${d[0].toUpperCase()}${d.slice(1)}.` : "");
  }

  // ── Numbers ───────────────────────────────────────────────────────────────
  const dayLabel = (mb: number, approximate: boolean) =>
    `${approximate ? "about " : ""}${Math.round(mb)} MB/day`;
  const downloadLabel = (bytes: number, approximate: boolean) =>
    bytes > 0 ? `${approximate ? "about " : ""}${formatBytes(bytes)}` : "nothing";

  /** The disk bar, in chain order. Rows with no disk cost never get a segment;
   *  capture sources wear their source colour. */
  const BAR: readonly (readonly [FeatureId, string])[] = [
    ["screen", "var(--app-source-screen)"],
    ["ocr", "var(--chart-grey-4)"],
    ["microphone", "var(--app-source-mic)"],
    ["systemAudio", "var(--app-source-sysaudio)"],
    ["transcription", "var(--chart-grey-3)"],
    ["semanticSearch", "var(--app-info)"],
  ];
  const barWidth = (id: FeatureId) =>
    cost.diskMbPerDay > 0 ? (cost.diskByFeature[id] / cost.diskMbPerDay) * 100 : 0;

  const aiBlocked = $derived(features.aiFeatures && !aiConfigured);
</script>

{#snippet featureRow(id: FeatureId, depth: number, src: string | null)}
  {@const fix = lockFix(features, id)}
  {@const cut = cutBy(id)}
  {@const willMove = movesWith(id)}
  {@const description = describe(id)}
  {@const disk = Math.round(cost.diskByFeature[id])}
  {@const dl = cost.downloadByFeature[id]}
  <div
    class="ob-feat"
    class:off={!features[id]}
    class:kid={depth > 0}
    class:cut={depth > 0 && cut !== null}
    class:willmove={willMove}
    class:peek={peeked?.id === id}
    style:--d={depth}
    role="group"
    aria-label={FEATURE_LABELS[id]}
    onmouseenter={() => setPeek(id)}
    onfocusin={() => setPeek(id)}
  >
    <div>
      <span class="ob-feat__name">
        {#if src}<i class="ob-src" style:--c="var({src})"></i>{/if}
        {FEATURE_LABELS[id]}
        {#if willMove}
          <span class="ob-feat__carry pre">goes {peeked?.p.after[id] ? "on" : "off"}</span>
        {:else if cut}
          <span class="ob-feat__carry">off with {cut}</span>
        {/if}
      </span>
      <span class="ob-feat__desc" class:warn={fix !== null && !cut}>{description}</span>
    </div>
    <span class="ob-feat__cost">
      {[disk > 0 ? `${disk} MB/day` : "", dl > 0 ? `${formatBytes(dl)} once` : ""].filter(Boolean).join(" · ") ||
        "—"}
    </span>
    <span class="ob-feat__ctl">
      {#if fix}
        <!-- The lock renders its remedy. Never a disabled switch: a dead control
             tells the user they are stuck without telling them how to leave. -->
        <button
          class="mx-btn mx-btn--sm"
          type="button"
          onclick={() => (fix.act === "grant" ? grant(fix.id) : commit(fix.id))}
        >
          {fix.label}
        </button>
      {:else}
        <!-- The row copy is folded into the accessible NAME: indentation carries
             the dependency visually, and a screen reader must hear it in words. -->
        <input
          type="checkbox"
          role="switch"
          class="mx-switch"
          data-src={src}
          checked={features[id]}
          aria-label={`${FEATURE_LABELS[id]}. ${description}`}
          onchange={(e) => {
            e.currentTarget.checked = features[id];
            commit(id);
          }}
        />
      {/if}
    </span>
  </div>
{/snippet}

<!-- The preview follows the pointer AND the keyboard: leaving the chain by either
     route drops it. `focusout` fires before the next `focusin`, so tabbing from
     one row to the next re-arms rather than clears. -->
<div class="ob-pane">
  <div class="ob-pane__head">
    <span class="mx-kicker">What Mnema will do</span>
    <span class="ob-fine">Indented rows need the row above them.</span>
  </div>

  <div
    class="ob-chain"
    role="group"
    aria-label="What Mnema will do"
    onmouseleave={() => setPeek(null)}
    onfocusout={() => setPeek(null)}
  >
    {@render featureRow("screen", 0, "--app-source-screen")}
    {@render featureRow("ocr", 1, null)}
    {@render featureRow("microphone", 0, "--app-source-mic")}
    {@render featureRow("systemAudio", 0, "--app-source-sysaudio")}
    {@render featureRow("transcription", 1, null)}
    {@render featureRow("speakerSeparation", 2, null)}
    {@render featureRow("semanticSearch", 0, null)}
    {@render featureRow("aiFeatures", 0, null)}
  </div>

  {#if aiBlocked}
    <!-- The soft gate renders in place, under the row it is about. -->
    <div class="mx-notice" data-tone="warn">
      <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
      <b class="mx-notice__title">AI features are on with nothing to run them.</b>
      <div class="mx-notice__acts">
        <button class="mx-btn mx-btn--sm" type="button" onclick={connect}>Connect a provider</button>
        <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => commit("aiFeatures")}>
          Turn it off
        </button>
      </div>
    </div>
  {/if}

  {#if undo}
    <div class="mx-notice">
      <span class="mx-notice__icon"><IconInfo width="15" height="15" /></span>
      <b class="mx-notice__title">{undo.message}</b>
      <div class="mx-notice__acts">
        <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={restore}>Undo</button>
      </div>
    </div>
  {/if}

  <div class="ob-totals">
    <div class="ob-bar-stack" aria-hidden="true">
      {#each BAR as [id, colour] (id)}
        <i style:--c={colour} style:width="{barWidth(id).toFixed(1)}%"></i>
      {/each}
    </div>
    <div class="ob-kv">
      <span>
        disk, once it is running<b>{dayLabel(cost.diskMbPerDay, cost.approximate)}</b>
        {#if peekedAfter && Math.round(peekedAfter.diskMbPerDay) !== Math.round(cost.diskMbPerDay)}
          <b class="delta">→ {dayLabel(peekedAfter.diskMbPerDay, peekedAfter.approximate)}</b>
        {/if}
      </span>
      <span>
        download<b>{downloadLabel(cost.downloadBytes, cost.approximate)}</b>
        {#if peekedAfter && peekedAfter.downloadBytes !== cost.downloadBytes}
          <b class="delta">→ {downloadLabel(peekedAfter.downloadBytes, peekedAfter.approximate)}</b>
        {/if}
      </span>
    </div>
  </div>

  <p class="ob-sr-only" aria-live="polite">{live}</p>
</div>

<style>
  .ob-pane {
    display: grid;
    gap: var(--s-4);
  }
  .ob-pane__head {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
  }
  .ob-chain {
    display: grid;
    border-top: 1px solid var(--mx-hairline);
  }
  .ob-feat {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 170px 64px;
    align-items: center;
    gap: var(--s-3);
    min-height: 46px;
    padding: 6px 0 6px calc(var(--d, 0) * 26px);
    border-bottom: 1px solid var(--mx-hairline);
  }
  /* Tree elbow for a dependent row — hierarchy, not a selection mark. Dashed
     when its parent has cut it off, warn-dashed while a hover would move it. */
  .ob-feat.kid::before {
    content: "";
    position: absolute;
    left: calc(var(--d) * 26px - 16px);
    top: 12px;
    width: 9px;
    height: 10px;
    border: solid var(--app-border-strong);
    border-width: 0 0 1px 1px;
    border-bottom-left-radius: 4px;
  }
  .ob-feat.kid.cut::before {
    border-style: dashed;
    border-color: var(--app-text-faint);
  }
  .ob-feat.kid.willmove::before {
    border-style: dashed;
    border-color: var(--app-warn);
  }
  .ob-feat.peek {
    background: var(--mx-wash);
  }
  .ob-feat.willmove {
    background: color-mix(in srgb, var(--app-warn-bg) 60%, transparent);
  }
  .ob-feat__name {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font: 500 var(--text-md) / 1.25 var(--font-sans);
    color: var(--app-text-strong);
  }
  .ob-feat__desc {
    display: block;
    margin-top: 2px;
    font: 400 var(--text-base) / 1.35 var(--font-sans);
    color: var(--app-text-subtle);
    text-wrap: pretty;
  }
  .ob-feat__desc.warn {
    color: var(--app-warn);
  }
  .ob-feat__carry {
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    letter-spacing: 0.02em;
  }
  .ob-feat__carry.pre {
    color: var(--app-warn);
  }
  .ob-feat__cost {
    justify-self: end;
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ob-feat__ctl {
    justify-self: end;
    display: flex;
  }
  .ob-feat.off .ob-feat__name {
    color: var(--app-text-muted);
  }
  .ob-src {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    flex: none;
    background: var(--c, var(--chart-grey-3));
  }
  /* Capture-source switches wear their source colour. */
  .mx-switch[data-src="--app-source-screen"]:checked {
    background: var(--app-source-screen);
  }
  .mx-switch[data-src="--app-source-mic"]:checked {
    background: var(--app-source-mic);
  }
  .mx-switch[data-src="--app-source-sysaudio"]:checked {
    background: var(--app-source-sysaudio);
  }
  /* A row that is about to move says so on its switch too. */
  .ob-feat.willmove .mx-switch:checked {
    outline: 1px dashed var(--app-warn-border);
    outline-offset: 2px;
  }

  .ob-totals {
    display: grid;
    gap: var(--s-2);
  }
  .ob-bar-stack {
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
  .ob-kv {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-6);
    font: 400 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ob-kv b {
    margin-left: 6px;
    font: 500 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .ob-kv b.delta {
    color: var(--app-warn);
    font-weight: 400;
  }
</style>
