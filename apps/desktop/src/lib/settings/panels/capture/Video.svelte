<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import CaptureRateControl from "$lib/components/CaptureRateControl.svelte";
  import { captureRateShortLabel } from "$lib/components/capture-rate";
  import ScreenResolutionControl from "$lib/components/ScreenResolutionControl.svelte";
  import VideoBitrateControl from "$lib/components/VideoBitrateControl.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import IconInfo from "~icons/lucide/info";
  import IconLoader from "~icons/lucide/loader-circle";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";

  const c = getSettingsController();
  const rec = c.rec;

  const nativeCaptureUnsupported = $derived(c.nativeCaptureUnsupported);
  const onlyOriginalResolutionSupported = $derived(c.onlyOriginalResolutionSupported);
  const resolutionSupportPending = $derived(c.resolutionSupportPending);
  const captureSupportFailed = $derived(c.captureSupportFailed);
  const nonOriginalResolutionSupported = $derived(c.nonOriginalResolutionSupported);
  const nonOriginalResolutionDisabled = $derived(c.nonOriginalResolutionDisabled);
  const customResolutionErrors = $derived(c.customResolutionErrors);
  const customBitrateErrors = $derived(c.customBitrateErrors);

</script>

<SettingGroup id="settings-section-video" title="Video Output">
  <SettingRow
    label="Screen Capture Rate"
    description="How often a snapshot of your screen is captured. More frequent snapshots produce larger files."
    full
  >
    {#snippet control()}
      <CaptureRateControl bind:value={rec.draftFrameRate} />
    {/snippet}
  </SettingRow>

  <SettingRow label="Screen Resolution" full divider={false}>
    {#snippet control()}
      <div class="control-stack">
        {#if nativeCaptureUnsupported}
          <div class="resolution-unsupported-notice">
            <span class="resolution-unsupported-notice__icon" aria-hidden="true"><IconInfo /></span>
            <span class="resolution-unsupported-notice__text">
              Native screen capture is unsupported on this system. Resolution settings are saved,
              but only apply when native screen capture is available.
            </span>
          </div>
        {:else if onlyOriginalResolutionSupported}
          <div class="resolution-locked-notice">
            <span class="resolution-locked-notice__icon" aria-hidden="true"><IconInfo /></span>
            <span class="resolution-locked-notice__text">
              Scaled and custom resolutions require macOS 15 or later (ScreenCaptureKit).
              Only <strong>Original</strong> resolution is available on this system.
            </span>
          </div>
        {:else if resolutionSupportPending}
          <div class="resolution-pending-notice">
            <span class="resolution-pending-notice__icon resolution-pending-notice__icon--spin" aria-hidden="true"><IconLoader /></span>
            <span class="resolution-pending-notice__text">
              Checking capture support… Scaled and custom resolutions are disabled until support is confirmed.
            </span>
          </div>
        {:else if captureSupportFailed}
          <div class="resolution-warn-notice">
            <span class="resolution-warn-notice__icon" aria-hidden="true"><IconAlert /></span>
            <span class="resolution-warn-notice__text">
              Could not determine capture support for this system. You can still edit and save —
              the backend will validate the chosen resolution.
            </span>
          </div>
        {:else if nonOriginalResolutionSupported}
          <div class="resolution-supported-notice">
            <span class="resolution-supported-notice__icon" aria-hidden="true"><IconCheck /></span>
            <span class="resolution-supported-notice__text">
              Native capture supports scaled and custom output resolutions.
            </span>
          </div>
        {/if}

        <!-- Original, the scaled presets, and Custom collapse into one
             segmented control. The shared ScreenResolutionControl owns the
             segmented + width/height inputs (and their accessible labels);
             this panel keeps the support notices and the mode description. -->
        <ScreenResolutionControl
          bind:mode={rec.draftResolutionMode}
          bind:preset={rec.draftResolutionPreset}
          bind:widthRaw={rec.customWidthRaw}
          bind:heightRaw={rec.customHeightRaw}
          disabledValues={nonOriginalResolutionDisabled ? ["1080p", "720p", "540p", "custom"] : []}
          customErrors={customResolutionErrors}
        />

        <p class="group-hint">
          {#if rec.draftResolutionMode === "original"}
            Output files will match the captured display's native pixel dimensions.
          {:else if rec.draftResolutionMode === "preset"}
            Output will be scaled to the selected preset. Aspect ratio is preserved.
          {:else}
            Output will be scaled to the exact dimensions you specify.
          {/if}
        </p>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup
  title="Video Bitrate"
  hint="Bitrate controls how much data is encoded per second of video. Higher bitrate = sharper image but larger files and higher CPU/GPU load. Applied on macOS 15+ via ScreenCaptureKit; older systems fall back to the system-default bitrate."
>
  <SettingRow label="Bitrate" full divider={false}>
    {#snippet control()}
      <div class="control-stack">
        <!-- Mode selector: presets (low/medium/high) + custom as one segmented
             control. The shared VideoBitrateControl owns the segmented + the
             Mbps stepper (and its accessible label); this panel keeps the
             richer preset/custom descriptions and the compat notice. customMbps
             is intentionally omitted so the component's terse custom line stays
             hidden in favour of the detailed hint below. -->
        <VideoBitrateControl
          bind:mode={rec.draftBitrateMode}
          bind:preset={rec.draftBitratePreset}
          bind:customMbpsRaw={rec.draftCustomMbpsRaw}
          customErrors={customBitrateErrors}
        />

        {#if rec.draftBitrateMode === "preset"}
          <p class="group-hint bitrate-preset-hint">
            {#if rec.draftBitratePreset === "low"}
              <strong>Low</strong> — ~3 Mbps. Good for long sessions, minimal storage. Best for
              low-motion content or when disk space is limited.
            {:else if rec.draftBitratePreset === "medium"}
              <strong>Medium</strong> — ~8 Mbps. Recommended default. Balanced quality and file
              size for most screen recordings.
            {:else}
              <strong>High</strong> — ~20 Mbps. Crisp detail and smooth motion at the cost of
              larger files. Ideal for high-motion content or final delivery.
            {/if}
            {#if rec.draftFrameRate && rec.draftResolutionMode !== "custom"}
              {' '}At {captureRateShortLabel(rec.draftFrameRate)}{rec.draftResolutionMode === "preset" ? ` / ${rec.draftResolutionPreset}` : rec.draftResolutionMode === "original" ? " / original resolution" : ""}.
            {/if}
          </p>
        {/if}

        {#if rec.draftBitrateMode === "custom" && customBitrateErrors.length === 0 && rec.draftCustomMbps !== null}
          <p class="group-hint">
            Custom bitrate: <strong>{rec.draftCustomMbps} Mbps</strong>.
            {#if rec.draftCustomMbps < 3}
              Low quality — may show compression artefacts on fast-moving content.
            {:else if rec.draftCustomMbps <= 12}
              Moderate quality — good for most recordings.
            {:else if rec.draftCustomMbps <= 25}
              High quality — suitable for detail-sensitive content.
            {:else}
              Very high bitrate — expect large output files.
            {/if}
            {#if rec.draftFrameRate && rec.draftResolutionMode !== "custom"}
              At {captureRateShortLabel(rec.draftFrameRate)}{rec.draftResolutionMode === "preset" ? ` / ${rec.draftResolutionPreset}` : rec.draftResolutionMode === "original" ? " / original resolution" : ""}.
            {/if}
          </p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* SettingRow's full-mode control slot is a flex row; stack these wide custom
     blocks (notices, radio group, preset/custom inputs, hints) vertically. */
  .control-stack {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }

  .control-stack :global(.group-hint) {
    margin: 0;
  }

  /* The notice `__icon` spans (styled globally in settings-blocks.css for a text
     glyph) now hold a Lucide SVG — size + stroke it to inherit the span's color.
     The svg is rendered by a child icon component, so it needs a `:global`
     descendant rule (a class on the component wouldn't carry this scope hash). */
  .resolution-unsupported-notice__icon :global(svg),
  .resolution-locked-notice__icon :global(svg),
  .resolution-pending-notice__icon :global(svg),
  .resolution-warn-notice__icon :global(svg),
  .resolution-supported-notice__icon :global(svg) {
    display: block;
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* The "checking support…" state spins its loader. Rotate the wrapper span, not
     the svg (WKWebView won't reliably rotate an svg around its own center). */
  .resolution-pending-notice__icon--spin {
    display: inline-flex;
    animation: settings-icon-spin 0.7s linear infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .resolution-pending-notice__icon--spin {
      animation: none;
    }
  }
</style>
