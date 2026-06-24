<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import type {
    OcrTesseractPageSegmentationMode,
    OcrTesseractPreprocessMode,
  } from "$lib/types";
  import Segmented from "$lib/components/Segmented.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import AdvancedReveal from "./AdvancedReveal.svelte";
  import { formatBytes } from "./onboarding-mapping";

  let { controller }: { controller: OnboardingController } = $props();

  // Mirrors Ocr.svelte: the Combobox selects the model; the card below reflects
  // the resolved `selectedOcrModel` and drives the in-app download. The status
  // pill reuses the mockup's `.pill` family (granted/pending/denied).
  const model = $derived(controller.selectedOcrModel);

  // Provider picker uses RadioGroup-with-descriptions (matching Settings →
  // Ocr.svelte): each provider has a meaningful description, so per the control
  // convention this is a RadioGroup, not a Segmented control. Descriptions come
  // from the live model status (mirrors controller-processing's option deriving);
  // fall back to a static loading description before the status arrives.
  const ocrProviderOptions = $derived(
    (controller.ocrModelStatus?.providers ?? []).map((provider) => ({
      value: provider.provider,
      label: provider.displayName,
      description: provider.models.some((m) => m.available)
        ? "At least one model is available"
        : "No available model detected",
    })),
  );

  const pillClass = $derived.by(() => {
    if (!model) return "pending";
    if (model.available) return "granted";
    if (controller.selectedOcrDownloadRunning) return "pending";
    return "denied";
  });

</script>

