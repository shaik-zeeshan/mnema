<script lang="ts">
  // Identity repair as a right-edge slide-over — never a modal, never a centred
  // popover over the text, because the whole point of a repair surface is that
  // you can keep reading the lines you are trying to attribute.
  //
  // One search box does both jobs: it filters the saved people and offers to
  // create the typed name as a new one. One primary commits whatever is picked.
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInbox from "~icons/lucide/inbox";
  import IconPlay from "~icons/lucide/play";
  import IconSearch from "~icons/lucide/search";
  import IconX from "~icons/lucide/x";
  import Input from "$lib/components/Input.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { tip } from "$lib/components/tooltip";
  import {
    clusterSummaryLabel,
    embeddingCountLabel,
    isDefaultSpeakerLabel,
    validateSpeakerName,
    type SpeakerMark,
    type SpeakerTranscriptGroup,
  } from "./audio-drawer-view";
  import SpeakerMarkGlyph from "./SpeakerMark.svelte";
  import type {
    PersonProfileDto,
    SpeakerClusterDto,
    SpeakerTurnDto,
  } from "$lib/types/app-infra";

  interface Props {
    group: SpeakerTranscriptGroup;
    mark: SpeakerMark | undefined;
    /** The drawer's cluster → mark map, for the move-line list's glyphs. */
    marks?: Map<number, SpeakerMark>;
    turns: SpeakerTurnDto[];
    clusters: SpeakerClusterDto[];
    profiles: PersonProfileDto[];
    /** The name shown for this cluster today (linked person, else its label). */
    persistedName: string;
    unnamed: boolean;
    busy: boolean;
    error: string | null;
    /** Clusters in this segment still without a person — the "what's next" count. */
    unnamedRemaining: number;
    linkedPersonName: string | null;
    /** A recognition suggestion is pending (vs. an already-linked person). */
    suggestionPending: boolean;
    mergeTargetLabel: string | null;
    /** `0.71`-style centroid similarity, when developer options expose it. */
    mergeScoreLabel: string | null;
    clusterOptionLabel: (cluster: SpeakerClusterDto) => string;
    onClose: () => void;
    /** Create a saved person with this name and link this cluster to it. */
    onApplyName: (name: string) => void;
    /** Link to a saved person. Confirming a suggestion is this same write
     *  (`confirm_speaker_recognition_suggestion` is a link to the suggested id). */
    onLink: (personId: number) => void;
    onMerge: () => void;
    /** Reject a pending suggestion, or unlink a confirmed person — whichever this
     *  cluster's current state calls for. Per-cluster, never global. */
    onNotThisPerson: () => void;
    onMoveGroupTo: (targetClusterId: number) => void;
    /** Bounded 8s previews: this cluster, then the merge candidate. */
    onPlaySamples: () => void;
  }

  let {
    group,
    mark,
    marks,
    turns,
    clusters,
    profiles,
    persistedName,
    unnamed,
    busy,
    error,
    unnamedRemaining,
    linkedPersonName,
    suggestionPending,
    mergeTargetLabel,
    mergeScoreLabel,
    clusterOptionLabel,
    onClose,
    onApplyName,
    onLink,
    onMerge,
    onNotThisPerson,
    onMoveGroupTo,
    onPlaySamples,
  }: Props = $props();

  // ponytail: no undo toast. Every speaker write here — create/link/unlink,
  // merge_speaker_clusters, move_speaker_turn_to_cluster — has NO backend inverse,
  // so an "Undo" would be invented state. Add it when the Rust side ships one.

  type Pick = number | "new" | null;
  let scope = $state<"speaker" | "line">("speaker");
  let query = $state("");
  let pick = $state<Pick>(null);
  let moveChoice = $state("");

  // The drawer reuses one panel instance across gutter clicks: reset per cluster.
  let seededClusterId = $state<number | null>(null);
  $effect(() => {
    if (seededClusterId === group.clusterId) return;
    seededClusterId = group.clusterId;
    scope = "speaker";
    query = "";
    pick = null;
    moveChoice = "";
  });

  const first = (name: string) => name.split(" ")[0] ?? name;
  const initials = (name: string) =>
    name
      .split(/\s+/)
      .map((part) => part[0] ?? "")
      .join("")
      .slice(0, 2)
      .toUpperCase();

  const summary = $derived(clusterSummaryLabel(group.clusterId, turns));
  const turnCount = $derived(turns.filter((t) => t.clusterId === group.clusterId).length);
  const cluster = $derived(clusters.find((c) => c.id === group.clusterId));
  const linked = $derived(profiles.find((p) => p.id === group.personId) ?? null);
  const suggested = $derived(
    suggestionPending ? (profiles.find((p) => p.id === group.suggestedPersonId) ?? null) : null,
  );

  const q = $derived(query.trim());
  const warning = $derived(q ? validateSpeakerName(q).message : null);
  const people = $derived(
    profiles.filter(
      (p) => p.id !== group.personId && (!q || p.displayName.toLowerCase().includes(q.toLowerCase())),
    ),
  );
  const canCreate = $derived(
    q !== "" &&
      warning == null &&
      !profiles.some((p) => p.displayName.toLowerCase() === q.toLowerCase()),
  );
  // A pick that the filter (or a landed write) took away is no pick; a typed new
  // name with nothing else matching picks itself.
  const effectivePick = $derived<Pick>(
    pick === "new"
      ? canCreate
        ? "new"
        : null
      : pick != null && people.some((p) => p.id === pick)
        ? pick
        : canCreate && people.length === 0
          ? "new"
          : null,
  );
  const pickedPerson = $derived(
    typeof effectivePick === "number" ? profiles.find((p) => p.id === effectivePick) : undefined,
  );
  const moveTargets = $derived(clusters.filter((c) => c.id !== group.clusterId));

  const commit = $derived.by(() => {
    if (scope === "line") {
      return { label: "Move this line", busyLabel: "Moving…", ready: moveChoice !== "" };
    }
    if (pickedPerson) {
      return { label: `Link to ${pickedPerson.displayName}`, busyLabel: "Linking…", ready: true };
    }
    return { label: "Name & apply", busyLabel: "Saving…", ready: effectivePick === "new" };
  });

  // The mockup closes the panel once a commit or a confirm lands; a failed write
  // keeps it open with the error. `busy` flips true synchronously when the write
  // starts, so "busy went false with no error" is the success signal.
  let closeWhenDone = false;
  let wasBusy = false;
  /** What "Try again" re-runs; cleared once a write lands. */
  let lastAction = $state<(() => void) | null>(null);
  $effect(() => {
    if (busy) {
      wasBusy = true;
      return;
    }
    if (!wasBusy) return;
    wasBusy = false;
    if (error) return;
    lastAction = null;
    if (closeWhenDone) onClose();
  });

  function run(action: () => void, closeOnSuccess = false): void {
    if (busy) return;
    lastAction = action;
    closeWhenDone = closeOnSuccess;
    action();
  }

  function submit(): void {
    if (!commit.ready) return;
    if (scope === "line") {
      const target = Number(moveChoice);
      run(() => onMoveGroupTo(target), true);
    } else if (effectivePick === "new") {
      const name = q;
      run(() => onApplyName(name), true);
    } else if (pickedPerson) {
      const id = pickedPerson.id;
      run(() => onLink(id), true);
    }
  }
