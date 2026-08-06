<script lang="ts">
  // Context — a DESTINATION (page 10), opened from the Overview's Context tile.
  //
  // The main window still has exactly two surfaces (Timeline · Overview); this
  // one comes back through the tool strip's first control. Inside, the shape is
  // the direction's: the tool strip navigates, one region scrolls, the inspector
  // carries the record, and the title/status strips belong to the root layout.
  //
  // The surface owns exactly ONE kind of data: authored context. The dismissed
  // archive at the bottom is inferred data, and it is labelled as such — every
  // other inferred belief lives on Subjects.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { ContextData } from "$lib/context/context-data.svelte";
  import ToolStrip from "$lib/context/ToolStrip.svelte";
  import Composer from "$lib/context/Composer.svelte";
  import Ledger from "$lib/context/Ledger.svelte";
  import Dismissed from "$lib/context/Dismissed.svelte";
  import Inspector from "$lib/context/Inspector.svelte";

  const data = new ContextData();

  onMount(() => data.start());

  // Same floor as Overview: the inspector collapses below 1000px of pane.
  let paneWidth = $state(1100);
  const wide = $derived(paneWidth >= 1000);
  let inspectorPinned = $state(true);
  const inspectorOpen = $derived(wide && inspectorPinned);

  let draftText = $state("");
  let draftTopic = $state("");
  let submitting = $state(false);
  let composerError = $state<string | null>(null);

  async function submitDraft(): Promise<void> {
    const text = draftText.trim();
    if (text.length === 0 || submitting) return;
    submitting = true;
    const failure = await data.add(text, draftTopic.trim());
    submitting = false;
    if (failure) {
      composerError = failure;
      return;
    }
    composerError = null;
    draftText = "";
    draftTopic = "";
  }
</script>

<div class="ctx" bind:clientWidth={paneWidth}>
  <ToolStrip
    {data}
    {inspectorOpen}
    inspectorAvailable={wide}
    onback={() => void goto("/overview")}
    ontoggleinspector={() => (inspectorPinned = !inspectorPinned)}
  />

  <div class="ss-body">
    <div class="ss-main">
      <div class="scroll">
        <!-- The composer is the top of the scroll rather than a modal: adding
             context is the reason you came to this page. -->
        <div class="chd">
          <span class="chd__n">Add context</span>
          <span class="t-meta note"
            >What you tell Mnema about yourself — it steers your dossier and never fades like an
            inferred conclusion</span
          >
        </div>
        <div class="composerwrap">
          <Composer
            variant="add"
            bind:text={draftText}
            bind:topic={draftTopic}
            busy={submitting}
            error={composerError}
            onsubmit={() => void submitDraft()}
          />
        </div>

        <Ledger {data} />
        <Dismissed {data} />
      </div>
    </div>

    {#if inspectorOpen}
      <Inspector {data} />
    {/if}
  </div>
</div>

<style>
  .ctx {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: var(--s-12);
  }

  .chd {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    height: 26px;
    padding: 0 var(--s-16);
    background: var(--app-bg);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .chd__n {
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }

  .note {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .composerwrap {
    margin: var(--s-12) var(--s-16) 0;
  }
</style>
