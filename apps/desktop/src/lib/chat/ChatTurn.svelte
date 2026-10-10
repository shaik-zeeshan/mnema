<script lang="ts">
  // One chat turn (chat.html › thread): the question bubble, a steps disclosure
  // (live label while working, "N steps · 3.4s" when done), the reasoning
  // disclosure, the answer blocks, sources, and an actions row with the model
  // and this answer's token bar. Pure render of the backend-owned TurnView.
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import { appIconFallback } from "$lib/app-privacy-exclusion";
  import AnswerProse from "$lib/AnswerProse.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import Timeline from "$lib/insights/charts/Timeline.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import ChatErrorTurn from "$lib/insights/ChatErrorTurn.svelte";
  import type { ToolActivityEntry } from "$lib/insights/conversation";
  import type { AiRuntimeSettings } from "$lib/types/recording";
  import ChatSources from "./ChatSources.svelte";
  import { formatTokenCount, stepsLine } from "./chat-format";
  import { answerPlainText, type ChatTurn } from "./turn-model";
  import IconCheck from "~icons/lucide/check";
  import IconChev from "~icons/lucide/chevron-down";
  import IconLoader from "~icons/lucide/loader-circle";
  import IconSparkle from "~icons/lucide/sparkle";
  import IconCopy from "~icons/lucide/copy";
  import IconRegen from "~icons/lucide/rotate-cw";

  interface Props {
    turn: ChatTurn;
    /** The last turn of the thread (only it gets Retry / Regenerate). */
    trailing: boolean;
    /** The model answering this chat (not recorded per turn) + where it runs. */
    model: string | null;
    where: "cloud" | "local" | null;
    /** The model's context window when known (for the token bar). */
    contextWindow: number | null;
    settings: AiRuntimeSettings | null;
    /** Can't re-run now (streaming / Ask AI off): Retry disables, Regenerate hides. */
    retryDisabled: boolean;
    /** Retry a failed turn, or Regenerate a finished one — the same in-place path. */
    onRetry: () => void;
  }
  let { turn, trailing, model, where, contextWindow, settings, retryDisabled, onRetry }: Props = $props();

  const live = $derived(turn.phase === "thinking" || turn.phase === "streaming");
  const liveLabel = $derived(
    `${turn.liveActivity?.label ?? (turn.phase === "streaming" ? "Writing" : "Thinking")}…`,
  );
  const steps = $derived.by(() => {
    const line = stepsLine(turn.toolActivities.length, turn.elapsedMs);
    return line && turn.scopeLabel ? `${line} · searched ${turn.scopeLabel}` : line;
  });
  const reasoningLive = $derived(live && (turn.reasoning ?? "").trim().length > 0 && turn.blocks.length === 0);
  const copyText = $derived(answerPlainText(turn));

  // UI-only disclosure state (settled turns start collapsed).
  let stepsOpen = $state(false);
  let reasonOpen = $state(false);
  const askedAt = $derived(
    turn.atMs === null ? "" : new Date(turn.atMs).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }),
  );

  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => () => {
    if (copyTimer !== null) clearTimeout(copyTimer);
  });
  async function copy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(copyText);
      copied = true;
      if (copyTimer !== null) clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copied = false), 1600);
    } catch {
      // Best-effort.
    }
  }
</script>