</script>

<div
  class="fix"
  role="dialog"
  tabindex="-1"
  aria-label={`Speaker repair — ${persistedName}`}
  aria-busy={busy}
  onpointerdown={(event) => event.stopPropagation()}
>
  <div class="fix__head">
    <SpeakerMarkGlyph {mark} ghosted={unnamed} />
    <span class="fix__nm">{persistedName}</span>
    {#if linked?.isAccountOwner}<span class="mx-chip">you</span>{/if}
    {#if cluster?.personLinkAuto && group.personId != null}
      <span class="mx-chip" use:tip={"Linked automatically from your voiceprint"}>auto</span>
    {/if}
    <span class="mx-spacer"></span>
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
      aria-label="Close speaker repair"
      onclick={onClose}><IconX width="15" height="15" /></button
    >
  </div>
  <div class="fix__sum">{summary}</div>

  <div class="fix__body">
    {#if suggested}
      <div class="fix__card">
        <div class="fix__card-h">
          Sounds like {suggested.displayName}
          {#if group.recognitionConfidence}
            <span class="mx-chip" use:tip={"Recognition confidence"}
              >{group.recognitionConfidence}</span
            >
          {/if}
        </div>
        <p>
          Only a suggestion. Confirm links this voice and saves a sample; “Not {first(
            suggested.displayName,
          )}” stops suggesting {first(suggested.displayName)} for this voice only.
        </p>
        <div class="fix__acts">
          <button
            type="button"
            class="mx-btn mx-btn--sm"
            disabled={busy}
            onclick={() => {
              const id = suggested.id;
              run(() => onLink(id), true);
            }}
            ><IconCheck width="13" height="13" />Confirm {first(suggested.displayName)}</button
          >
          <button
            type="button"
            class="mx-btn mx-btn--danger mx-btn--sm"
            disabled={busy}
            onclick={() => run(onNotThisPerson)}>Not {first(suggested.displayName)}</button
          >
        </div>
      </div>
    {/if}

    {#if mergeTargetLabel && scope === "speaker"}
      <div class="fix__card">
        <div class="fix__card-h">Possibly the same voice as {mergeTargetLabel}</div>
        <p>
          {mergeScoreLabel ? `Centroid similarity ${mergeScoreLabel}. ` : ""}Over-segmentation is
          the common failure — one person split in two. Merging folds every turn of this voice into
          theirs.
        </p>
        <div class="fix__acts">
          <button
            type="button"
            class="mx-btn mx-btn--sm"
            disabled={busy}
            onclick={() => run(onMerge, true)}>Merge them</button
          >
          <button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            disabled={busy}
            onclick={onPlaySamples}><IconPlay width="11" height="11" />Play 8s of each</button
          >
        </div>
      </div>
    {/if}

    <div class="fix__field">
      <div class="fix__row">
        <span class="mx-label">Apply to</span>
        <Segmented
          ariaLabel="Repair scope"
          disabled={busy}
          value={scope}
          onValueChange={(v) => (scope = v === "line" ? "line" : "speaker")}
          options={[
            { value: "speaker", label: "This speaker" },
            { value: "line", label: "This line only" },
          ]}
        />
      </div>
      <p class="fix__hint">
        {scope === "speaker"
          ? `Every turn this voice holds — ${turnCount} in this segment.`
          : "Naming is always speaker-wide, so a single line moves to another speaker instead."}
      </p>
    </div>

    {#if scope === "speaker"}
      <form
        class="fix__field"
        onsubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label class="mx-label" for={`repair-q-${group.clusterId}`}
          >Name this voice, or find a saved person</label
        >
        <Input
          id={`repair-q-${group.clusterId}`}
          placeholder="Type a name…"
          bind:value={query}
          disabled={busy}
          invalid={warning != null}
          errorId={`repair-warn-${group.clusterId}`}
        />
        {#if warning}
          <p class="mx-inline fix__warn" data-tone="warn" id={`repair-warn-${group.clusterId}`}>
            {warning}
          </p>
        {/if}
        <div class="mx-choice mx-choice--list" role="radiogroup" aria-label="Saved people">
          {#each people as person (person.id)}
            {@const samples = embeddingCountLabel(person.embeddingCount)}
            <label class="mx-choice__opt">
              <input
                type="radio"
                name={`repair-pick-${group.clusterId}`}
                checked={effectivePick === person.id}
                disabled={busy}
                onchange={() => (pick = person.id)}
              />
              <span class="mx-choice__icon" aria-hidden="true">{initials(person.displayName)}</span>
              <span class="mx-choice__title"
                >{person.displayName}<span class="mx-choice__tag"
                  >{person.isAccountOwner ? (samples ? "you · " : "you") : ""}{samples}</span
                ></span
              >
            </label>
          {/each}
          {#if canCreate}
            <label class="mx-choice__opt">
              <input
                type="radio"
                name={`repair-pick-${group.clusterId}`}
                checked={effectivePick === "new"}
                disabled={busy}
                onchange={() => (pick = "new")}
              />
              <span class="mx-choice__icon" aria-hidden="true">+</span>
              <span class="mx-choice__title"
                >Create “{q}”<span class="mx-choice__tag">new person</span></span
              >
            </label>
          {/if}
          {#if people.length === 0 && !canCreate}
            {#if profiles.length > 0}
              <span class="mx-inline fix__nomatch"
                ><IconSearch width="13" height="13" />No saved person matches “{q}”.</span
              >
            {:else}
              <div class="mx-empty mx-empty--compact">
                <span class="mx-empty__glyph"><IconInbox width="14" height="14" /></span>
                <b class="mx-empty__title">No saved people yet</b>
                <p class="mx-empty__text">Type a name to save this voice as the first one.</p>
              </div>
            {/if}
          {/if}
        </div>
      </form>

      {#if linkedPersonName}
        <div class="fix__field">
          <div>
            <button
              type="button"
              class="mx-btn mx-btn--danger mx-btn--sm"
              disabled={busy}
              onclick={() => run(onNotThisPerson)}>Unlink {first(linkedPersonName)}</button
            >
          </div>
          <p class="fix__hint">
            Unlinks this voice only. {first(linkedPersonName)}’s saved voice and every other
            recording stay as they are.
          </p>
        </div>
      {/if}
    {:else}
      <div class="fix__field">
        <span class="mx-label">Move this line to</span>
        <div class="mx-choice mx-choice--list" role="radiogroup" aria-label="Move this line to">
          {#each moveTargets as target (target.id)}
            {@const n = turns.filter((t) => t.clusterId === target.id).length}
            <label class="mx-choice__opt">
              <input
                type="radio"
                name={`repair-move-${group.clusterId}`}
                value={String(target.id)}
                bind:group={moveChoice}
                disabled={busy}
              />
              {#if marks?.get(target.id)}
                <span class="mx-choice__icon" aria-hidden="true"
                  ><SpeakerMarkGlyph
                    mark={marks.get(target.id)}
                    ghosted={target.personId == null && isDefaultSpeakerLabel(target.speakerLabel)}
                  /></span
                >
              {/if}
              <span class="mx-choice__title">{clusterOptionLabel(target)}</span>
              <span class="mx-choice__desc"
                >{n ? `${n} turn${n === 1 ? "" : "s"} in this segment` : "elsewhere in this session"}</span
              >
            </label>
          {/each}
        </div>
      </div>
    {/if}
  </div>

  {#if error}
    <div class="fix__err">
      <span class="mx-inline" data-tone="danger" role="alert"
        ><IconAlert width="13" height="13" />{error}
        {#if lastAction}
          <button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            disabled={busy}
            onclick={() => lastAction && run(lastAction, closeWhenDone)}>Try again</button
          >
        {/if}
      </span>
    </div>
  {/if}

  <div class="fix__foot">
    <button
      type="button"
      class="mx-btn mx-btn--primary"
      disabled={busy || !commit.ready}
      onclick={submit}
    >
      {#if busy}<span class="mx-spin mx-spin--sm"></span>{commit.busyLabel}{:else}{commit.label}{/if}
    </button>
    {#if unnamedRemaining > 0}
      <span class="fix__left">{unnamedRemaining} more unnamed here</span>
    {/if}
  </div>
</div>

<style>
  .fix {
    position: absolute;
    z-index: 8;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(352px, 92%);
    display: flex;
    flex-direction: column;
    background: var(--app-surface-raised);
    border-left: 1px solid var(--app-border-strong);
    box-shadow: var(--app-shadow-popover);
    animation: fix-in var(--t-med) var(--ease-expo) both;
  }

  @keyframes fix-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }

  .fix[aria-busy="true"] {
    cursor: progress;
  }

  .fix__head {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 10px 8px 0 16px;
  }

  .fix__nm {
    font: 600 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .fix__sum {
    padding: 4px 16px 10px 37px;
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .fix__body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: grid;
    align-content: start;
    gap: 14px;
    padding: 14px 16px 16px;
  }

  .fix__field {
    display: grid;
    gap: 7px;
    margin: 0;
  }

  .fix__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .fix__hint,
  .fix__warn {
    margin: 0;
    font: 400 var(--text-base) / 1.45 var(--font-sans);
    color: var(--app-text-subtle);
  }

  .fix__warn {
    color: var(--app-warn);
  }

  .fix__nomatch {
    padding: 8px 2px;
  }

  .fix :global(.mx-input) {
    width: 100%;
  }

  .fix .mx-choice--list .mx-choice__opt {
    padding: 7px 40px 7px 10px;
    column-gap: 10px;
  }

  .fix .mx-choice--list .mx-choice__icon {
    width: 24px;
    height: 24px;
    border-radius: var(--r-sm);
    font: 600 var(--text-sm) / 1 var(--font-sans);
  }

  .fix .mx-choice__title {
    font-size: var(--text-md);
  }

  .fix .mx-choice__desc {
    font-size: var(--text-sm);
  }

  .fix .mx-choice__opt:not(:has(.mx-choice__desc)) .mx-choice__icon {
    grid-row: auto;
  }

  .fix .mx-choice__title .mx-choice__tag {
    margin-left: auto;
    font-weight: 400;
  }

  .fix .mx-choice--list .mx-choice__opt::before {
    right: 12px;
  }

  .fix .mx-choice--list .mx-choice__opt::after {
    right: 17.5px;
  }

  .fix__card {
    display: grid;
    gap: 8px;
    padding: 12px;
    border-radius: var(--r-md);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
  }

  .fix__card-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 600 var(--text-md) / 1.3 var(--font-sans);
    color: var(--app-text-strong);
  }

  .fix__card p {
    margin: 0;
    font: 400 var(--text-base) / 1.45 var(--font-sans);
    color: var(--app-text-muted);
  }

  .fix__acts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 2px;
  }

  .fix__err {
    padding: 0 16px 10px;
    word-break: break-word;
  }

  .fix__foot {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-top: 1px solid var(--mx-hairline);
  }

  .fix__left {
    font: 400 var(--text-sm) / 1.3 var(--font-mono);
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    .fix {
      animation: none;
    }
  }
</style>
