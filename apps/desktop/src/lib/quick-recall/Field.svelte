<!-- Quick Recall field (recall.html `.rc-bar`): the mode chip (search ⇄ Ask,
     "auto" while a detected question is answered on its own), the pills, the
     text — recognised phrases tinted in place by a mirror under a transparent
     input — and the right-hand status. Fills BarShell's `bar` slot; the
     module-level `fieldFoot` snippet fills its `foot` slot with the real
     operator query and the keys that work right now (Ask passes its own keys). Keys: search-keys.ts. -->
<script module lang="ts">
  import IconCode from "~icons/lucide/code";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { mirrorSegments } from "./natural-tokens";
  import { pillLabel, pillOperators } from "./filter-chips";
  import { quickLook, lookKeys } from "./detailStore.svelte";

  // The token the backend rejected, while the shown results are this query's.
  function badToken(): string | null {
    const err = search.firstParseError;
    return err && search.resultsQuery === search.operatorQuery ? err.token : null;
  }

  // Keys for search mode, most specific surface first.
  function searchKeys(): [string, string][] {
    const f = search.filters;
    if (f.popoverOpen) return [["↑↓", "pick"], ["↵", "apply"], ["esc", "close"]];
    if (f.pillIndex !== null) return [["↵", "change"], ["⌫", "remove"], ["esc", "back"]];
    const look = lookKeys();
    if (look) return look;
    const st = search.states;
    if (st.showRecent) {
      return [["↑↓", "pick"], ["↵", "use"], ...(st.recentIndex < st.recent.length ? [["⌫", "remove"] as [string, string]] : [])];
    }
    if (st.emptyState === "none") return [["↵", "apply"], ["esc", "clear"]];
    if (st.emptyState === "error") return [["↵", "retry"], ["esc", "clear"]];
    const ghost: [string, string][] = f.hasGhost ? [["→", "complete"]] : [];
    if (search.resultCount > 0 && !search.loading) {
      return [
        ...ghost,
        ["↵", "open"],
        [quickLook.nav ? "space" : "↓ space", "look"],
        ...(search.selectedResultIsOpenable ? [["⌘↵", "page"] as [string, string]] : []),
        ["⌘C", "copy"],
      ];
    }
    return [...ghost, ["esc", "clear"]];
  }

  export { fieldFoot };
</script>

<script lang="ts">
  import IconSearch from "~icons/lucide/search";
  import IconSparkle from "~icons/lucide/sparkle";
  import IconCalendar from "~icons/lucide/calendar";
  import { handleSearchKeydown } from "./search-keys";
  import { appIcons } from "./app-icons.svelte";

  let {
    mode = "search",
    auto = false,
    askAvailable,
    onToggleMode,
  }: {
    mode?: "search" | "ask";
    // A detected question is being answered without Tab (QR5).
    auto?: boolean;
    askAvailable: boolean;
    onToggleMode: () => void;
  } = $props();

  const filters = search.filters;
  let scrollLeft = $state(0);

  const segments = $derived(
    mirrorSegments(search.query, search.naturalParse.spans, badToken()),
  );

  function onInput(): void {
    search.updateCaretAtEnd();
    filters.selectPill(null);
    search.scheduleCommit();
  }

  function onPillClick(i: number): void {
    if (filters.popoverOpen && filters.pillIndex === i) filters.selectPill(null);
    else filters.openPopover(i);
    search.inputEl?.focus();
  }
</script>

