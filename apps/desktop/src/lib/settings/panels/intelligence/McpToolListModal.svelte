<script lang="ts">
  // McpToolListModal — the "See tool list" curation surface for one MCP
  // connector (Workstream C, C3). On open it connects the server ON DEMAND via
  // the `mcp_list_server_tools` command (spinner during an npx cold boot,
  // readable error + retry on failure), then lists every discovered tool with a
  // checkbox. Toggling a checkbox writes the server's `enabledTools` back through
  // `onCurate` — which rides the existing ai_runtime autosave (no extra save).
  //
  // Curation semantics MIRROR the Rust `offered_tools` (ADR 0048): a null/absent
  // `enabledTools` is the default-offer (first 32 checked); the first toggle
  // MATERIALIZES an explicit list (curated); a curated list checks only names
  // that still exist, so newly-appeared tools stay unchecked (drift). See
  // `mcp-tool-curation.ts` for the pure, unit-tested logic.
  //
  // Overlay / focus / ESC / backdrop scaffold mirrors CategoryDetailModal.

  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { humanizeError } from "$lib/format-error";
  import { trapTabKey } from "$lib/keyboard";
  import Checkbox from "$lib/components/Checkbox.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { activeToolNames, toggleTool } from "$lib/settings/state/mcp-tool-curation";
  import type { McpServerConfig } from "$lib/types";

  interface McpToolDescriptor {
    name: string;
    description: string | null;
  }

  interface Props {
    open: boolean;
    /** The draft connector being curated (null while closed). */
    server: McpServerConfig | null;
    onClose: () => void;
    /** Persist a new curation: an array (curated) or null (reset to default). */
    onCurate: (enabledTools: string[] | null) => void;
    /** Report the discovered tool count so the card caption can show "N tools". */
    onToolsDiscovered?: (id: string, count: number) => void;
  }

  let { open, server, onClose, onCurate, onToolsDiscovered }: Props = $props();

  let loading = $state(false);
  let error = $state<string | null>(null);
  let tools = $state<McpToolDescriptor[]>([]);

  const allNames = $derived(tools.map((t) => t.name));
  // Reads `server.enabledTools` so a curation write re-derives the checked set —
  // the checkboxes are controlled, reflecting the persisted state immediately.
  const checkedNames = $derived(new Set(activeToolNames(allNames, server?.enabledTools ?? null)));

  async function load(id: string): Promise<void> {
    loading = true;
    error = null;
    try {
      const result = await invoke<McpToolDescriptor[]>("mcp_list_server_tools", { id });
      tools = result;
      onToolsDiscovered?.(id, result.length);
    } catch (err) {
      error = humanizeError(err);
      tools = [];
    } finally {
      loading = false;
    }
  }

  function onToggle(name: string, active: boolean): void {
    onCurate(toggleTool(allNames, server?.enabledTools ?? null, name, active));
  }

  // Move focus into the dialog on open (WebKit hands the opener no focus), load
  // the tool list fresh each open, and restore focus + reset transient state on
  // close. Mirrors CategoryDetailModal's opener-capture/return.
  let panelEl = $state<HTMLDivElement | null>(null);
  let opener: HTMLElement | null = null;
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      opener = document.activeElement as HTMLElement | null;
      panelEl?.focus();
      if (server) void load(server.id);
    } else if (!open && wasOpen) {
      const trigger = opener;
      opener = null;
      tools = [];
      error = null;
      loading = false;
      void tick().then(() => trigger?.focus());
    }
    wasOpen = open;
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (!open) return;
    if (trapTabKey(e, panelEl)) return;
    if (e.key === "Escape") onClose();
  }}
/>

{#if open && server}
  <div
    class="cat-modal"
    role="presentation"
    onpointerdown={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div
      bind:this={panelEl}
      class="cat-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="mcp-tools-title"
      tabindex="-1"
    >
      <header class="cat-modal__header">
        <div>
          <p class="cat-modal__eyebrow">MCP connector</p>
          <h2 id="mcp-tools-title">{server.label.trim() || server.id} · tools</h2>
        </div>
        <button
          type="button"
          class="cat-modal__close"
          aria-label="Close tool list"
          onclick={onClose}>×</button
        >
      </header>

      <div class="cat-modal__body">
        {#if loading}
          <div class="mcp-tools__status" role="status">
            <ButtonSpinner />Connecting to {server.label.trim() || server.id}…
          </div>
        {:else if error}
          <div class="mcp-tools__status mcp-tools__status--error" role="alert">
            <p class="mcp-tools__error-text">{error}</p>
            <button
              type="button"
              class="btn btn--ghost btn--sm"
              onclick={() => server && load(server.id)}
            >
              Retry
            </button>
          </div>
        {:else if tools.length === 0}
          <p class="cat-modal__empty">This connector exposes no tools.</p>
        {:else}
          <div class="mcp-tools__toolbar">
            <span class="mcp-tools__count">
              {tools.length} tools · {checkedNames.size} active
            </span>
            <div class="mcp-tools__toolbar-actions">
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={() => onCurate(allNames.slice())}
              >
                Select all
              </button>
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={() => onCurate([])}
              >
                Select none
              </button>
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={() => onCurate(null)}
              >
                Reset to default
              </button>
            </div>
          </div>
          <ul class="mcp-tools__list">
            {#each tools as t (t.name)}
              <li class="mcp-tools__row">
                <Checkbox
                  checked={checkedNames.has(t.name)}
                  onCheckedChange={(v) => onToggle(t.name, v)}
                  label={t.name}
                  description={t.description ?? undefined}
                />
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay + panel (mirrors CategoryDetailModal) ---- */
  .cat-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--app-overlay-bg);
    backdrop-filter: blur(10px);
  }
  .cat-modal__panel {
    width: min(560px, 100%);
    max-height: min(720px, calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover);
  }
  .cat-modal__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 18px 12px;
  }
  .cat-modal__eyebrow {
    margin: 0 0 2px;
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .cat-modal__header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    overflow-wrap: anywhere;
  }
  .cat-modal__close {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--text-lg);
    line-height: 1;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .cat-modal__close:hover,
  .cat-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }
  .cat-modal__close:focus-visible {
    box-shadow: var(--app-ring);
  }
  .cat-modal__close:not(:disabled):active {
    transform: translateY(1px);
  }
  .cat-modal__body {
    overflow-y: auto;
    padding: 0 18px 18px;
  }
  .cat-modal__empty {
    margin: 0;
    padding: 8px 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }

  /* ---- Loading / error status ---- */
  .mcp-tools__status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px 0;
    font-size: 12px;
    color: var(--app-text-muted);
  }
  .mcp-tools__status--error {
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }
  .mcp-tools__error-text {
    margin: 0;
    color: var(--app-warn);
    overflow-wrap: anywhere;
  }

  /* ---- Toolbar (count + bulk actions) ---- */
  .mcp-tools__toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 0;
    position: sticky;
    top: 0;
    background: var(--app-surface-raised);
    z-index: 1;
  }
  .mcp-tools__count {
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .mcp-tools__toolbar-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  /* ---- Tool rows ---- */
  .mcp-tools__list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .mcp-tools__row {
    padding: 9px 0;
  }
  .mcp-tools__row + .mcp-tools__row {
    border-top: 1px dashed var(--app-border);
  }

  @media (prefers-reduced-motion: reduce) {
    .cat-modal__close {
      transition: none;
    }
    .cat-modal__close:not(:disabled):active {
      transform: none;
    }
  }
</style>