<div class="group">
  <div class="group-title">Provider</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">OCR provider</div>
      <div class="desc">
        Apple Vision is on-device with no download; Tesseract supports many languages.
      </div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <RadioGroup
        value={controller.draftOcrProvider}
        onValueChange={(v) => controller.chooseOcrProvider(v)}
        label="OCR provider"
        options={ocrProviderOptions.length > 0
          ? ocrProviderOptions
          : [
              { value: "apple_vision", label: "Apple Vision", description: "Model status is loading" },
              { value: "tesseract", label: "Tesseract", description: "Model status is loading" },
            ]}
      />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Model</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Model</div>
      <div class="desc">Pick the OCR model for the selected provider.</div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <Combobox
        value={controller.draftOcrModelId ?? "__os_managed__"}
        onValueChange={(v) => controller.chooseOcrModel(v)}
        searchPlaceholder="Search models…"
        options={controller.ocrModelOptions.length > 0
          ? controller.ocrModelOptions
          : [{ value: controller.draftOcrModelId ?? "__os_managed__", label: "Loading model options" }]}
      />
    </div>
  </div>

  {#if controller.ocrModelError}
    <div class="note">
      <div>Failed to load OCR model status: {controller.ocrModelError}</div>
      <div class="dl-meta">
        <span>This is a fetch error, not a missing model — retry to recheck.</span>
        <button
          type="button"
          class="btn sm"
          disabled={controller.loadingOcrModelStatus}
          onclick={() => controller.loadOcrModelStatus()}
        >
          {controller.loadingOcrModelStatus ? "Retrying…" : "Retry"}
        </button>
      </div>
    </div>
  {:else if model}
    <div class="model-card">
      <div class="model-top">
        <div class="model-id">
          {model.displayName}
          <div class="meta">{controller.ocrStatusLabel(model)}{model.description ? ` · ${model.description}` : ""}</div>
        </div>
        <span class="pill {pillClass}">
          <span class="d"></span>{controller.ocrStatusLabel(model)}
        </span>
      </div>

      {#if model.management === "app_managed"}
        {#if controller.selectedOcrDownloadRunning}
          <div class="dl">
            <div class="dl-track">
              <div class="dl-fill" style={`width: ${controller.selectedOcrDownloadPercent ?? 8}%`}></div>
            </div>
            <div class="dl-meta">
              <span>
                <b>{controller.selectedOcrDownloadPercent ?? 0}%</b>
                · {controller.selectedOcrDownloadProgress?.status ?? "downloading"}
              </span>
              <button
                type="button"
                class="btn sm danger"
                disabled={controller.cancellingOcrDownload}
                onclick={() => controller.cancelSelectedOcrModelDownload()}
              >
                {controller.cancellingOcrDownload ? "Cancelling…" : "Cancel"}
              </button>
            </div>
          </div>
        {:else if model.download}
          <div class="dl-meta">
            <span>{model.available ? "Downloaded" : "Not downloaded"} · {formatBytes(model.download.byteSize)}</span>
            <button
              type="button"
              class="btn sm accent"
              disabled={controller.startingOcrDownload || model.available}
              onclick={() => controller.startSelectedOcrModelDownload()}
            >
              {controller.startingOcrDownload ? "Starting…" : "Download"}
            </button>
          </div>
        {:else if !model.available}
          <div class="note">
            This app-managed OCR bundle is missing, and the current manifest does not ship a
            downloadable artifact yet.
          </div>
        {/if}
        {#if controller.ocrDownloadError}
          <div class="note">Download failed: {controller.ocrDownloadError}</div>
        {/if}
      {:else}
        <div class="note muted">This provider is managed by macOS. There is no app-managed model download.</div>
      {/if}
    </div>
  {:else if controller.loadingOcrModelStatus}
    <div class="note muted">Checking installed OCR models…</div>
  {:else}
    <div class="note">No OCR model status is available for the selected provider.</div>
  {/if}
</div>

{#if controller.draftOcrProvider === "tesseract"}
  <div class="group">
    <div class="ctl stack-field">
      <div class="ctl-label">
        <div class="name">Language</div>
        <div class="desc">Tesseract only.</div>
      </div>
      <div class="ctl-field">
        <SelectMenu
          value={controller.draftOcrLanguage || "eng"}
          onValueChange={(v) => (controller.draftOcrLanguage = v)}
          options={[
            { value: "eng", label: "English (eng)" },
            { value: "fra", label: "French (fra)" },
            { value: "deu", label: "German (deu)" },
            { value: "spa", label: "Spanish (spa)" },
            { value: "ita", label: "Italian (ita)" },
            { value: "por", label: "Portuguese (por)" },
            { value: "nld", label: "Dutch (nld)" },
            { value: "jpn", label: "Japanese (jpn)" },
            { value: "chi_sim", label: "Chinese, Simplified (chi_sim)" },
          ]}
        />
      </div>
    </div>
  </div>
{/if}

<div class="group">
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Recognition mode</div>
      <div class="desc">Fast trades accuracy for speed; Accurate spends more time per frame.</div>
    </div>
    <div class="ctl-field">
      <Segmented
        bind:value={controller.draftOcrRecognitionMode}
        ariaLabel="Recognition mode"
        options={[
          { value: "fast", label: "Fast" },
          { value: "accurate", label: "Accurate" },
        ]}
      />
    </div>
  </div>
</div>

{#if controller.draftOcrProvider === "tesseract"}
  <div class="group">
    <AdvancedReveal>
      <div class="ctl stack-field">
        <div class="ctl-label">
          <div class="name">Page segmentation</div>
          <div class="desc">
            Auto for mixed layouts, Single block for paragraphs, Single line/word for short
            labels, Sparse text for scattered screenshots.
          </div>
        </div>
        <div class="ctl-field">
          <SelectMenu
            value={controller.draftOcrTesseractPageSegmentationMode}
            onValueChange={(v) => {
              controller.draftOcrTesseractPageSegmentationMode = v as OcrTesseractPageSegmentationMode;
            }}
            options={[
              { value: "auto", label: "Auto" },
              { value: "single_block", label: "Single block" },
              { value: "single_line", label: "Single line" },
              { value: "single_word", label: "Single word" },
              { value: "sparse_text", label: "Sparse text" },
            ]}
          />
        </div>
      </div>

      <div class="ctl stack-field">
        <div class="ctl-label">
          <div class="name">Image preprocessing</div>
          <div class="desc">Grayscale suits clean UI text; Thresholded helps muddy edges or weak contrast.</div>
        </div>
        <div class="ctl-field">
          <SelectMenu
            value={controller.draftOcrTesseractPreprocessMode}
            onValueChange={(v) => {
              controller.draftOcrTesseractPreprocessMode = v as OcrTesseractPreprocessMode;
            }}
            options={[
              { value: "grayscale", label: "Grayscale" },
              { value: "thresholded", label: "Thresholded" },
            ]}
          />
        </div>
      </div>

      <div class="ctl stack-field">
        <div class="ctl-label">
          <div class="name">Upscale before OCR</div>
          <div class="desc">Tesseract works best near 300 DPI; for tiny screenshots try 2× first.</div>
        </div>
        <div class="ctl-field">
          <SelectMenu
            value={String(controller.draftOcrTesseractUpscaleFactor)}
            onValueChange={(v) => {
              controller.draftOcrTesseractUpscaleFactor = parseInt(v, 10) || 1;
            }}
            options={[
              { value: "1", label: "1×" },
              { value: "2", label: "2×" },
              { value: "3", label: "3×" },
              { value: "4", label: "4×" },
            ]}
          />
        </div>
      </div>

      <div class="ctl">
        <div class="ctl-label">
          <div class="name">Language correction</div>
          <div class="desc">Spend extra work correcting recognized text using language models.</div>
        </div>
        <div class="ctl-field">
          <Switch bind:checked={controller.draftOcrLanguageCorrection} />
        </div>
      </div>
    </AdvancedReveal>
  </div>
{/if}
