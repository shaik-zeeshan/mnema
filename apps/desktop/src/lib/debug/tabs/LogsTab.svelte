<!--
  Logs tab (docs/mockups/redesign/developer.html ?state=logs): file + level
  segments, filter / follow / reveal, feature chips, and the tail filling the
  rest of the tab. Lines are oldest first, so Follow sticks to the bottom.
-->
<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import IconSearch from "~icons/lucide/search";
  import IconFolder from "~icons/lucide/folder";
  import IconFile from "~icons/lucide/file-text";
  import Segmented from "$lib/components/Segmented.svelte";
  import { tip } from "$lib/components/tooltip";
  import type { AppLogFile } from "$lib/types";
  import type { LogFeature } from "../log-filter";
  import type { LogLevelFilter } from "../log-line";
  import { getDebugController } from "../state/controller.svelte";
  import { createLogTailStore, LOG_TAIL_LINES } from "../state/logs.svelte";

  const controller = getDebugController();
  // The inspector's "Show in log" lands here with its needle.
  const logs = createLogTailStore(controller.poll, { needle: controller.nav.logNeedle });
  $effect(() => logs.startPolling());

  const FILES = [
    { value: "rust", label: "App log" },
    { value: "nativeCapture", label: "Capture log" },
  ];
  const LEVELS = [
    { value: "all", label: "All" },
    { value: "warn", label: "Warnings" },
    { value: "error", label: "Errors" },
  ];
  const FEATURES: { value: LogFeature | null; label: string }[] = [
    { value: null, label: "Everything" },
    { value: "capture", label: "Capture" },
    { value: "privacy", label: "Privacy" },
    { value: "ocr", label: "Reading" },
    { value: "transcription", label: "Transcription" },
    { value: "diarization", label: "Speakers" },
    { value: "embeddings", label: "Meaning" },
    { value: "aiRuntime", label: "AI" },
    { value: "userContext", label: "User Context" },
    { value: "jobsAndStorage", label: "Jobs" },
  ];

  const toneOf = (level: string | null) => (level === "WARN" ? "warn" : level === "ERROR" ? "danger" : undefined);

  let tailEl = $state<HTMLDivElement>();
  // Follow: after every render of new lines, pin to the newest. Off: leave the scroll alone.
  $effect(() => {
    void logs.entries;
    if (logs.follow && tailEl) tailEl.scrollTop = tailEl.scrollHeight;
  });

  let copied = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  function copyLine(event: MouseEvent) {
    const row = (event.target as Element).closest<HTMLElement>(".dv-line");
    const entry = row ? logs.entries[Number(row.dataset.i)] : undefined;
    if (!entry) return;
    void writeText(entry.raw).catch(() => {});
    copied = entry.raw;
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied = null), 1200);
  }
  $effect(() => () => clearTimeout(copiedTimer));

  const footer = $derived(
    (logs.totalLines >= LOG_TAIL_LINES ? `Showing the last ${LOG_TAIL_LINES} lines` : `Showing all ${logs.totalLines} lines`) +
      (logs.follow ? " · following" : ""),
  );
</script>

