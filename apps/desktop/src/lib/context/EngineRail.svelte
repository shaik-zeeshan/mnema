<script lang="ts">
  // The engine's side of Context — counted, never narrated.
  //
  // Everything on this rail is a field that exists on `get_user_context_status`
  // or on the newest `Conclusion`. The mockup's "Steering your dossier" rail is
  // deliberately NOT drawn: nothing records a link between a standing statement
  // and a conclusion, so staging the top-confidence beliefs beside your writing
  // would read as causation the data cannot support (page 10, honesty notes).
  import { goto } from "$app/navigation";

  import { formatClock } from "$lib/overview/format";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";
  import { compactCount, relativeAge } from "./data";

  interface Props {
    status: UserContextStatus | null;
    conclusions: Conclusion[];
    nowMs: number;
    busy: boolean;
    onPin: (c: Conclusion) => void;
    onDismiss: (c: Conclusion) => void;
  }

  let { status, conclusions, nowMs, busy, onPin, onDismiss }: Props = $props();

  const newest = $derived.by<Conclusion | null>(() => {
    if (conclusions.length === 0) return null;
    return conclusions.reduce((a, b) => (b.formedAtMs > a.formedAtMs ? b : a));
  });

  // Evidence is a list of Activity refs. `ConclusionEvidenceRef` carries the
  // activity's title + start, and NOT which capture source it came from — so
  // these chips wear no scr/mic tag. The mockup draws one; the field does not
  // exist on this payload (page 10 "backend asks" territory, not a fiction to
  // paint in).
  const EVIDENCE_SHOWN = 3;
  const support = $derived((newest?.evidence ?? []).filter((e) => e.stance === "support"));
  const shownEvidence = $derived(support.slice(0, EVIDENCE_SHOWN));
  const moreEvidence = $derived(Math.max(0, support.length - shownEvidence.length));

  const tokens = $derived(status?.tokenUsage ?? null);
</script>

<div class="rcard">
  <div class="rcard__h">
    <span class="t-label">What it worked out</span>
    <span class="kbd" style="margin-left:auto">⌃J</span>
  </div>

  {#if status}
    <div class="kv">
      <b class="is-num">{status.activityCount}</b><span>activities summarized</span>
    </div>
    <div class="kv">
      <b class="is-num">{status.conclusionCount}</b><span
        >conclusions across {status.subjectCount}
        {status.subjectCount === 1 ? "subject" : "subjects"}</span
      >
    </div>
    <div class="kv">
      <b class="is-num">{status.dismissedCount}</b><span>dismissed by you</span>
    </div>

    <div class="rcard__mark t-meta is-mono is-num">
      {status.coveredUntilMs
        ? `covered to ${formatClock(status.coveredUntilMs)}`
        : "nothing covered yet"}{status.lastDerivedAtMs
        ? ` · last pass ${relativeAge(status.lastDerivedAtMs, nowMs)}`
        : ""}
    </div>

    {#if tokens && tokens.runCount > 0}
      <!-- Real fields, honest caveat: `estimate_tokens` is chars/4, so this is a
           text-length estimate and never a billed count. -->
      <div class="rcard__mark t-meta is-mono is-num">
        ≈ {compactCount(tokens.totalTokens)} tokens across {tokens.runCount} passes — estimated from
        text length, not a billed count
      </div>
    {/if}

    {#if !status.engineAvailable}
      <div class="rcard__mark t-meta">
        The Reasoning Engine is off{status.reason ? ` — ${status.reason}` : ""}. Nothing new is being
        worked out.
      </div>
    {/if}
  {:else}
    <p class="rcard__mark t-meta">Engine status unavailable.</p>
  {/if}

  <button type="button" class="btn btn--sm rcard__go" onclick={() => void goto("/subjects")}>
    Browse subjects <span class="kbd">⌃J</span>
  </button>
</div>

<div class="rcard">
  <div class="rcard__h">
    <span class="t-label">Newest conclusion</span>
    {#if newest}
      <span class="t-meta is-mono is-num rcard__age">{relativeAge(newest.formedAtMs, nowMs)}</span>
    {/if}
  </div>

  {#if newest}
    <span class="t-ui rcard__stmt">{newest.statement}</span>
    <span class="cbar"><i style={`width:${Math.round(newest.confidence * 100)}%`}></i></span>
    <div class="rcard__conf">
      <span class="t-ui is-mono is-num rcard__pct">{Math.round(newest.confidence * 100)}%</span>
      <span class="t-meta"
        >confidence · subject <b class="med">{newest.subject}</b>{#if newest.pinned}
          · pinned{/if}</span
      >
    </div>

    {#if shownEvidence.length > 0}
      <span class="t-label rcard__gl">Grounded in</span>
      <div class="evchips">
        {#each shownEvidence as e (e.activityId)}
          <span class="evchip">
            <span class="evchip__t">{e.activityTitle ?? `activity #${e.activityId}`}</span>
            {#if e.activityStartedAtMs}
              <span class="evchip__a is-num">{relativeAge(e.activityStartedAtMs, nowMs)}</span>
            {/if}
          </span>
        {/each}
        {#if moreEvidence > 0}
          <span class="t-meta rcard__more">+{moreEvidence} more</span>
        {/if}
      </div>
    {/if}

    <div class="rcard__acts">
      <button type="button" class="btn btn--sm" disabled={busy} onclick={() => onPin(newest)}>
        {newest.pinned ? "Unpin" : "Pin"}
      </button>
      <button type="button" class="btn btn--sm" disabled={busy} onclick={() => onDismiss(newest)}>
        Dismiss
      </button>
    </div>
  {:else}
    <p class="t-meta rcard__empty">
      Nothing concluded yet. Conclusions form once the engine has enough activity to distil.
    </p>
  {/if}
</div>

<!-- The guardrail print, verbatim from the mockup. -->
<div class="callout">
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M1.6 8S4.2 3.6 8 3.6 14.4 8 14.4 8 11.8 12.4 8 12.4 1.6 8 1.6 8z" />
    <circle cx="8" cy="8" r="1.9" />
  </svg>
  <span class="t-meta">
    Health, politics, sexuality, religion and similar are never inferred or surfaced — even if you
    mention them here. Mnema errs toward over-suppression.
  </span>
</div>

<style>
  .rcard__mark {
    color: var(--app-text-subtle);
  }
  .rcard__go {
    margin-top: var(--s-4);
    align-self: flex-start;
  }
  .rcard__age {
    margin-left: auto;
    color: var(--app-text-subtle);
  }
  .rcard__stmt {
    line-height: 1.45;
    color: var(--app-text-strong);
  }
  .rcard__conf {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
  }
  .rcard__pct {
    color: var(--app-text-strong);
    font-weight: var(--w-semi);
  }
  .rcard__gl {
    margin-top: 2px;
  }
  .rcard__more {
    color: var(--app-text-subtle);
  }
  .rcard__acts {
    display: flex;
    gap: var(--s-4);
    margin-top: var(--s-4);
  }
  .rcard__empty {
    margin: 0;
    line-height: 1.5;
  }
</style>
