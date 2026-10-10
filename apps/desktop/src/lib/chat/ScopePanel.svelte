<script lang="ts">
  // The composer's context panel (chat.html › Context panel, CH3): what the
  // model may read this turn. Slim by decision (IMPLEMENTATION.md §8 #11): a
  // time range + an About-you switch, then where it goes. No per-source toggles,
  // no pre-send meter. Web fetch and MCP connectors aren't limited by the scope,
  // so they're named, never hidden.
  import DateInput from "$lib/components/DateInput.svelte";
  import type { ChatScope, GoesTo } from "./scope";
  import IconCloud from "~icons/lucide/cloud";
  import IconLaptop from "~icons/lucide/laptop";

  interface Props {
    scope: ChatScope;
    goesTo: GoesTo;
    /** Docked at the window bottom: open the range calendar upward. */
    up: boolean;
  }
  let { scope = $bindable(), goesTo, up }: Props = $props();

  const local = $derived(goesTo.where === "local");
  const name = $derived(goesTo.name ?? "your AI model");
  // An instance label may already carry its endpoint ("Ollama · localhost:11434").
  const via = $derived(goesTo.via && !name.includes(goesTo.via) ? goesTo.via : "");
  const unscoped = $derived(
    [goesTo.webFetch && "web fetch", goesTo.connectors > 0 && `${goesTo.connectors} connector${goesTo.connectors === 1 ? "" : "s"}`]
      .filter(Boolean)
      .join(" · "),
  );
  const foot = $derived(
    [local ? "text excerpts only · no frames or audio files" : "", unscoped && `+ ${unscoped} — not limited by scope`, local ? "" : `${name}’s data policy applies`]
      .filter(Boolean)
      .join(" · "),
  );
</script>

<dl class="ch-ctx" class:ch-ctx--up={up} aria-label="Context sent to the model">
  <dt>Scope</dt>
  <dd>
    <DateInput mode="range" value="{scope.start}..{scope.end}" onchange={(r) => (scope = { ...scope, start: r.start, end: r.end })} />
  </dd>
  <dt>Include</dt>
  <dd>
    <label class="ch-about">
      <input
        type="checkbox"
        class="mx-switch"
        role="switch"
        checked={scope.aboutYou}
        onchange={(e) => (scope = { ...scope, aboutYou: e.currentTarget.checked })}
      />
      <span><b>About you</b><small>Mnema’s notes about you (inferred from your activity)</small></span>
    </label>
  </dd>
  <dt>Goes to</dt>
  <dd>
    <div class="ch-priv" class:ch-priv--local={local}>
      {#if local}
        <IconLaptop width="15" height="15" aria-hidden="true" />
        <div><b>Stays on this Mac.</b> {name}{#if via} at <span class="mono">{via}</span>{/if}.<small>{foot}</small></div>
      {:else}
        <IconCloud width="15" height="15" aria-hidden="true" />
        <div>
          <b>Sent to {name}{via ? `${via.startsWith("your") ? " with" : " at"} ${via}` : ""}.</b>
          Only text: screen words, transcript lines, app and window names{scope.aboutYou ? ", page addresses, and Mnema’s notes about you" : ", and page addresses"}. Never images or audio.<small>{foot}</small>
        </div>
      {/if}
    </div>
  </dd>
</dl>

<style>
  .ch-ctx {
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr);
    gap: 0 var(--s-4);
    margin: 0 var(--s-2);
    padding: var(--s-2) var(--s-2) var(--s-3);
    border-top: 1px solid var(--mx-hairline);
  }
  .ch-ctx > dt { padding: 16px 0 0; font: 500 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-subtle); }
  .ch-ctx > dd { margin: 0; padding: 8px 0 0; min-width: 0; }
  .ch-ctx--up :global(.mx-date__pop) { top: auto; bottom: calc(100% + 8px); transform-origin: bottom left; }
  /* One switch row (the kit has no label+helper row; settings.html keeps its own the same way). */
  .ch-about { display: flex; align-items: flex-start; gap: 10px; padding-top: 8px; cursor: pointer; }
  .ch-about .mx-switch { margin-top: 1px; }
  .ch-about b { display: block; font: 500 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-strong); }
  .ch-about small { display: block; font: 400 var(--text-sm)/1.45 var(--font-sans); color: var(--app-text-subtle); }
  .ch-priv {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 10px;
    margin-top: 2px;
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--app-info) 6%, var(--app-surface-subtle));
    border: 1px solid var(--app-info-border);
    font: 400 var(--text-sm)/1.45 var(--font-sans);
    color: var(--app-text);
  }
  .ch-priv :global(svg) { flex: none; margin-top: 2px; color: var(--app-info); }
  .ch-priv b { color: var(--app-text-strong); font-weight: 600; }
  .ch-priv small { display: block; font: 400 var(--text-sm)/1.45 var(--font-sans); color: var(--app-text-subtle); }
  .ch-priv small:empty { display: none; }
  .ch-priv--local { background: color-mix(in srgb, var(--app-accent) 5%, var(--app-surface-subtle)); border-color: var(--app-accent-border); }
  .ch-priv--local :global(svg) { color: var(--app-accent); }
</style>
