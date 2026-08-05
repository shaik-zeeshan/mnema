<script lang="ts">
  // Identity repair as a right-edge slide-over — never a modal, never a centred
  // popover over the text, because the whole point of a repair surface is that
  // you can keep reading the lines you are trying to attribute. This replaces the
  // old top-layer `popover="manual"` speaker-actions panel.
  import {
    clusterSummaryLabel,
    embeddingCountLabel,
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
    turns: SpeakerTurnDto[];
    clusters: SpeakerClusterDto[];
    profiles: PersonProfileDto[];
    /** The name already persisted for this cluster (the field's initial value). */
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
    onApplyName: (name: string) => void;
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

  // ponytail: no undo toast (mockup region K). Every speaker write here —
  // name_speaker_cluster, link/unlink, merge_speaker_clusters,
  // move_speaker_turn_to_cluster — has NO backend inverse, so an "Undo" would be
  // invented state with real data-loss risk the moment it drifted from the DB.
  // Add it when the Rust side ships a real inverse (e.g. a per-write journal the
  // undo replays), not before.

  let nameDraft = $state("");
  let scope = $state<"speaker" | "line">("speaker");
  let linkChoice = $state("");
  let moveChoice = $state("");

  // Seed on mount, and re-seed whenever the panel is pointed at a different
  // cluster (the drawer reuses one panel instance across gutter clicks).
  let seededClusterId = $state<number | null>(null);
  let seededName = $state("");
  $effect(() => {
    if (seededClusterId !== group.clusterId) {
      seededClusterId = group.clusterId;
      seededName = persistedName;
      nameDraft = persistedName;
      scope = "speaker";
      linkChoice = "";
      moveChoice = "";
      return;
    }
    // Same cluster, but the persisted name moved under us — which is what a landed
    // write looks like from here, above all an unlink/reject: the cluster loses its
    // person and `persistedName` reverts to the diarizer label. The field has to show
    // what the DB says NOW, or it keeps offering the person the user just vetoed, one
    // Enter from re-applying that name as a cluster label. A draft the user typed
    // themselves is theirs, and survives.
    if (persistedName === seededName) return;
    const untouched = nameDraft === seededName;
    seededName = persistedName;
    if (untouched) nameDraft = persistedName;
  });

  const validation = $derived(validateSpeakerName(nameDraft));
  const nameChanged = $derived(nameDraft.trim() !== persistedName.trim());
  const applyDisabled = $derived(busy || !validation.ok || !nameChanged);
  const summary = $derived(clusterSummaryLabel(group.clusterId, turns));
  const linkable = $derived(profiles.filter((p) => p.id !== group.personId));
  const moveTargets = $derived(clusters.filter((c) => c.id !== group.clusterId));
  /** The person "Not this person" is actually about. */
  const rejectSubject = $derived(
    linkedPersonName ??
      (group.suggestedPersonId != null
        ? profiles.find((p) => p.id === group.suggestedPersonId)?.displayName ?? null
        : null),
  );

  function submitName(event: SubmitEvent): void {
    event.preventDefault();
    if (applyDisabled) return;
    onApplyName(nameDraft.trim());
  }
</script>

<div
  class="so"
  role="dialog"
  tabindex="-1"
  aria-label={`Speaker repair — ${summary}`}
  aria-busy={busy}
  onpointerdown={(event) => event.stopPropagation()}
>
  <div class="so__head">
    <SpeakerMarkGlyph {mark} ghosted={unnamed} />
    <span class="so__t">{summary}</span>
    <span class="so__grow"></span>
    <button type="button" class="so__close" aria-label="Close speaker repair" onclick={onClose}
      >✕</button
    >
  </div>

  <div class="so__body">
    <form onsubmit={submitName}>
      <label class="lbl" for={`repair-name-${group.clusterId}`}>name this voice</label>
      <input
        id={`repair-name-${group.clusterId}`}
        class="inp"
        class:is-error={scope === "speaker" && !validation.ok}
        bind:value={nameDraft}
        disabled={busy || scope === "line"}
        aria-invalid={scope === "speaker" && !validation.ok}
        aria-describedby={`repair-warn-${group.clusterId} repair-scope-${group.clusterId}`}
      />
      {#if scope === "speaker" && validation.message}
        <p class="fieldwarn" id={`repair-warn-${group.clusterId}`}>
          <span aria-hidden="true">▲</span>
          <span>{validation.message}</span>
        </p>
      {:else}
        <p class="fieldwarn" id={`repair-warn-${group.clusterId}`} hidden></p>
      {/if}
    </form>

    <div>
      <span class="lbl">apply to</span>
      <div class="scope" role="group" aria-label="Rename scope">
        <button
          type="button"
          aria-pressed={scope === "speaker"}
          onclick={() => (scope = "speaker")}>this speaker</button
        >
        <button type="button" aria-pressed={scope === "line"} onclick={() => (scope = "line")}
          >this line only</button
        >
      </div>
      <p class="hint" id={`repair-scope-${group.clusterId}`}>
        {#if scope === "speaker"}
          Cluster-wide: every turn this voice holds in the segment.
        {:else}
          Naming is always cluster-wide, so this line gets moved to another speaker
          instead.
        {/if}
      </p>
    </div>

    {#if scope === "speaker"}
      <div>
        <label class="lbl" for={`repair-link-${group.clusterId}`}>or link to a saved person</label>
        <select
          id={`repair-link-${group.clusterId}`}
          class="inp"
          disabled={busy || linkable.length === 0}
          bind:value={linkChoice}
          onchange={() => {
            const id = Number(linkChoice);
            if (Number.isFinite(id) && id > 0) onLink(id);
          }}
        >
          <option value="">— none —</option>
          {#each linkable as profile (profile.id)}
            {@const samples = embeddingCountLabel(profile.embeddingCount)}
            <option value={String(profile.id)}
              >{profile.displayName}{samples ? ` · ${samples}` : ""}</option
            >
          {/each}
        </select>
      </div>
    {:else}
      <div>
        <label class="lbl" for={`repair-move-${group.clusterId}`}>move this line to</label>
        <select
          id={`repair-move-${group.clusterId}`}
          class="inp"
          disabled={busy || moveTargets.length === 0}
          bind:value={moveChoice}
        >
          <option value="">— pick a speaker —</option>
          {#each moveTargets as cluster (cluster.id)}
            <option value={String(cluster.id)}>{clusterOptionLabel(cluster)}</option>
          {/each}
        </select>
      </div>
    {/if}

    {#if mergeTargetLabel}
      <div class="card card--warn">
        <div class="card__h">Possibly the same voice as {mergeTargetLabel}</div>
        <div class="card__d">
          {mergeScoreLabel ? `Centroid similarity ${mergeScoreLabel}. ` : ""}Over-segmentation is
          the common failure — one person split in two.
        </div>
        <div class="row row--card">
          <button type="button" class="btn btn--primary" disabled={busy} onclick={onMerge}
            >Merge them</button
          >
          <button type="button" class="btn" disabled={busy} onclick={onPlaySamples}
            >Play 8s of each</button
          >
        </div>
      </div>
    {/if}

    {#if error}
      <p class="so__error" role="alert">{error}</p>
    {/if}

    <div class="row row--commit">
      {#if scope === "speaker"}
        <button
          type="button"
          class="btn btn--primary"
          disabled={applyDisabled}
          onclick={() => onApplyName(nameDraft.trim())}
        >
          {busy ? "Applying…" : "Name & apply"}
        </button>
      {:else}
        <button
          type="button"
          class="btn btn--primary"
          disabled={busy || !moveChoice}
          onclick={() => onMoveGroupTo(Number(moveChoice))}
        >
          {busy ? "Moving…" : "Move this line"}
        </button>
      {/if}
      {#if unnamedRemaining > 0}
        <span class="so__mono"
          >{unnamedRemaining} more unnamed in this segment</span
        >
      {/if}
    </div>

    {#if rejectSubject}
      <div class="row">
        <button type="button" class="btn btn--danger" disabled={busy} onclick={onNotThisPerson}>
          {suggestionPending ? `Not ${rejectSubject}` : `Unlink ${rejectSubject}`}
        </button>
      </div>
      <!-- The shipped copy, NOT the mockup's: rejections are per-cluster booleans
           on this branch, so they are emphatically not "everywhere, from now on". -->
      <p class="so__note">
        <strong>Not {rejectSubject}</strong> applies to <em>this speaker only</em> — it stops
        {rejectSubject} being suggested for this one voice in this segment, and changes nothing
        about any other recording. The <code>✕</code> in the transcript only hides the suggestion.
      </p>
    {/if}
  </div>
</div>

<style>
  .so {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 8;
    width: min(330px, 92%);
    display: flex;
    flex-direction: column;
    background: var(--app-surface-raised);
    border-left: 1px solid var(--app-border-strong);
    box-shadow: var(--app-shadow-popover);
    animation: so-in 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  @keyframes so-in {
    from {
      transform: translateX(100%);
    }
    to {
      transform: translateX(0);
    }
  }

  .so[aria-busy="true"] {
    cursor: progress;
  }

  .so__head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 11px 14px;
    border-bottom: 1px solid var(--app-border);
  }

  .so__t {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }

  .so__grow {
    flex: 1 1 auto;
  }

  .so__close {
    padding: 2px 6px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    cursor: pointer;
  }

  .so__close:hover,
  .so__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
    outline: none;
  }

  .so__body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 14px;
  }

  .so__body form {
    display: block;
  }

  .lbl {
    display: block;
    margin-bottom: 5px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
  }

  .inp {
    width: 100%;
    padding: 6px 8px;
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    background: var(--app-surface);
    color: var(--app-text-strong);
    font: inherit;
    font-size: 12px;
  }

  select.inp {
    cursor: pointer;
  }

  .inp:focus-visible {
    border-color: var(--app-accent-border);
    outline: none;
    box-shadow: var(--app-ring);
  }

  .inp:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  /* AUDIT 9 — the name field can fail and now says so. */
  .inp.is-error {
    border-color: var(--app-danger-border);
    background: color-mix(in srgb, var(--app-danger) 8%, transparent);
  }

  .fieldwarn {
    display: flex;
    gap: 6px;
    margin: 5px 0 0;
    font-size: 10px;
    line-height: 1.5;
    color: var(--app-danger-text, var(--app-danger));
  }

  .fieldwarn[hidden] {
    display: none;
  }

  .hint {
    margin: 5px 0 0;
    font-size: 10px;
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .scope {
    display: flex;
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    overflow: hidden;
  }

  .scope button {
    flex: 1 1 0;
    padding: 5px 4px;
    border: 0;
    border-right: 1px solid var(--app-border-strong);
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    cursor: pointer;
  }

  .scope button:last-child {
    border-right: none;
  }

  .scope button[aria-pressed="true"] {
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }

  .card {
    padding: 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle, var(--app-surface));
  }

  .card--warn {
    border-color: var(--app-warn-border);
    background: color-mix(in srgb, var(--app-warn) 10%, transparent);
  }

  .card__h {
    margin-bottom: 4px;
    font-size: 11px;
    color: var(--app-text-strong);
  }

  .card__d {
    font-size: 10px;
    line-height: 1.55;
    color: var(--app-text-muted);
  }

  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }

  .row--card {
    margin-top: 8px;
  }

  .row--commit {
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
  }

  /* `.btn` + variants come from the global design system (+layout.svelte). */

  .so__mono {
    font-family: var(--app-font-mono);
    font-size: 10px;
    color: var(--app-text-subtle);
  }

  .so__note {
    margin: -6px 0 0;
    font-size: 10px;
    line-height: 1.55;
    color: var(--app-text-muted);
  }

  .so__note strong {
    color: var(--app-danger-text, var(--app-danger));
  }

  .so__note code {
    font-family: var(--app-font-mono);
  }

  .so__error {
    margin: 0;
    font-family: var(--app-font-mono);
    font-size: 10px;
    line-height: 1.4;
    color: var(--app-danger-text, var(--app-danger));
    word-break: break-word;
  }

  @media (prefers-reduced-motion: reduce) {
    .so {
      animation: none;
    }
  }
</style>