{#snippet appChip(entry: ToolActivityEntry)}
  <span>
    · {#if entry.appIconPath}<img class="ch-appicon" src={convertFileSrc(entry.appIconPath)} alt="" />{:else}<i class="ch-appicon">{appIconFallback(entry.app ?? "", entry.app ?? "")}</i>{/if}{entry.app}
  </span>
{/snippet}

<article class="ch-turn">
  <div class="ch-q">{turn.question}</div>
  {#if askedAt}<div class="ch-q-meta">{askedAt}</div>{/if}

  {#if turn.phase === "error"}
    {#if turn.toolActivities.length > 0}
      <details class="ch-steps">
        <summary><IconCheck width="12" height="12" aria-hidden="true" />{steps}<IconChev width="11" height="11" aria-hidden="true" /></summary>
        <ul>
          {#each turn.toolActivities as a, i (i)}<li><IconCheck width="13" height="13" aria-hidden="true" />{a.label}{#if a.app}{@render appChip(a)}{/if}</li>{/each}
        </ul>
      </details>
    {/if}
    <ChatErrorTurn
      message={turn.errorMessage}
      kind={turn.errorKind}
      {settings}
      {trailing}
      {retryDisabled}
      {onRetry}
    />
  {:else}
    {#if live || steps}
      <details class="ch-steps" open={live || stepsOpen} ontoggle={(e) => !live && (stepsOpen = e.currentTarget.open)}>
        <summary>
          {#if live}
            <span class="ch-live">{liveLabel}</span>
          {:else}
            <IconCheck width="12" height="12" aria-hidden="true" />{steps}<IconChev width="11" height="11" aria-hidden="true" />
          {/if}
        </summary>
        {#if turn.toolActivities.length > 0}
          <ul>
            {#each turn.toolActivities as a, i (i)}
              {@const running = live && turn.liveActivity !== null && i === turn.toolActivities.length - 1}
              <li data-run={running || undefined}>
                {#if running}<IconLoader width="13" height="13" aria-hidden="true" />{:else}<IconCheck width="13" height="13" aria-hidden="true" />{/if}
                {a.label}{#if a.app}{@render appChip(a)}{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </details>
    {/if}

    <div class="ch-answer">
      {#if (turn.reasoning ?? "").trim().length > 0}
        <details class="ch-reason" open={reasoningLive || reasonOpen} ontoggle={(e) => !reasoningLive && (reasonOpen = e.currentTarget.open)}>
          <summary><IconSparkle width="12" height="12" aria-hidden="true" />Thought process<IconChev width="11" height="11" aria-hidden="true" /></summary>
          <p>{turn.reasoning}</p>
        </details>
      {/if}
      {#each turn.blocks as block, bi (bi)}
        {#if block.kind === "prose"}
          <AnswerProse source={block.markdown} isStreaming={turn.phase !== "done" && bi === turn.blocks.length - 1} />
        {:else if block.kind === "bars"}
          <div class="mx-panel ch-block">
            {#if block.title}<div class="ch-block__h"><h5>{block.title}</h5></div>{/if}
            <MiniBars items={block.items} />
          </div>
        {:else if block.kind === "dossier"}
          <div class="mx-panel ch-block">
            <div class="ch-block__h"><h5>Relevant to this</h5><span class="mx-label">from User Context</span></div>
            <div class="ch-dos">
              {#each block.items as item, di (di)}
                <div>
                  {#if item.subject}<span class="mx-chip">{item.subject}</span>{:else}<span></span>{/if}
                  <p>{item.statement}</p>
                  <ConfidenceBar confidence={item.confidence} />
                </div>
              {/each}
            </div>
          </div>
        {:else if block.kind === "timeline"}
          <div class="mx-panel ch-block"><Timeline title={block.title} intervals={block.items} /></div>
        {/if}
      {/each}
    </div>

    {#if turn.phase === "done"}
      {#if turn.sources.length > 0}<ChatSources sources={turn.sources} />{/if}
      <div class="ch-acts">
        {#if copyText}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Copy answer" use:tip={copied ? "Copied" : "Copy"} onclick={() => void copy()}>
            {#if copied}<IconCheck width="15" height="15" aria-hidden="true" />{:else}<IconCopy width="15" height="15" aria-hidden="true" />{/if}
          </button>
          {#if copied}<span class="mx-inline" data-tone="ok">Copied</span>{/if}
        {/if}
        {#if trailing && !retryDisabled}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Regenerate" use:tip={"Regenerate"} onclick={onRetry}>
            <IconRegen width="15" height="15" aria-hidden="true" />
          </button>
        {/if}
        {#if model}<span class="mx-label"><span class="mono">{model}</span>{where ? ` · ${where}` : ""}</span>{/if}
        {#if turn.contextTokens !== null}
          <span
            class="mx-label num ch-ctxread"
            use:tip={contextWindow ? "Context used by this answer" : "Tokens this answer used (window not reported)"}
          >
            {#if contextWindow}
              <i style="--w:{Math.min(100, (turn.contextTokens / contextWindow) * 100)}%"></i>{formatTokenCount(turn.contextTokens)} / {formatTokenCount(contextWindow)}
            {:else}
              {formatTokenCount(turn.contextTokens)} tokens
            {/if}
          </span>
        {/if}
      </div>
    {/if}
  {/if}
</article>

<style>
  :global(.ch-turn) + .ch-turn { margin-top: var(--s-6); padding-top: var(--s-6); border-top: 1px dashed var(--mx-hairline); }
  .ch-q {
    margin: 0 0 6px auto;
    max-width: 78%;
    width: fit-content;
    padding: 10px 14px;
    border-radius: var(--r-lg) var(--r-lg) var(--r-xs) var(--r-lg);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    color: var(--app-text-strong);
    font: 400 var(--text-md)/1.5 var(--font-sans);
    white-space: pre-wrap;
  }
  .ch-q-meta { margin-bottom: var(--s-4); text-align: right; font: 400 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); }
  .ch-turn > .ch-q + :not(.ch-q-meta) { margin-top: var(--s-4); }
  .ch-steps, .ch-reason { margin: 0 0 var(--s-4); }
  .ch-reason { margin-bottom: var(--s-3); }
  .ch-steps > summary, .ch-reason > summary {
    list-style: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    padding: 4px 0;
    font: 500 var(--text-base)/1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ch-steps > summary::-webkit-details-marker, .ch-reason > summary::-webkit-details-marker { display: none; }
  .ch-steps > summary:hover, .ch-reason > summary:hover { color: var(--app-text-strong); }
  .ch-steps > summary > :global(svg:last-child), .ch-reason > summary > :global(svg:last-child) { transition: transform var(--t-med) var(--ease-expo); }
  .ch-steps[open] > summary > :global(svg:last-child), .ch-reason[open] > summary > :global(svg:last-child) { transform: rotate(180deg); }
  .ch-steps ul { list-style: none; margin: 6px 0 0; padding: 2px 0 2px 12px; display: grid; gap: 5px; border-left: 1px solid var(--app-border); }
  .ch-steps li { display: flex; align-items: center; gap: var(--s-2); font: 500 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-muted); animation: mx-rise var(--t-med) var(--ease-expo) both; }
  .ch-steps li > :global(svg) { flex: none; color: var(--app-text-subtle); }
  .ch-steps li > span { display: inline-flex; align-items: center; gap: 4px; color: var(--app-text-subtle); }
  .ch-steps li[data-run] { color: var(--app-text-strong); }
  .ch-steps li[data-run] > :global(svg) { color: var(--app-accent); }
  .ch-appicon { width: 13px; height: 13px; border-radius: 3px; font: 600 8px/13px var(--font-sans); font-style: normal; text-align: center; background: var(--app-surface-hover); }
  .ch-live { display: inline-flex; align-items: center; gap: 8px; font: 500 var(--text-base)/1 var(--font-sans); color: var(--app-text-muted); }
  .ch-live::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--app-accent); box-shadow: 0 0 0 3px var(--app-accent-glow); }
  .ch-reason p {
    margin: 6px 0 0;
    padding-left: 12px;
    border-left: 1px solid var(--app-border);
    max-width: 68ch;
    font: 400 var(--text-base)/1.55 var(--font-sans);
    color: var(--app-text-subtle);
    white-space: pre-wrap;
  }
  .ch-answer :global(.answer-prose) { max-width: 68ch; }
  .ch-block { padding: var(--s-4) var(--s-5); margin: 0 0 var(--s-4); }
  .ch-block__h { display: flex; align-items: baseline; gap: var(--s-3); margin-bottom: var(--s-3); }
  .ch-block__h h5 { margin: 0; font: 600 var(--text-md)/1.3 var(--font-sans); color: var(--app-text-strong); }
  .ch-block__h .mx-label { margin-left: auto; font-size: var(--text-xs); }
  .ch-dos { display: grid; }
  .ch-dos > div { display: grid; grid-template-columns: 96px minmax(0, 1fr) 70px; gap: var(--s-3); align-items: center; padding: 9px 0; border-top: 1px solid var(--mx-hairline); }
  .ch-dos > div:first-child { border-top: 0; padding-top: 0; }
  .ch-dos .mx-chip { justify-self: start; }
  .ch-dos p { margin: 0; font-size: var(--text-base); line-height: 1.5; color: var(--app-text-strong); }
  .ch-acts { display: flex; align-items: center; gap: 2px; margin-top: var(--s-3); }
  .ch-acts .mx-label { margin-left: var(--s-2); font-size: var(--text-xs); }
  .ch-acts .mx-inline { margin: 0 6px; font-size: var(--text-sm); }
  .ch-ctxread { margin-left: auto !important; display: inline-flex; align-items: center; gap: 6px; }
  .ch-ctxread i { display: inline-block; width: 40px; height: 4px; border-radius: 2px; background: color-mix(in srgb, var(--app-fg) 8%, transparent); position: relative; overflow: hidden; }
  .ch-ctxread i::after { content: ""; position: absolute; inset: 0 auto 0 0; width: var(--w); background: var(--chart-grey-4); }
</style>
