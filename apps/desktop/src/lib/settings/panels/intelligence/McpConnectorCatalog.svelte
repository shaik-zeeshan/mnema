<script lang="ts">
  // McpConnectorCatalog — step 1 of the Add-connector picker (split out of
  // McpConnectorPicker.svelte for the 800-line cap): the searchable 2-column
  // preset grid (hosted / local groups), "✓ added" badges, and the dashed
  // Custom escape-hatch tile. Search filters tiles; Enter picks the first
  // visible one (Custom when nothing matches — mirrors the mockup).

  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import { MCP_PRESETS, type McpPreset } from "$lib/settings/state/mcp-presets";

  interface Props {
    onPick: (preset: McpPreset) => void;
    onPickCustom: () => void;
  }

  let { onPick, onPickCustom }: Props = $props();

  const rec = getSettingsController().rec;

  let query = $state("");
  const q = $derived(query.trim().toLowerCase());
  const matchesQuery = (p: McpPreset) =>
    !q || `${p.label} ${p.tagline}`.toLowerCase().includes(q);
  const hostedPresets = $derived(MCP_PRESETS.filter((p) => p.kind === "hosted" && matchesQuery(p)));
  const localPresets = $derived(MCP_PRESETS.filter((p) => p.kind === "local" && matchesQuery(p)));

  // "✓ added": a draft already carries this preset's slugged id or a
  // slugger-suffixed variant ("github", "github-2", …). Still selectable —
  // duplicates are allowed (the slugger + label suffix keep them distinct).
  function slugOf(label: string): string {
    return (
      label
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "") || "connector"
    );
  }
  const addedPresetIds = $derived.by(() => {
    const ids = new Set<string>();
    for (const p of MCP_PRESETS) {
      const re = new RegExp(`^${slugOf(p.label)}(-\\d+)?$`);
      if (rec.draftMcpServers.some((s) => re.test(s.id))) ids.add(p.id);
    }
    return ids;
  });

  function onSearchKeydown(e: KeyboardEvent): void {
    if (e.key !== "Enter") return;
    e.preventDefault();
    const first = hostedPresets[0] ?? localPresets[0];
    if (first) onPick(first);
    else onPickCustom();
  }
</script>

{#snippet presetTile(p: McpPreset)}
  <button type="button" class="tile" onclick={() => onPick(p)}>
    <span class="tile__icon">{@html p.brandSvg}</span>
    <span class="tile__text">
      <span class="tile__name">
        {p.label}
        {#if addedPresetIds.has(p.id)}<span class="tile__badge tile__badge--added">✓ added</span>{/if}
      </span>
      <span class="tile__tagline">{p.tagline}</span>
    </span>
  </button>
{/snippet}

<input
  class="text-input catalog-search"
  type="text"
  placeholder="Search services…"
  aria-label="Search services"
  bind:value={query}
  onkeydown={onSearchKeydown}
/>
{#if hostedPresets.length > 0}
  <div class="catalog-group">
    <p class="catalog-eyebrow">Hosted — just paste a token</p>
    <div class="catalog-grid">
      {#each hostedPresets as p (p.id)}{@render presetTile(p)}{/each}
    </div>
  </div>
{/if}
{#if localPresets.length > 0}
  <div class="catalog-group">
    <p class="catalog-eyebrow">Runs on this Mac</p>
    <div class="catalog-grid">
      {#each localPresets as p (p.id)}{@render presetTile(p)}{/each}
    </div>
  </div>
{/if}
{#if hostedPresets.length === 0 && localPresets.length === 0}
  <p class="catalog-empty">No service matches “{query.trim()}” — add it yourself:</p>
{/if}
<div class="catalog-custom">
  <button type="button" class="tile tile--custom" onclick={onPickCustom}>
    <span class="tile__icon">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 21v-7" /><path d="M4 10V3" /><path d="M12 21v-9" /><path d="M12 8V3" /><path d="M20 21v-5" /><path d="M20 12V3" /><path d="M1 14h6" /><path d="M9 8h6" /><path d="M17 16h6" /></svg>
    </span>
    <span class="tile__text">
      <span class="tile__name">Custom</span>
      <span class="tile__tagline">Any other MCP server — fill in every field yourself.</span>
    </span>
  </button>
</div>

<style>
  .catalog-search {
    width: 100%;
    margin-bottom: 14px;
  }
  .catalog-eyebrow {
    margin: 14px 0 8px 2px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .catalog-group:first-of-type .catalog-eyebrow {
    margin-top: 0;
  }
  .catalog-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .tile {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 11px 12px;
    text-align: left;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface);
    font-family: inherit;
    cursor: pointer;
    transition:
      background 0.15s,
      border-color 0.15s;
    outline: none;
  }
  .tile:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .tile:active {
    background: var(--app-surface-active);
  }
  .tile:focus-visible {
    box-shadow: var(--app-ring);
  }
  .tile__icon {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 7px;
    color: var(--app-text-muted);
  }
  .tile__icon :global(svg) {
    width: 16px;
    height: 16px;
  }
  .tile__text {
    min-width: 0;
    flex: 1;
  }
  .tile__name {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .tile__tagline {
    display: block;
    margin-top: 2px;
    font-size: 10.5px;
    line-height: 1.35;
    color: var(--app-text-muted);
  }
  .tile__badge {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-radius: 4px;
    padding: 1px 5px;
  }
  .tile__badge--added {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
  }
  .tile--custom {
    width: 100%;
    border-style: dashed;
    background: transparent;
  }
  .tile--custom .tile__name {
    color: var(--app-text-muted);
  }
  .tile--custom:hover {
    background: var(--app-surface-hover);
  }
  .catalog-custom {
    margin-top: 8px;
  }
  .catalog-empty {
    margin: 0;
    padding: 4px 2px 12px;
    font-size: 11px;
    color: var(--app-text-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    .tile {
      transition: none;
    }
  }
</style>
