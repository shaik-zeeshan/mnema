<!--
  Settings → Data → Storage: Delete Recent Capture, the same command the menu
  bar's submenu runs (which stays). Same confirm wording as the tray; unlike the
  tray, a one-line receipt under the row says it happened.
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { ask } from "@tauri-apps/plugin-dialog";
  import Segmented from "$lib/components/Segmented.svelte";
  import { humanizeError } from "$lib/format-error";
  import IconTrash from "~icons/lucide/trash-2";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import {
    DELETE_WINDOW_OPTIONS,
    deleteConfirmMessage,
    deleteReceipt,
    deleteWindowPhrase,
    type DeleteWindowSeconds,
  } from "./delete-recent-capture";

  let windowValue = $state("300");
  let busy = $state(false);
  let receipt = $state<{ tone: "ok" | "warn" | "danger"; text: string } | null>(null);
  const seconds = $derived(Number(windowValue) as DeleteWindowSeconds);

  async function deleteRecent() {
    if (busy) return;
    const s = seconds;
    const ok = await ask(deleteConfirmMessage(s), {
      title: "Delete Recent Capture",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    busy = true;
    receipt = null;
    try {
      const summary = await invoke<{ fileDeleteErrors: number }>("delete_recent_capture", {
        request: { windowSeconds: s },
      });
      receipt = deleteReceipt(s, summary.fileDeleteErrors);
    } catch (err) {
      receipt = { tone: "danger", text: `Couldn’t delete the ${deleteWindowPhrase(s)}: ${humanizeError(err)}` };
    } finally {
      busy = false;
    }
  }
</script>

<div class="st-danger">
  <div class="st-danger__row">
    <div>
      <div class="st-danger__label">Delete Recent Capture</div>
      <p class="st-danger__desc">
        Permanently removes every segment that overlaps the window — screen, audio, OCR text, and
        transcripts. Can’t be undone.
      </p>
    </div>
    <div class="st-danger__ctl">
      <Segmented md mono options={DELETE_WINDOW_OPTIONS} bind:value={windowValue} ariaLabel="Delete window" disabled={busy} />
      <button type="button" class="mx-btn mx-btn--danger" onclick={deleteRecent} disabled={busy} aria-busy={busy}>
        {#if busy}<span class="mx-spin mx-spin--sm"></span>Deleting…{:else}<IconTrash width="14" height="14" />Delete last {seconds / 60} min…{/if}
      </button>
    </div>
  </div>
  {#if receipt}
    <p class="mx-inline" data-tone={receipt.tone} role="status">
      {#if receipt.tone === "ok"}<IconCheck width="13" height="13" />{:else}<IconAlert width="13" height="13" />{/if}
      <span>{receipt.text}</span>
    </p>
  {/if}
  <p class="st-danger__note">Also in the menu bar · Retention cleanup is separate and never touches User Context.</p>
</div>

<style>
  /* Mockup `.st-danger` (settings.html › Storage): one danger-tinted box. */
  .st-danger {
    border: 1px solid var(--app-danger-border);
    border-radius: var(--r-md);
    background: var(--app-danger-bg-soft);
    padding: 0 var(--s-4);
  }
  .st-danger__row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-6);
    padding: 14px 0;
  }
  .st-danger__label {
    font: 500 var(--text-md)/1.35 var(--font-sans);
    color: var(--app-danger-text);
  }
  .st-danger__desc {
    margin: 3px 0 0;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
    max-width: 52ch;
  }
  .st-danger .mx-inline {
    display: flex;
    flex-wrap: nowrap;
    margin: -4px 0 var(--s-3);
  }
  .st-danger__ctl {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    justify-self: end;
  }
  .st-danger__note {
    margin: 0;
    padding: 0 0 var(--s-3);
    font: 400 var(--text-sm)/1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
</style>
