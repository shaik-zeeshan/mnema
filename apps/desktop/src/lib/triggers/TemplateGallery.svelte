<script lang="ts">
  // The wizard's Template step 0 (PLAN.md Slice 6; app-match/triggers.html
  // Frame 2): the static template card gallery + the Start-from-scratch door.
  // Pure presentation — picking/scratching is the wizard's business.
  import { CONDITION_SECTIONS } from "$lib/triggers/api";
  import { CONDITION_ICON } from "$lib/triggers/condition-icons";
  import { TRIGGER_TEMPLATES, type TriggerTemplate } from "$lib/triggers/templates";

  interface Props {
    onpick: (tpl: TriggerTemplate) => void;
    onscratch: () => void;
  }
  let { onpick, onscratch }: Props = $props();
</script>

<div class="wiz-step">
  <div class="gal-lead">
    <p class="wiz-lead">
      <span class="q">Start from a template.</span>
      <span class="hint">
        Each one fills in the condition and prompt for you — you only name it and save.
      </span>
    </p>
    <span class="gal-note">picking a template lands you on <b>Review</b>, ready to save</span>
  </div>
  <div class="gal-grid" role="group" aria-label="Templates">
    {#each TRIGGER_TEMPLATES as tpl (tpl.id)}
      {@const Icon = CONDITION_ICON[tpl.condition.type]}
      <button type="button" class="tpl" onclick={() => onpick(tpl)}>
        <span class="tpl-go" aria-hidden="true">→</span>
        <span class="tpl-glyph" aria-hidden="true"><Icon /></span>
        <span class="tpl-name">{tpl.title ?? tpl.name}</span>
        <span class="tpl-desc">{tpl.blurb}</span>
        <span class="tpl-cond"><Icon aria-hidden="true" />{tpl.condLine}</span>
      </button>
    {/each}
  </div>
  <button type="button" class="scratch" onclick={onscratch}>
    <span class="plus" aria-hidden="true">＋</span>
    <span class="scratch-copy">
      <b>Start from scratch</b>
      <span>Build your own — choose a condition, write the prompt, review, save.</span>
    </span>
    <span class="scratch-note" aria-hidden="true">begins at 01 Condition →</span>
  </button>
  <div class="glyph-legend" aria-hidden="true">
    {#each CONDITION_SECTIONS as section (section.cond)}
      {@const Icon = CONDITION_ICON[section.cond]}
      <span><Icon />{section.title.toLowerCase()}</span>
    {/each}
  </div>
</div>

<style>
  /* Token-clean (--app-*); .wiz-lead/.wiz-step come from wizard.css (global). */
  .gal-lead {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 24px;
  }
  .gal-note {
    font-size: 10.5px;
    color: var(--app-text-subtle);
    margin-bottom: 14px;
    white-space: nowrap;
  }
  .gal-note b {
    color: var(--app-text-muted);
    font-weight: 600;
  }
  .gal-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 12px;
  }
  .tpl {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 16px 16px 14px;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface);
    cursor: pointer;
    text-align: left;
    font: inherit;
    color: inherit;
    transition: border-color 0.14s ease, box-shadow 0.14s ease, transform 0.14s ease;
  }
  .tpl:hover {
    border-color: var(--app-accent-border);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
    transform: translateY(-2px);
  }
  .tpl:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }
  .tpl-go {
    position: absolute;
    top: 14px;
    right: 14px;
    font-size: 12px;
    color: var(--app-accent);
    opacity: 0;
    transform: translateX(-4px);
    transition: opacity 0.14s ease, transform 0.14s ease;
  }
  .tpl:hover .tpl-go,
  .tpl:focus-visible .tpl-go {
    opacity: 1;
    transform: translateX(0);
  }
  .tpl-glyph {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    margin-bottom: 10px;
  }
  .tpl-glyph :global(svg) {
    width: 14px;
    height: 14px;
  }
  .tpl-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .tpl-desc {
    font-size: 11px;
    line-height: 1.55;
    color: var(--app-text-muted);
    margin-top: 4px;
  }
  .tpl-cond {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
    letter-spacing: 0.03em;
    color: var(--app-text-subtle);
    margin-top: auto;
    padding-top: 10px;
  }
  .tpl-cond :global(svg) {
    width: 10px;
    height: 10px;
    color: var(--app-accent);
  }
  .scratch {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    margin-top: 14px;
    padding: 13px 18px;
    border: 1.5px dashed var(--app-border-strong);
    border-radius: 10px;
    background: transparent;
    cursor: pointer;
    text-align: left;
    font: inherit;
    color: inherit;
    transition: border-color 0.14s ease, background 0.14s ease;
  }
  .scratch:hover {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .scratch:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 1px;
  }
  .scratch .plus {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    flex: 0 0 auto;
    border-radius: 7px;
    border: 1px solid var(--app-border-strong);
    background: var(--app-surface);
    font-size: 13px;
    color: var(--app-text-muted);
  }
  .scratch:hover .plus {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }
  .scratch-copy b {
    display: block;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .scratch-copy span {
    display: block;
    font-size: 11px;
    color: var(--app-text-muted);
    margin-top: 2px;
  }
  .scratch-note {
    margin-left: auto;
    font-size: 10.5px;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .glyph-legend {
    display: flex;
    gap: 20px;
    margin-top: 16px;
    font-size: 10px;
    letter-spacing: 0.03em;
    color: var(--app-text-subtle);
  }
  .glyph-legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .glyph-legend :global(svg) {
    width: 10px;
    height: 10px;
    color: var(--app-accent);
  }
</style>
