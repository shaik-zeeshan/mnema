<script lang="ts">
  // "What Mnema is doing" (status.html › Part 2): the status bar's upward
  // popover, opened from the REC pill, the lane group or the disk flag. It shows
  // *now* only; the bell keeps events. Every word comes from `status-pop.ts`.
  import { tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import IconCheck from "~icons/lucide/check";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { getEffectiveGlobalShortcut, type GlobalShortcutId } from "$lib/global-shortcuts";
  import { formatShortcut, getFocusableElements, trapTabKey, type KeyboardPlatform } from "$lib/keyboard";
  import { renderIdle } from "$lib/render-idle.svelte";
  import { openDebugWindow, openSettings } from "$lib/surface-windows";
  import type { QuickRecallIndexStatus } from "$lib/types/app-infra";
  import type { ProcessorPipelineStatus } from "$lib/types/debug";
  import { captureHealth, statusPop, subscribeCaptureHealth } from "./status-live.svelte";
  import { diskSoon, sysAudioReconnecting } from "./status-bar";
  import { aiRow, captureGroup, catchingUp, diskRow, type Part, type PopInput, type PopRow } from "./status-pop";

  interface Props {
    platform: KeyboardPlatform;
    devEnabled: boolean;
    engine: PopInput["engine"];
    /** The trigger that opened it (focus returns there) and whether it was the keyboard. */
    trigger: HTMLElement | null;
    byKeyboard: boolean;
  }
  let { platform, devEnabled, engine, trigger, byKeyboard }: Props = $props();

  let popEl = $state<HTMLDivElement | null>(null);
  const open = $derived(statusPop.open);

  const keyOf = (id: GlobalShortcutId) => {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "";
  };

  // ── Data: only while open (2 s, renderIdle-gated, in-flight guarded) ──────
  let index = $state<QuickRecallIndexStatus | null>(null);
  let pipeline = $state<ProcessorPipelineStatus[] | null>(null);
  let deepgramAuthError = $state<string | null>(null);
  let inFlight = false;

  const transcriptionProvider = $derived.by(() => {
    const t = captureControls.recordingSettings?.transcription;
    return t?.enabled ? t.provider : null;
  });

  async function poll(): Promise<void> {
    if (renderIdle() || inFlight) return;
    inFlight = true;
    const [ix, pl] = await Promise.all([
      invoke<QuickRecallIndexStatus>("get_quick_recall_index_status").catch(() => null),
      invoke<ProcessorPipelineStatus[]>("get_processing_pipeline_status").catch(() => null),
    ]);
    const audio = pl?.find((p) => p.processor === "audio_transcription");
    // Key rejected vs unreachable: only asked while Deepgram jobs are retrying.
    const auth =
      audio && audio.retrying > 0 && transcriptionProvider === "deepgram"
        ? await invoke<string | null>("transcription_deepgram_auth_status").catch(() => null)
        : null;
    inFlight = false;
    if (!statusPop.open) return;
    index = ix;
    pipeline = pl;
    deepgramAuthError = auth;
  }

  $effect(() => {
    if (!open) return;
    return untrack(() => {
      const release = subscribeCaptureHealth();
      void poll();
      const handle = setInterval(poll, 2000);
      return () => {
        release();
        clearInterval(handle);
      };
    });
  });

  const input = $derived.by<PopInput>(() => {
    const running = captureControls.running;
    const lowDisk = captureControls.isLowDiskSuspended;
    const health = captureHealth.value;
    return {
      running,
      loadingStart: captureControls.loadingStart,
      lowDisk,
      health,
      sysAudioReconnecting: sysAudioReconnecting(health),
      diskSoon: diskSoon(health, running, lowDisk),
      index,
      pipeline,
      transcriptionProvider,
      deepgramAuthError,
      engine,
    };
  });
  const capture = $derived(captureGroup(input));
  const catching = $derived(catchingUp(input));
  const disk = $derived(diskRow(input));
  const ai = $derived(aiRow(input));
  const recordKey = $derived(keyOf("toggleRecording"));

  function close(): void {
    statusPop.open = false;
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Element | null;
    if (!target || popEl?.contains(target) || target.closest?.("[data-status-pop-trigger]")) return;
    close();
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (!open || event.key !== "Escape" || event.defaultPrevented) return;
    event.preventDefault();
    event.stopPropagation();
    close();
  }

  // Keyboard-opened: focus the first control. Close: return focus to the trigger.
  $effect(() => {
    if (!open) return;
    const from = untrack(() => trigger);
    const keyboard = untrack(() => byKeyboard);
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !keyboard) return;
      (getFocusableElements(popEl)[0] ?? popEl)?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (!active || active === document.body || popEl?.contains(active)) from?.focus({ preventScroll: true });
    };
  });

  function act(row: PopRow): void {
    if (!row.action) return;
    close();
    void openSettings(row.action.settings);
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

{#snippet parts(list: Part[])}{#each list as p}{#if typeof p === "string"}{p}{:else}<span class="num">{p.num}</span>{/if}{/each}{/snippet}

{#snippet rows(list: PopRow[])}
  <ul class="ss-rows">
    {#each list as row (row.name)}
      <li class="ss-row" data-tone={row.tone ?? undefined} data-dot={row.dot === "ring" ? "ring" : undefined}>
        <span class="ss-row__dot"></span>
        <span class="ss-row__name">{row.name}</span>
        <span class="ss-row__val">
          {@render parts(row.value)}
          {#if row.note}<small>{@render parts(row.note)}</small>{/if}
          {#if row.action}
            <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => act(row)}>{row.action.label}</button>
          {/if}
        </span>
      </li>
    {/each}
  </ul>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  id="status-popover"
  class="mx-pop mx-pop--up ss-pop"
  data-open={open ? "" : undefined}
  role="dialog"
  aria-label="What Mnema is doing"
  aria-hidden={!open}
  inert={!open}
  tabindex="-1"
  bind:this={popEl}
  onkeydown={(e) => trapTabKey(e, popEl)}
>
  <div class="mx-pop__head ss-pop__head"><span class="mx-kicker">What Mnema is doing</span></div>

  {#if capture.kind !== "rows" || capture.rows.length > 0}
    <section class="ss-grp">
      <span class="mx-label">Capture</span>
      {#if capture.kind === "rows"}
        {@render rows(capture.rows)}
      {:else if capture.kind === "starting"}
        <p class="ss-quiet">Starting…</p>
      {:else}
        <p class="ss-quiet">Not recording. Press Record{#if recordKey}{" or "}<kbd>{recordKey}</kbd>{/if}.</p>
      {/if}
    </section>
  {/if}

  {#if catching}
    <section class="ss-grp">
      <span class="mx-label">Catching up</span>
      {#if catching.kind === "caught-up"}
        <span class="mx-inline ss-quiet"><IconCheck width="13" height="13" aria-hidden="true" />All caught up</span>
      {:else}
        {#if catching.rows.length > 0}{@render rows(catching.rows)}{/if}
        {#if catching.meaningPercent !== null}
          {@const pct = Math.round(catching.meaningPercent)}
          <div class="ss-prog">
            <div class="mx-progress__meta"><b>Building meaning index</b><span>{pct}%</span></div>
            <div
              class="mx-progress"
              role="progressbar"
              aria-label="Building meaning index"
              aria-valuenow={pct}
              aria-valuemin="0"
              aria-valuemax="100"
              style:--v={pct / 100}
            ><i></i></div>
          </div>
        {/if}
      {/if}
    </section>
  {/if}

  {#if disk}
    <section class="ss-grp"><span class="mx-label">Disk</span>{@render rows([disk])}</section>
  {/if}

  {#if ai}
    <section class="ss-grp"><span class="mx-label">AI</span>{@render rows([ai])}</section>
  {/if}

  {#if devEnabled}
    {@const debugKey = keyOf("openDebug")}
    <div class="ss-pop__foot">
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        onclick={() => {
          close();
          void openDebugWindow();
        }}
      >Open developer window{#if debugKey}<kbd>{debugKey}</kbd>{/if}</button>
    </div>
  {/if}
</div>

<style>
  /* Mounted inside the mono, nowrap status bar: reset to the popover's sans voice. */
  .mx-pop.ss-pop {
    left: -4px; right: auto; width: 340px; transform-origin: bottom left; padding-bottom: var(--s-2);
    font: 400 var(--text-base)/1.4 var(--font-sans); letter-spacing: normal; white-space: normal;
    color: var(--app-text); cursor: default;
  }
  .mx-pop.ss-pop:focus { outline: none; }
  .ss-pop__head { display: flex; align-items: center; gap: var(--s-2); }
  .ss-grp { padding: var(--s-2) var(--s-4) var(--s-2); border-top: 1px solid var(--mx-hairline); display: grid; gap: 6px; }
  .ss-grp > .mx-label { display: block; }
  .ss-grp p { margin: 0; }
  .ss-rows { list-style: none; margin: 0; padding: 0; display: grid; gap: 7px; }
  .ss-row { display: grid; grid-template-columns: 8px 92px minmax(0, 1fr); gap: 8px; align-items: baseline; font: 400 var(--text-base)/1.4 var(--font-sans); }
  .ss-row__dot { width: 7px; height: 7px; border-radius: 50%; translate: 0 -1px; background: var(--tone, var(--app-text-faint)); }
  .ss-row[data-dot="ring"] .ss-row__dot { background: none; box-shadow: inset 0 0 0 1.5px var(--app-text-muted); }
  .ss-row__name { color: var(--app-text-strong); font-weight: 500; }
  .ss-row__val { color: var(--app-text-muted); min-width: 0; }
  .ss-row[data-tone="warn"] .ss-row__val { color: var(--app-text); }
  .ss-row__val .mx-btn { --_h: 22px; --_px: 6px; display: flex; width: max-content; margin: 2px 0 0 -6px; }
  .ss-row__val small { display: block; font: inherit; color: var(--app-text-muted); }
  .ss-prog { padding-top: 2px; }
  .ss-prog .mx-progress__meta { margin-bottom: 6px; }
  .ss-quiet { color: var(--app-text-subtle); }
  .ss-pop__foot { margin: var(--s-1) var(--s-2) 0; padding-top: var(--s-2); border-top: 1px solid var(--mx-hairline); }
  .ss-pop__foot .mx-btn { width: 100%; justify-content: flex-start; }
  .ss-pop__foot .mx-btn kbd { margin-left: auto; }
</style>
