<script lang="ts">
  // One "model status" row for OCR / Transcription / Speakers / Semantic
  // Search (settings.html #s-ocr…#s-semantic): status word or Download (size)
  // on the right, a chip for failed/missing, and — while a download runs — a
  // stack row with the kit progress bar and Cancel.
  import type { Snippet } from "svelte";
  import SettingRow from "./SettingRow.svelte";
  import ButtonSpinner from "./ButtonSpinner.svelte";
  import type { DownloadMeta } from "./model-download";
  import { formatBytes } from "$lib/settings/state/format";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInfo from "~icons/lucide/info";
  import IconDownload from "~icons/lucide/download";
  import IconRetry from "~icons/lucide/rotate-cw";

  interface Props {
    label: string;
    description?: string;
    chip?: { text: string; tone?: "warn" | "danger" | "accent" } | null;
    /** Non-null = the running layout (meta + bar). `percent: null` = indeterminate. */
    progress?: DownloadMeta | null;
    onCancel?: () => void;
    cancelling?: boolean;
    cancelLabel?: string;
    /** Danger line under the copy (a failed download / action). */
    error?: string | null;
    /** Warn line under the copy (runtime problem, nothing to download). */
    warning?: string | null;
    /** Healthy status word (`.settings-state--ok`). */
    status?: string | null;
    download?: { bytes: number | null; starting: boolean; onclick: () => void; disabled?: boolean } | null;
    /** Extra right-side controls (model select, Delete…), between status and Download. */
    controls?: Snippet;
    /** Extra lines under the copy. */
    note?: Snippet;
    /** Install path / licence / source / missing files, folded into one <details>. */
    info?: {
      installPath?: string | null;
      licenseLabel?: string | null;
      sourceUrl?: string | null;
      missingFiles?: string[];
    } | null;
  }

  let {
    label,
    description,
    chip = null,
    progress = null,
    onCancel,
    cancelling = false,
    cancelLabel = "Cancel download",
    error = null,
    warning = null,
    status = null,
    download = null,
    controls,
    note,
    info = null,
  }: Props = $props();

  const hasInfo = $derived(
    !!info && !!(info.installPath || info.licenseLabel || info.sourceUrl || info.missingFiles?.length),
  );
  const hasNotes = $derived(!!error || !!warning || !!note || hasInfo);
  const failed = $derived(!!error || chip?.tone === "danger");
</script>

{#snippet labelChip()}
  {#if chip}<span class="mx-chip {chip.tone ? `mx-chip--${chip.tone}` : ''}">{chip.text}</span>{/if}
{/snippet}

{#snippet right()}
  {#if status}<span class="settings-state settings-state--ok">{status}</span>{/if}
  {#if controls}{@render controls()}{/if}
  {#if download}
    <button
      type="button"
      class="mx-btn"
      onclick={download.onclick}
      disabled={download.starting || download.disabled}
      aria-busy={download.starting}
    >
      {#if download.starting}
        <ButtonSpinner />Starting
      {:else}
        {#if failed}<IconRetry width="13" height="13" />{:else}<IconDownload width="13" height="13" />{/if}
        Download{download.bytes != null ? ` (${formatBytes(download.bytes)})` : ""}
      {/if}
    </button>
  {/if}
{/snippet}

{#if progress}
  <SettingRow {label} {description} labelExtra={chip ? labelChip : undefined} full>
    {#snippet aside()}{#if controls}{@render controls()}{/if}{/snippet}
    {#snippet control()}
      <div class="mdr-stack">
        <div>
          <div class="mx-progress__meta"><b>{progress.title}</b><span>{progress.detail}</span></div>
          <div
            class="mx-progress"
            class:mx-progress--indeterminate={progress.percent === null}
            role="progressbar"
            aria-label={progress.title}
            aria-valuenow={progress.percent ?? undefined}
            aria-valuemin={progress.percent === null ? undefined : 0}
            aria-valuemax={progress.percent === null ? undefined : 100}
            style={progress.percent === null ? undefined : `--v:${progress.percent / 100}`}
          ><i></i></div>
        </div>
        {#if onCancel}
          <div>
            <button type="button" class="mx-btn mx-btn--sm" onclick={onCancel} disabled={cancelling} aria-busy={cancelling}>
              {#if cancelling}<ButtonSpinner />Cancelling{:else}{cancelLabel}{/if}
            </button>
          </div>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
{:else}
  <SettingRow
    {label}
    {description}
    labelExtra={chip ? labelChip : undefined}
    control={right}
    below={hasNotes ? notes : undefined}
  />
{/if}

{#snippet notes()}
  {#if error}
    <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{error}</span>
  {/if}
  {#if warning}
    <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />{warning}</span>
  {/if}
  {#if note}{@render note()}{/if}
  {#if hasInfo && info}
    <details class="mdr-info">
      <summary>Model files</summary>
      <dl>
        {#if info.installPath}<dt>Install path</dt><dd class="mdr-mono">{info.installPath}</dd>{/if}
        {#if info.licenseLabel}<dt>Licence</dt><dd>{info.licenseLabel}</dd>{/if}
        {#if info.sourceUrl}<dt>Source</dt><dd class="mdr-mono">{info.sourceUrl}</dd>{/if}
        {#if info.missingFiles?.length}
          <dt>Missing</dt>
          <dd class="mdr-mono">{#each info.missingFiles as file}<div>{file}</div>{/each}</dd>
        {/if}
      </dl>
    </details>
  {/if}
{/snippet}

<style>
  .mdr-stack {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    width: 100%;
  }

  .mdr-info summary {
    width: fit-content;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
  }

  .mdr-info summary:hover {
    color: var(--app-text);
  }

  .mdr-info dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 4px var(--s-3);
    margin: var(--s-2) 0 0;
    font-size: var(--text-sm);
  }

  .mdr-info dt {
    color: var(--app-text-subtle);
  }

  .mdr-info dd {
    margin: 0;
    color: var(--app-text-muted);
    overflow-wrap: anywhere;
  }

  .mdr-mono {
    font-family: var(--app-font-mono);
  }
</style>