{#snippet fieldFoot(keys?: [string, string][])}
  {@const bad = badToken()}
  <span class="rc-foot__q">
    {#if search.operatorQuery}
      <IconCode width="13" height="13" aria-hidden="true" />
      <code
        >{#if search.naturalParse.residual}<b
            >{#each mirrorSegments(search.naturalParse.residual, [], bad) as seg, i (i)}{#if seg.kind === "bad"}<span
                  class="rc-bad">{seg.text}</span
                >{:else}{seg.text}{/if}{/each}</b
          >{/if}{#each search.effectivePills as pill (pill.kind)}{" "}{pillOperators(pill)}{/each}</code
      >
    {:else}
      Type naturally — “in Arc”, “yesterday afternoon”, “on a call” — or a question.
    {/if}
  </span>
  <span class="rc-foot__k">
    {#each keys ?? searchKeys() as [key, label] (key)}
      <span><kbd>{key}</kbd>{label}</span>
    {/each}
  </span>
{/snippet}

<button
  type="button"
  class="rc-mode"
  data-ask={mode === "ask" || undefined}
  tabindex="-1"
  aria-label={mode === "ask" ? "Asking — Tab to search" : "Searching — Tab to ask"}
  aria-keyshortcuts="Tab"
  onclick={() => {
    onToggleMode();
    search.inputEl?.focus();
  }}
>
  <span class="rc-mode__g" aria-hidden="true"
    ><IconSearch width="17" height="17" /><IconSparkle width="16" height="16" /></span
  >{#if mode === "ask"}<b>Ask</b>{#if auto}<small>auto</small>{/if}{/if}
</button>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="rc-field" onclick={() => search.inputEl?.focus()}>
  {#each search.pills as pill, i (pill.kind)}
    <button
      type="button"
      class="mx-btn mx-btn--sm rc-tok"
      tabindex="-1"
      aria-pressed={filters.pillIndex === i}
      aria-label="{pillLabel(pill)} — {pillOperators(pill)}. Click to change"
      onclick={(event) => {
        event.stopPropagation();
        onPillClick(i);
      }}
    >
      {#if pill.kind === "app"}
        {@const icon = appIcons.src(pill.app)}
        {#if icon}<img class="rc-app" src={icon} alt="" />{:else}<i class="rc-app"
            >{pill.app.charAt(0).toUpperCase()}</i
          >{/if}
      {:else if pill.kind === "date"}
        <IconCalendar width="13" height="13" aria-hidden="true" />
      {:else}
        <i class="rc-dot" data-src={pill.source}></i>
      {/if}
      <span>{pillLabel(pill)}</span>
    </button>
  {/each}
  <span class="rc-inwrap">
    <span class="rc-mirror" aria-hidden="true"
      ><span style:transform="translateX({-scrollLeft}px)"
        >{#each segments as seg, i (i)}{#if seg.kind === "plain"}{seg.text}{:else}<span
              class={seg.kind === "live" ? "rc-live" : "rc-bad"}>{seg.text}</span
            >{/if}{/each}{#if filters.hasGhost}<span class="rc-ghost"
            >{filters.ghostCompletion}</span
          >{/if}</span
      ></span
    >
    <input
      bind:this={search.inputEl}
      bind:value={search.query}
      placeholder="Recall anything you saw or heard…"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      role="combobox"
      aria-label="Recall anything — Tab to ask"
      aria-controls="quick-recall-results-list"
      aria-expanded={search.resultCount > 0}
      aria-activedescendant={search.activeOptionId}
      aria-keyshortcuts="ArrowUp ArrowDown Enter Escape Tab Meta+Enter"
      onkeydown={(event) => handleSearchKeydown(search, event, onToggleMode)}
      oninput={onInput}
      onkeyup={() => search.updateCaretAtEnd()}
      onclick={() => search.updateCaretAtEnd()}
      onselect={() => search.updateCaretAtEnd()}
      onscroll={(event) => (scrollLeft = event.currentTarget.scrollLeft)}
      onfocus={() => {
        search.updateCaretAtEnd();
        void search.ensureSearchableAppsLoaded();
      }}
    />
  </span>
</div>

<span class="rc-right">
  {#if quickLook.result && quickLook.index >= 0}
    <span><b>{quickLook.index + 1}</b> of {quickLook.cites?.length ?? search.resultCount}</span><span
      ><kbd>esc</kbd>back</span
    >
  {:else if mode === "ask"}
    <span><kbd>⇥</kbd>search</span>
  {:else if search.loading}
    <span><span class="mx-spin mx-spin--sm"></span>searching…</span>
  {:else if search.states.showRecent}
    <span><kbd>esc</kbd>close</span>
  {:else if !search.operatorQuery}
    <span><kbd>↓</kbd>recent</span>
  {:else}
    {#if search.totalResultCount > 0}
      <span
        ><b>{search.totalResultCount}</b> moment{search.totalResultCount === 1 ? "" : "s"}</span
      >
    {/if}
    {#if askAvailable && search.operatorQuery}
      <span><kbd>⇥</kbd>ask</span>
    {/if}
  {/if}
</span>

<style>
  .rc-mode {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    min-width: 30px;
    padding: 0 7px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    color: var(--app-text-subtle);
    cursor: pointer;
    font: 600 var(--text-md) / 1 var(--font-sans);
    transition:
      background-color var(--t-med) var(--ease-quart),
      color var(--t-med) var(--ease-quart),
      box-shadow var(--t-med) var(--ease-quart);
  }
  .rc-mode:hover {
    background: var(--mx-wash-strong);
    color: var(--app-text);
  }
  .rc-mode__g {
    display: grid;
    width: 17px;
    height: 17px;
  }
  .rc-mode__g :global(svg) {
    grid-area: 1 / 1;
    transition:
      transform var(--t-slow) var(--ease-expo),
      opacity var(--t-med) var(--ease-quart);
  }
  .rc-mode__g :global(svg + svg) {
    opacity: 0;
    transform: rotate(-90deg) scale(0.5);
  }
  .rc-mode small {
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: color-mix(in srgb, var(--app-accent) 70%, var(--app-text-subtle));
  }
  .rc-mode[data-ask] {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: inset 0 0 0 1px var(--app-accent-border);
  }
  .rc-mode[data-ask] .rc-mode__g :global(svg:first-child) {
    opacity: 0;
    transform: rotate(90deg) scale(0.5);
  }
  .rc-mode[data-ask] .rc-mode__g :global(svg + svg) {
    opacity: 1;
    transform: none;
  }

  .rc-field {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
  }
  .rc-tok {
    flex: none;
    max-width: 220px;
  }
  .rc-tok > span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rc-tok :global(svg) {
    color: var(--app-text-subtle);
  }
  .rc-tok[aria-pressed="true"] {
    --_bd: var(--app-border-hover);
  }
  .rc-app {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 4px;
    font: 700 8.5px / 1 var(--font-sans);
    font-style: normal;
    background: var(--mx-wash-strong);
    color: var(--app-text-strong);
    object-fit: contain;
  }
  img.rc-app {
    background: none;
  }
  .rc-dot {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    background: var(--app-source-screen);
  }
  .rc-dot[data-src="mic"] {
    background: var(--app-source-mic);
  }
  .rc-dot[data-src="system"] {
    background: var(--app-source-sysaudio);
  }

  .rc-inwrap {
    position: relative;
    flex: 1;
    min-width: 120px;
    height: 34px;
    display: flex;
    overflow: hidden;
  }
  .rc-inwrap input,
  .rc-mirror {
    font: 400 19px / 34px var(--font-sans);
    letter-spacing: -0.012em;
  }
  .rc-inwrap input {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    outline: 0;
    background: none;
    color: transparent;
    caret-color: var(--app-text-strong);
  }
  .rc-inwrap input::placeholder {
    color: var(--app-text-subtle);
  }
  .rc-inwrap input::selection {
    color: transparent;
    background: var(--mx-selected);
  }
  .rc-mirror {
    position: absolute;
    inset: 0;
    pointer-events: none;
    white-space: pre;
    overflow: hidden;
    color: var(--app-text-strong);
  }
  .rc-mirror > span {
    display: inline-block;
  }
  /* A recognised phrase, still text: tinted with a shadow so no glyph moves. */
  .rc-live {
    border-radius: 4px;
    background: var(--mx-selected);
    box-shadow: 0 0 0 3px var(--mx-selected);
    animation: rc-tint var(--t-med) var(--ease-quart) both;
  }
  @keyframes rc-tint {
    from {
      background: transparent;
      box-shadow: 0 0 0 3px transparent;
    }
  }
  .rc-ghost {
    color: var(--app-text-subtle);
    opacity: 0.7;
  }
  :global(.rc-bad) {
    color: var(--app-danger-text);
    text-decoration: underline wavy var(--app-danger-border);
    text-underline-offset: 4px;
  }

  .rc-right {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 12px;
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .rc-right span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .rc-right kbd {
    height: 18px;
  }
  .rc-right b {
    font-weight: 500;
    color: var(--app-text);
  }

  /* fieldFoot renders inside BarShell's `.rc-foot`. */
  .rc-foot__q {
    min-width: 0;
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    overflow: hidden;
  }
  .rc-foot__q code {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font: 400 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-muted);
  }
  .rc-foot__q code b {
    font-weight: 400;
    color: var(--app-text);
  }
  .rc-foot__q :global(svg) {
    flex: none;
  }
  .rc-foot__k {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .rc-foot__k span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .rc-foot__k kbd {
    height: 17px;
    min-width: 17px;
    padding: 0 4px;
  }

  @media (prefers-reduced-motion: reduce) {
    .rc-live {
      animation: none;
    }
    .rc-mode,
    .rc-mode__g :global(svg) {
      transition: none;
    }
  }
</style>
