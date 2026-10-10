<script lang="ts">
  // The body of "The read" (Direction A, OV-05..09), shared by Overview and the
  // Journal day: the prose, or exactly one truthful state in its place.
  // - no read, no error, not loading → not enough activity. The backend owns
  //   the fewer-than-two guard; nothing is counted here.
  // - `DIGEST_SENSITIVE_HOLD` → held back by the sensitive filter.
  // - any other error → failed, with engine-cause copy. A failed refresh keeps
  //   the read it already had and says so underneath.
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import { turnErrorCopy } from "$lib/insights/engine-state";
  import type { UserContextDigest } from "$lib/types/recording";

  /** Mirrors `digest::DIGEST_SENSITIVE_HOLD`. */
  const SENSITIVE_HOLD = "digest_sensitive_hold";

  interface Props {
    digest: UserContextDigest | null;
    loading: boolean;
    error: string | null;
    /** "today's" / "this week's" — names the read in the copy. */
    whose: string;
    whenLabel: string;
  }

  let { digest, loading, error, whose, whenLabel }: Props = $props();

  const held = $derived(error === SENSITIVE_HOLD);
  const failText = $derived(error && !held ? turnErrorCopy(error, null, null).text : "");
</script>

{#if digest}
  <!-- Keyed on generation time: fresh prose replays the reveal, a same-range
       cache hit does not. -->
  {#key digest.generatedAtMs}
    <div class="lede-body">
      {#if digest.headline}
        <h2 class="lede-headline">{digest.headline}</h2>
      {/if}
      <p class="lede-text">{digest.narrative}</p>
    </div>
  {/key}
  {#if error && !loading}
    <p class="lede-sub">
      <span class="wdot" aria-hidden="true"></span>
      Couldn't refresh. {held ? "The new read touched a private category." : failText}
      Showing the read from {whenLabel}.
    </p>
  {/if}
{:else if loading}
  <div class="sk-row"><Skeleton variant="text" width="92%" height="12px" /></div>
  <div class="sk-row"><Skeleton variant="text" width="64%" height="12px" /></div>
{:else if held}
  <p class="lede-quiet">{whose} read was held back.</p>
  <p class="lede-sub">
    It touched a private category, so Mnema dropped it rather than show it. Your
    activities are untouched. Try re-read, or step to another range.
  </p>
{:else if error}
  <p class="lede-error" role="alert">Couldn't write {whose.toLowerCase()} read. {failText}</p>
{:else}
  <p class="lede-quiet">Not enough yet for a read.</p>
  <p class="lede-sub">
    A read needs at least two activities. It's written automatically once there
    are enough, so there's nothing to press.
  </p>
{/if}

<style>
  .lede-body {
    animation: lede-reveal 0.25s ease;
  }
  @keyframes lede-reveal {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lede-body {
      animation: none;
    }
  }
  .lede-headline {
    margin: 0 0 10px;
    font-size: 24px;
    line-height: 1.22;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--app-text-strong);
  }
  .lede-text,
  .lede-quiet,
  .lede-error {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-text);
  }
  .lede-error {
    color: var(--app-danger, var(--app-text-subtle));
  }
  .lede-sub {
    margin: 4px 0 0;
    max-width: 620px;
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--app-text-muted);
  }
  .wdot {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 8px;
    vertical-align: 1px;
    border-radius: 50%;
    background: var(--app-warn);
  }
  .sk-row {
    padding: 9px 0;
  }
  .sk-row + .sk-row {
    border-top: 1px dashed var(--app-border);
  }
</style>
