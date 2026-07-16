<script lang="ts">
  // The reusable log-tail viewer: file switch, per-feature filter chips, follow
  // mode, and a bounded scrollback of the last LOG_TAIL_LINES lines.
  //
  // Used by `sections/LogsSection.svelte` today and by slice 7's per-feature
  // "Log tail" sub-tab next — hence a component with its own store rather than
  // markup inlined into the section. Each instance owns its file/filter/follow
  // state and its own poll loop, so two viewers never fight over one.

  import { untrack } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import Segmented from "$lib/components/Segmented.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import IconFilterX from "~icons/lucide/filter-x";
  import { DEBUG_SECTIONS } from "./sections";
  import { logLineClass } from "./log-filter";
  import { LOG_FILE_OPTIONS, LOG_TAIL_LINES, createLogTailStore } from "./state/logs.svelte";
  import type { AppLogFile, DebugFeature } from "$lib/types";

  interface Props {
    /** Log to open on. The file switch can still change it. Default `rust`. */
    file?: AppLogFile;
    /** Feature filter to open on; `null` (default) is the "all" chip. */
    feature?: DebugFeature | null;
    /** Text filter to open on (e.g. a job id) — the input can still change it. */
    needle?: string;
  }

  let { file = "rust", feature = null, needle = "" }: Props = $props();

  // Seed values only — the store owns file/feature/needle from here on, so a
  // later prop change must NOT stomp what the user picked. `untrack` says that
  // to the compiler as much as to the reader.
  const logs = createLogTailStore(untrack(() => ({ file, feature, needle })));

  // The 9 features that have a chip, with their real section labels — the
  // section registry is already the source of truth for that list.
  const FEATURE_CHIPS = DEBUG_SECTIONS.filter((section) => section.healthFeature != null);

  let viewport = $state<HTMLDivElement | null>(null);

  $effect(() => logs.startPolling());

  // Follow mode: re-pin to the newest line whenever the rendered rows change.
  // `rows` is the tracked read that drives this; $effect runs after the DOM
  // updates, so scrollHeight is already the new one.
  $effect(() => {
    const rows = logs.lines;
    if (!logs.follow || rows.length === 0) return;
    const el = viewport;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  });

  // Follow disengages by scrolling up and re-engages by scrolling back down —
  // the viewer must never yank the view out from under someone reading
  // scrollback. The pin above lands at the bottom, so the scroll event it
  // triggers reads `atBottom` and leaves follow alone (no feedback loop).
  function onScroll() {
    const el = viewport;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    if (logs.follow !== atBottom) logs.follow = atBottom;
  }
</script>

<div class="log-tail">
  <div class="log-toolbar">
    <Segmented
      options={LOG_FILE_OPTIONS}
      value={logs.file}
      onValueChange={(value) => (logs.file = value as AppLogFile)}
      ariaLabel="Log file"
      compact
    />
    <div class="log-toolbar__follow">
      <Switch
        checked={logs.follow}
        onCheckedChange={(value) => (logs.follow = value)}
        label="Follow"
      />
    </div>
  </div>

  <div class="log-chips" role="group" aria-label="Filter log lines by feature">
    <button
      type="button"
      class="log-chip"
      class:log-chip--active={logs.feature == null}
      aria-pressed={logs.feature == null}
      onclick={() => (logs.feature = null)}
    >
      all
    </button>
    {#each FEATURE_CHIPS as section (section.id)}
      <button
        type="button"
        class="log-chip"
        class:log-chip--active={logs.feature === section.healthFeature}
        aria-pressed={logs.feature === section.healthFeature}
        onclick={() => (logs.feature = section.healthFeature)}
      >
        {section.label}
      </button>
    {/each}
    <!-- Free-text needle over the tailed lines — client-side, like the chips.
         The inspector's "filter log to this job" seeds it with a job id. -->
    <input
      class="log-needle"
      type="search"
      placeholder="filter…"
      aria-label="Filter log lines by text"
      value={logs.needle}
      oninput={(event) => (logs.needle = event.currentTarget.value)}
    />
  </div>

  {#if logs.error}
    <p class="debug-err" role="alert" aria-live="polite">{logs.error}</p>
  {/if}

  <div class="log-meta">
    <span class="log-meta__path" use:tip={logs.tail?.path ?? ""}>{logs.tail?.path ?? "…"}</span>
    <span class="log-meta__count">
      {#if logs.feature != null || logs.needle.trim() !== ""}
        {logs.lines.length} of {logs.totalLines} lines
      {:else}
        {logs.totalLines} lines
      {/if}
      · last {LOG_TAIL_LINES} tailed
    </span>
  </div>

  <!-- `role="region"`, deliberately NOT `role="log"`: log's implicit
       aria-live="polite" would have a screen reader announce the rows the 2s
       poll patches, forever. `tabindex="0"` trips the noninteractive-tabindex
       rule, but a scrollback nobody can reach by keyboard is the worse outcome
       — WebKit won't focus an overflow container on its own. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="log-view" bind:this={viewport} onscroll={onScroll} tabindex="0" role="region" aria-label="Log tail">
    {#if logs.tail == null}
      <p class="empty">reading log…</p>
    {:else if !logs.tail.exists}
      <!-- `exists: false` is the calm case, not a failure: the log was deleted
           or never written. The poll keeps running, so it fills in by itself
           once the app writes the file again. -->
      <p class="empty">log file not present — it will appear here once the app writes to it</p>
    {:else if logs.totalLines === 0}
      <p class="empty">log file is empty</p>
    {:else if logs.lines.length === 0}
      <p class="empty">
        <span class="log-empty__glyph" aria-hidden="true"><IconFilterX /></span>
        no lines in this tail match the filter
      </p>
    {:else}
      <!-- Keyed by index on purpose: log lines are not unique, and an index key
           lets Svelte reuse the row nodes and only patch their text as the
           window slides — the cheap path for a list replaced every poll. -->
      {#each logs.lines as line, index (index)}
        <div class={logLineClass(line)}>{line}</div>
      {/each}
    {/if}
  </div>
</div>
