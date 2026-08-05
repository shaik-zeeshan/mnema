<!-- The Quick Access inspector in ASK mode: the machine record of the turn the
     answer column is showing — which tools ran, what they read, what it cost.
     Same panel, different subject (direction 02's one-inspector rule).

     Every row here is a fact the backend already streamed. There is no provider
     latency, no dollar figure and no token price: G8 — a number ships only where
     the value is real on this machine. -->
<script lang="ts">
  let {
    turnIndex,
    turnCount,
    phase,
    stoppedEarly = false,
    toolLabels = [],
    frameSourceCount = 0,
    audioSourceCount = 0,
    contextTokens = null,
  }: {
    turnIndex: number;
    turnCount: number;
    phase: "thinking" | "streaming" | "done" | "error";
    stoppedEarly?: boolean;
    toolLabels?: string[];
    frameSourceCount?: number;
    audioSourceCount?: number;
    contextTokens?: number | null;
  } = $props();

  const phaseLabel = $derived(
    stoppedEarly
      ? "Stopped early"
      : phase === "thinking"
        ? "Working"
        : phase === "streaming"
          ? "Writing"
          : phase === "error"
            ? "Failed"
            : "Answered",
  );
</script>

<aside class="ss-insp ss-insp--wide turn-inspector">
  <div class="ss-insp__h">
    <span class="turn-inspector__title">Turn</span>
    {#if turnCount > 0}
      <span class="ss-tstrip__spacer"></span>
      <span class="t-meta is-mono is-num">{turnIndex + 1} of {turnCount}</span>
    {/if}
  </div>

  <div class="ss-insp__b">
    {#if turnCount === 0}
      <p class="ss-insp__empty">
        Ask a question and this panel carries the record of the answer — the tools
        that ran, the captures they read, and the context it took.
      </p>
    {:else}
      <div class="ss-insp__sec">This turn</div>
      <div class="ss-kv">
        <span class="ss-kv__k">State</span>
        <span class="ss-kv__v">{phaseLabel}</span>
      </div>
      {#if contextTokens !== null}
        <div class="ss-kv">
          <span class="ss-kv__k">Context</span>
          <span class="ss-kv__v is-mono">{contextTokens.toLocaleString()} tokens</span>
        </div>
      {/if}

      <div class="ss-insp__sec">What it read</div>
      <div class="ss-kv">
        <span class="ss-kv__k">Screen</span>
        <span class="ss-kv__v is-mono"
          >{frameSourceCount}
          {frameSourceCount === 1 ? "moment" : "moments"}</span
        >
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Audio</span>
        <span class="ss-kv__v is-mono"
          >{audioSourceCount}
          {audioSourceCount === 1 ? "moment" : "moments"}</span
        >
      </div>

      <div class="ss-insp__sec">Tools</div>
      {#if toolLabels.length === 0}
        <p class="ss-insp__empty">No tools have run for this turn.</p>
      {:else}
        {#each toolLabels as label, i (i)}
          <div class="ss-kv">
            <span class="ss-kv__k is-num">{(i + 1).toString().padStart(2, "0")}</span>
            <span class="ss-kv__v turn-inspector__tool">{label}</span>
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</aside>

<style>
  .turn-inspector__title {
    color: var(--app-text-strong);
  }

  /* Tool labels are backend-formatted strings of arbitrary length; they wrap
     rather than truncate, since knowing exactly what ran is the point. */
  .turn-inspector__tool {
    color: var(--app-text);
  }
</style>
