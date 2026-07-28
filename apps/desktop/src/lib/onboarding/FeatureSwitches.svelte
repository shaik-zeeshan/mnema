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

  Three rules the mockup exists to enforce:
   · Indentation means exactly ONE thing: child of the row above. Transcription
     has a second parent (either audio source), and that is drawn as a bracket in
     the gutter — a different visual axis, so neither has to be decoded.
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
  import Switch from "$lib/components/Switch.svelte";
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
  }: Props = $props();

  const costCtx = $derived({ models, installed, captureIntervalSeconds });
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

  /** The disk bar, in chain order. Rows with no disk cost never get a segment.
   *  Colours ride the style attribute (tokens, never a literal) so there is no
   *  dynamic class for Svelte's CSS pruner to guess at. */
  const BAR: readonly (readonly [FeatureId, string])[] = [
    ["screen", "var(--app-accent-strong)"],
    ["ocr", "var(--app-accent)"],
    ["microphone", "var(--app-info)"],
    ["systemAudio", "var(--app-info)"],
    ["transcription", "var(--app-text-subtle)"],
    ["semanticSearch", "var(--app-warn)"],
  ];
  const barWidth = (id: FeatureId) =>
    cost.diskMbPerDay > 0 ? (cost.diskByFeature[id] / cost.diskMbPerDay) * 100 : 0;

  const aiBlocked = $derived(features.aiFeatures && !aiConfigured);
</script>

