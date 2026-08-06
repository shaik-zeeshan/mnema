<script lang="ts">
  // The dismissal archive + the one destructive action on this surface.
  //
  // Copy is checked against the code, not against the mockup:
  //   • Dismiss really DELETES the conclusion row and writes a veto
  //     (`dismiss_conclusion`); the belief returns only on substantially fresher
  //     evidence, never from the evidence just rejected.
  //   • Restore lifts the veto (`undismiss`) — it does not resurrect the row.
  //   • Retention never touches user_context_* (ADR 0029).
  //   • Delete recent capture deletes the raw window AND the activities whose
  //     evidence pointed at it, then re-applies the formation bar, dropping the
  //     conclusions left under it (`cascade_derived_for_deleted_subjects_in`).
  import type { DismissedView } from "$lib/types/recording";
  import { relativeAge } from "./data";

  interface Props {
    dismissed: DismissedView[];
    selected: number;
    restoringKey: string | null;
    nowMs: number;
    wiping: boolean;
    keyOf: (d: DismissedView) => string;
    onSelect: (index: number) => void;
    onRestore: (d: DismissedView) => void;
    onHide: () => void;
    onWipe: () => void;
  }

  let {
    dismissed,
    selected,
    restoringKey,
    nowMs,
    wiping,
    keyOf,
    onSelect,
    onRestore,
    onHide,
    onWipe,
  }: Props = $props();
</script>

<div class="ctx__main">
  <div class="stlist">
    <div class="strow strow--head">
      <span class="strow__t">
        <span class="strow__x">Dismissed · {dismissed.length}</span>
        <span class="t-meta">
          Beliefs you removed from your dossier. Restoring lets one form again — only if your
          activity still supports it.
        </span>
      </span>
      <button type="button" class="btn btn--sm" onclick={onHide}>
        Hide <span class="kbd">⌘D</span>
      </button>
    </div>
  </div>

  {#if dismissed.length > 0}
    <div class="stlist" role="listbox" aria-label="Dismissed beliefs" tabindex="-1">
      {#each dismissed as d, i (keyOf(d))}
        <div
          class="arow"
          class:is-key={i === selected}
          role="option"
          aria-selected={i === selected}
          tabindex="-1"
          onclick={() => onSelect(i)}
          onkeydown={() => {}}
        >
          <span class="arow__s">{d.subject}</span>
          <span class="arow__x">{d.statement}</span>
          <span class="t-meta is-mono is-num arow__t">{relativeAge(d.dismissedAtMs, nowMs)}</span>
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            disabled={restoringKey === keyOf(d)}
            onclick={(e) => {
              e.stopPropagation();
              onRestore(d);
            }}
          >
            {restoringKey === keyOf(d) ? "Restoring…" : "Restore"}
            {#if i === selected}<span class="kbd">⏎</span>{/if}
          </button>
        </div>
      {/each}
    </div>
  {:else}
    <div class="stlist">
      <div class="strow">
        <span class="strow__t">
          <span class="t-meta">
            Nothing dismissed. Dismissing a conclusion moves it here and records a veto against it.
          </span>
        </span>
      </div>
    </div>
  {/if}

  <div class="callout callout--warn">
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M8 2.4 14.6 13.4H1.4z" /><path d="M8 6.4v3.2M8 11.6v.1" />
    </svg>
    <span class="t-meta">
      Dismissing <b class="med">deletes</b> the belief and records a veto against it. Restoring lifts
      the veto — it does not bring the old belief back; the same belief can only form again from
      substantially fresher evidence.
    </span>
  </div>
</div>

<div class="ctx__rail">
  <div class="rcard">
    <div class="rcard__h"><span class="t-label">What deletion reaches</span></div>
    <div class="reach">
      <div>
        <span class="t-ui strong">Retention cleanup</span>
        <div class="t-meta">
          Ages out frames and audio on your schedule. <b class="med">Never</b> touches activities,
          conclusions or standing context.
        </div>
      </div>
      <div>
        <span class="t-ui strong">Delete recent capture</span>
        <div class="t-meta">
          Deletes the raw window <b class="med">and</b> the activities derived from it, then drops
          the conclusions left under the evidence bar.
        </div>
      </div>
      <div>
        <span class="t-ui strong">Wipe user context</span>
        <div class="t-meta">
          Clears the understanding itself — including your standing statements and ask history — and
          turns AI features off. Recordings kept.
        </div>
      </div>
    </div>
    <button
      type="button"
      class="btn btn--danger btn--sm reach__wipe"
      disabled={wiping}
      onclick={onWipe}
    >
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M2.8 4.2h10.4M6.4 4.2V2.8h3.2v1.4M4.2 4.2l.7 8.4a1.2 1.2 0 0 0 1.2 1.1h3.8a1.2 1.2 0 0 0 1.2-1.1l.7-8.4" />
      </svg>
      {wiping ? "Wiping…" : "Wipe user context"}
    </button>
  </div>

  <div class="callout">
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="8" cy="8" r="6" /><path d="M8 4.6V8l2.4 1.6" />
    </svg>
    <span class="t-meta">
      The understanding outlives the recordings. Once the footage behind a conclusion ages out, the
      summary stays and its receipt stops resolving to a frame.
    </span>
  </div>
</div>

<style>
  .arow__t {
    flex: 0 0 auto;
    color: var(--app-text-subtle);
  }
  .reach {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
  }
  .reach__wipe {
    margin-top: var(--s-4);
    align-self: flex-start;
  }
  .reach__wipe svg {
    width: 12px;
    height: 12px;
  }
</style>