<div class="dv-logbar">
  <div class="dv-logbar__row">
    <Segmented options={FILES} value={logs.file} ariaLabel="Log file" onValueChange={(v) => (logs.file = v as AppLogFile)} />
    <span class="mx-spacer"></span>
    <Segmented options={LEVELS} value={logs.level} ariaLabel="Level" onValueChange={(v) => (logs.level = v as LogLevelFilter)} />
  </div>
  <div class="dv-logbar__row">
    <label class="mx-search">
      <IconSearch width="13" height="13" aria-hidden="true" />
      <input
        placeholder="Filter lines"
        autocomplete="off"
        spellcheck="false"
        value={logs.needle}
        oninput={(e) => (logs.needle = controller.nav.logNeedle = e.currentTarget.value)}
      />
    </label>
    <label class="dv-follow">
      <input type="checkbox" role="switch" class="mx-switch" checked={logs.follow} onchange={(e) => (logs.follow = e.currentTarget.checked)} />Follow
    </label>
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
      aria-label="Reveal in Finder"
      use:tip={"Reveal in Finder"}
      disabled={!logs.tail?.exists}
      onclick={() => logs.tail && void revealItemInDir(logs.tail.path)}
    ><IconFolder width="14" height="14" aria-hidden="true" /></button>
  </div>
  <div class="dv-feats" role="group" aria-label="Feature">
    {#each FEATURES as chip (chip.label)}
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        aria-pressed={logs.feature === chip.value}
        onclick={() => (logs.feature = chip.value)}
      >{chip.label}</button>
    {/each}
  </div>
</div>

<!-- Click-to-copy is a mouse convenience; the text stays selectable for everyone else. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="dv-tail" bind:this={tailEl} aria-label="Log tail, click a line to copy it" onclick={copyLine}>
  {#if logs.error}
    <div class="mx-empty mx-empty--compact" data-tone="danger">
      <span class="mx-empty__glyph"><IconFile width="14" height="14" aria-hidden="true" /></span>
      <b class="mx-empty__title">Couldn't read the log</b>
      <p class="mx-empty__text num">{logs.error}</p>
    </div>
  {:else if logs.tail && !logs.tail.exists}
    <div class="mx-empty mx-empty--compact">
      <span class="mx-empty__glyph"><IconFile width="14" height="14" aria-hidden="true" /></span>
      <b class="mx-empty__title">Nothing logged yet</b>
      <p class="mx-empty__text">Lines appear here as soon as the app writes them.</p>
    </div>
  {:else if logs.tail && logs.entries.length === 0}
    <div class="mx-empty mx-empty--compact">
      <span class="mx-empty__glyph"><IconSearch width="14" height="14" aria-hidden="true" /></span>
      <b class="mx-empty__title">No matching lines</b>
      <p class="mx-empty__text">Try another level or feature.</p>
    </div>
  {:else if logs.tail}
    {#each logs.entries as entry, i (i)}
      <div class="dv-line" class:is-copied={copied === entry.raw} data-i={i} data-tone={toneOf(entry.head?.level ?? null)} title="Click to copy">
        <span class="dv-line__t">{entry.head?.time ?? ""}</span>
        <span class="dv-line__l">{entry.head?.level ?? ""}</span>
        {#if entry.head}
          <span class="dv-line__m">{#if entry.head.prefix}<b>{entry.head.prefix}</b>{" "}{/if}{entry.head.message}</span>
        {/if}
        {#each entry.more as more, j (j)}
          <span class="dv-line__m dv-line__more">{more}</span>
        {/each}
      </div>
    {/each}
    <p class="dv-tail__end">{footer}</p>
  {/if}
</div>

<style>
  .dv-logbar { display: grid; gap: 8px; }
  .dv-logbar__row { display: flex; align-items: center; gap: var(--s-2); min-width: 0; }
  .dv-logbar .mx-search { flex: 1; height: var(--h-sm); padding-left: 8px; }
  .dv-logbar .mx-search input { font-size: var(--text-base); }
  .dv-follow { display: inline-flex; align-items: center; gap: 6px; font: 500 var(--text-base)/1 var(--font-sans); color: var(--app-text-muted); cursor: pointer; white-space: nowrap; }
  .dv-feats { display: flex; gap: 2px; overflow-x: auto; scrollbar-width: none; margin: 0 calc(-1 * var(--s-4)); padding: 0 var(--s-4);
    mask-image: linear-gradient(90deg, transparent, black 16px, black calc(100% - 24px), transparent); }
  .dv-feats::-webkit-scrollbar { display: none; }
  .dv-feats .mx-btn { --_h: 22px; --_px: 8px; flex: none; }
  .dv-tail { min-height: 0; overflow: auto; margin: 0 calc(-1 * var(--s-4)); padding: 4px 0 8px; border-top: 1px solid var(--mx-hairline);
    background: var(--app-surface-subtle); font: 450 var(--text-sm)/1.5 var(--font-mono); font-variant-numeric: tabular-nums; }
  .dv-line { display: grid; grid-template-columns: 82px 38px minmax(0, 1fr); column-gap: 8px; padding: 2px var(--s-4); color: var(--app-text-muted); cursor: copy; }
  .dv-line:hover { background: var(--mx-wash); }
  .dv-line__t { color: var(--app-text-subtle); }
  .dv-line__l { color: var(--app-text-subtle); font-size: var(--text-xs); line-height: 16.5px; }
  .dv-line__m { overflow-wrap: anywhere; color: var(--app-text); }
  .dv-line__m b { font-weight: 500; color: var(--app-text-muted); }
  .dv-line__more { grid-column: 3; white-space: pre-wrap; color: var(--app-text-muted); }
  .dv-line[data-tone] { background: color-mix(in srgb, var(--tone-bg) 70%, transparent); }
  .dv-line[data-tone] .dv-line__l, .dv-line[data-tone] .dv-line__m b { color: var(--tone); font-weight: 600; }
  .dv-line.is-copied .dv-line__l::after { content: " copied"; }
  .dv-tail__end { margin: 0; padding: 6px var(--s-4) 0; font: 400 var(--text-sm)/1 var(--font-sans); color: var(--app-text-subtle); }
  .dv-tail .mx-empty { padding: var(--s-4); }
</style>