{#snippet featureRow(id: FeatureId)}
  {@const fix = lockFix(features, id)}
  {@const cut = cutBy(id)}
  {@const willMove = movesWith(id)}
  {@const description = describe(id)}
  <div
    class="row"
    class:dead={fix !== null}
    class:willmove={willMove}
    class:peek={peeked?.id === id}
    role="group"
    aria-label={FEATURE_LABELS[id]}
    onmouseenter={() => setPeek(id)}
    onfocusin={() => setPeek(id)}
  >
    <div>
      <div class="t">
        {FEATURE_LABELS[id]}
        {#if willMove}
          <span class="carry pre">goes {peeked?.p.after[id] ? "on" : "off"}</span>
        {:else if cut}
          <span class="carry">off with {cut}</span>
        {/if}
      </div>
      <div class="d" class:warn={fix !== null && !cut}>{description}</div>
    </div>

    <div class="cost">
      {#if cost.diskByFeature[id] > 0}
        <span class="ob-num">{Math.round(cost.diskByFeature[id])} MB/day</span>
      {/if}
      {#if cost.downloadByFeature[id] > 0}
        {#if cost.diskByFeature[id] > 0}<span class="sep">·</span>{/if}
        <span class="dl ob-num">{formatBytes(cost.downloadByFeature[id])} once</span>
      {/if}
      {#if cost.diskByFeature[id] <= 0 && cost.downloadByFeature[id] <= 0}
        <span class="zero">—</span>
      {/if}
    </div>

    {#if fix}
      <!-- The lock renders its remedy. Never a disabled switch: a dead control
           tells the user they are stuck without telling them how to leave. -->
      <button
        class="ob-btn sm"
        type="button"
        onclick={() => (fix.act === "grant" ? grant(fix.id) : commit(fix.id))}
      >
        {fix.label}
      </button>
    {:else}
      <!-- The row copy is folded into the accessible NAME: indentation carries
           the dependency visually, and a screen reader must hear it in words. -->
      <Switch
        checked={features[id]}
        ariaLabel={`${FEATURE_LABELS[id]}. ${description}`}
        onCheckedChange={() => commit(id)}
      />
    {/if}
  </div>
{/snippet}

<!-- The preview follows the pointer AND the keyboard: leaving the chain by either
     route drops it. `focusout` fires before the next `focusin`, so tabbing from
     one row to the next re-arms rather than clears. -->
<div
  class="chain"
  role="group"
  aria-label="What Mnema will do"
  onmouseleave={() => setPeek(null)}
  onfocusout={() => setPeek(null)}
>
  <div class="chain-head">
    <span class="ob-m">What Mnema will do</span>
    <span class="ob-fine">Indented rows need the row above them.</span>
  </div>

  {@render featureRow("screen")}
  <div
    class="kid"
    class:cut={!features.screen}
    class:warnpeek={movesWith("ocr")}
  >
    {@render featureRow("ocr")}
  </div>

  <!-- Microphone and system audio are ROOTS that share one child, so the pair is
       bracketed in the gutter rather than indented. Two relationships, two axes. -->
  <div
    class="grp"
    class:cut={!features.microphone && !features.systemAudio}
    class:warnpeek={movesWith("transcription")}
  >
    {@render featureRow("microphone")}
    {@render featureRow("systemAudio")}
  </div>
  <div
    class="kid"
    class:cut={!features.microphone && !features.systemAudio}
    class:warnpeek={movesWith("transcription")}
  >
    {@render featureRow("transcription")}
  </div>
  <div
    class="kid k2"
    class:cut={!features.transcription}
    class:warnpeek={movesWith("speakerSeparation")}
  >
    {@render featureRow("speakerSeparation")}
  </div>

  {@render featureRow("semanticSearch")}
  {@render featureRow("aiFeatures")}

  {#if aiBlocked}
    <!-- The soft gate renders in place, under the row it is about. -->
    <div class="gate">
      AI features are on with nothing to run them.
      <span class="grow"></span>
      <button class="ob-btn sm" type="button" onclick={connect}>Connect a provider</button>
      <button class="ob-btn sm" type="button" onclick={() => commit("aiFeatures")}>
        Turn it off
      </button>
    </div>
  {/if}

  {#if undo}
    <div class="undo">
      {undo.message}
      <span class="grow"></span>
      <button class="ob-btn sm" type="button" onclick={restore}>Undo</button>
    </div>
  {/if}

  <div class="totals">
    <div class="bar">
      {#each BAR as [id, colour] (id)}
        <span class="seg" style={`width:${barWidth(id).toFixed(1)}%;background:${colour}`}
        ></span>
      {/each}
    </div>
    <div class="kv">
      <span>disk, once it is running</span>
      <b>
        {dayLabel(cost.diskMbPerDay, cost.approximate)}
        {#if peekedAfter && Math.round(peekedAfter.diskMbPerDay) !== Math.round(cost.diskMbPerDay)}
          <span class="delta">
            → {dayLabel(peekedAfter.diskMbPerDay, peekedAfter.approximate)}
          </span>
        {/if}
      </b>
    </div>
    <div class="kv second">
      <span>to download before it works</span>
      <b>
        {downloadLabel(cost.downloadBytes, cost.approximate)}
        {#if peekedAfter && peekedAfter.downloadBytes !== cost.downloadBytes}
          <span class="delta">
            → {downloadLabel(peekedAfter.downloadBytes, peekedAfter.approximate)}
          </span>
        {/if}
      </b>
    </div>
  </div>

  <p class="chain-live ob-fine" aria-live="polite">{live}</p>
</div>

<style>
  .chain {
    max-width: 100%;
  }
  .chain-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 10px;
    flex-wrap: wrap;
  }

  /* ── rows ───────────────────────────────────────────────────────────────── */
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 12px;
    align-items: center;
    padding: 7px 0;
  }
  .row .t {
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .row .d {
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-text-subtle);
    margin-top: 2px;
    /* 78ch, the mockup's frame measure (`revision-2.html:172`). At 46ch every
       row wrapped to two lines and the chain overflowed its section at the
       1120x800 DESIGN size, not just the 920x620 minimum. */
    max-width: 78ch;
  }
  .row.dead .t {
    color: var(--app-text-muted);
  }
  .row.dead .d {
    color: var(--app-text-faint);
  }
  /* The lock reason is the one line on a dead row that must stay readable. */
  .row .d.warn,
  .row.dead .d.warn {
    color: var(--app-warn);
  }

  .cost {
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    text-align: right;
    white-space: nowrap;
  }
  .cost .zero {
    color: var(--app-text-faint);
  }
  .cost .dl {
    color: var(--app-info);
  }

  /* ── the spine ──────────────────────────────────────────────────────────── */
  .kid {
    padding-left: 26px;
    position: relative;
  }
  .kid::before {
    content: "";
    position: absolute;
    left: 9px;
    top: 0;
    bottom: 50%;
    width: 10px;
    border-left: 1px solid var(--app-border-strong);
    border-bottom: 1px solid var(--app-border-strong);
    border-bottom-left-radius: 6px;
    transition: border-color 0.2s ease;
  }
  .kid.k2 {
    padding-left: 48px;
  }
  .kid.k2::before {
    left: 31px;
  }
  .kid.cut::before,
  .kid.warnpeek::before {
    border-left-style: dashed;
    border-bottom-style: dashed;
  }
  .kid.cut::before {
    border-color: var(--app-text-faint);
  }
  .kid.warnpeek::before {
    border-color: var(--app-warn);
  }

  /* The audio pair is a BRACKET in the gutter, not an indent: indentation means
     "child of the row above", and these two are roots that share one child. */
  .grp {
    border-left: 1px solid var(--app-border-strong);
    margin: 2px 0 2px -12px;
    padding-left: 11px;
    transition: border-color 0.2s ease;
  }
  .grp.cut {
    border-left-color: var(--app-text-faint);
    border-left-style: dashed;
  }
  .grp.warnpeek {
    border-left-color: var(--app-warn);
    border-left-style: dashed;
  }

  /* ── tags ───────────────────────────────────────────────────────────────── */
  .carry {
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .carry.pre {
    color: var(--app-warn);
    border: 1px solid var(--app-warn-border);
    background: var(--app-warn-bg);
    border-radius: 4px;
    padding: 0 5px;
  }
  .row.peek {
    background: var(--app-surface-hover);
  }
  .row.willmove {
    background: var(--app-warn-bg);
    box-shadow: inset 2px 0 0 var(--app-warn);
  }
  .row.peek,
  .row.willmove {
    border-radius: 6px;
    margin: 0 -8px;
    padding-left: 8px;
    padding-right: 8px;
  }
  /* A row that is about to move says so on its switch too. */
  .row.willmove :global(.switch-track[data-state="checked"]) {
    border-color: var(--app-warn-border);
    border-style: dashed;
  }

  /* ── gate, undo, totals ─────────────────────────────────────────────────── */
  .gate,
  .undo {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    font-size: var(--text-sm);
  }
  .gate {
    color: var(--app-warn);
    border: 1px solid var(--app-warn-border);
    background: var(--app-warn-bg);
    border-radius: 6px;
    padding: 7px 10px;
    margin: 4px 0 6px;
  }
  .undo {
    color: var(--app-text-muted);
    border: 1px dashed var(--app-border-strong);
    border-radius: 6px;
    padding: 6px 10px;
    margin: 8px 0 0;
  }
  .grow {
    margin-left: auto;
  }

  .totals {
    margin-top: 12px;
    border-top: 1px solid var(--app-border);
    padding-top: 10px;
  }
  .kv {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
  }
  .kv.second {
    margin-top: 3px;
  }
  .kv b {
    color: var(--app-text-strong);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .kv .delta {
    color: var(--app-warn);
    font-weight: 400;
  }

  .bar {
    height: 6px;
    border-radius: 999px;
    background: var(--app-border);
    overflow: hidden;
    display: flex;
    margin: 0 0 8px;
  }
  .seg {
    display: block;
    height: 100%;
    transition: width 0.22s ease;
  }

  .chain-live {
    min-height: 1.7em;
    margin: 8px 0 0;
    color: var(--app-text-muted);
  }

  /* No breakpoint: the name column is minmax(0, 1fr), so a narrower pane is
     absorbed by wrapping the description. Cost and switch stay on the line —
     stacking them made every row three lines tall for no gain. */
  @media (prefers-reduced-motion: reduce) {
    .kid::before,
    .grp,
    .seg {
      transition: none;
    }
  }
</style>
