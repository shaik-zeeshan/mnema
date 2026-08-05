<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("video");

  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import CaptureRateControl from "$lib/components/CaptureRateControl.svelte";
  import { captureRateShortLabel } from "$lib/components/capture-rate";
  import ScreenResolutionControl from "$lib/components/ScreenResolutionControl.svelte";
  import VideoBitrateControl from "$lib/components/VideoBitrateControl.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import {
    captureRateConsequence,
    retentionConsequence,
    retentionFootprint,
  } from "$lib/settings/state/system-facts";
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

  // G8: the slider's consequence in real bytes, projected from this machine's
  // measured capture days. Null (no complete day measured yet, or an unreadable
  // volume) renders no line at all rather than a made-up figure.
  void systemFacts.ensureLoaded();
  const captureRateHint = $derived(
    captureRateConsequence(systemFacts.value, rec.draftFrameRate),
  );

  // ── Retention, shown here rather than under Data (a STATED deviation from
  // the IA, see 07-components.html): the keep-window is the second half of the
  // frame-rate decision, and the two only make sense on one pane — the slider
  // sets the daily cost, the ladder sets how many days of it survive.
  const retentionCleanupSummary = $derived(c.retentionCleanupSummary);
  const retentionCleanupRunning = $derived(c.retentionCleanupRunning);
  const retentionCleanupError = $derived(c.retentionCleanupError);

  const retentionHint = $derived(
    retentionConsequence(systemFacts.value, rec.draftRetentionPolicy),
  );
  // The footprint bar's two halves — both measured on this machine (the rate
  // over your last complete capture days × the window, against the volume's
  // real free bytes). Null when either is unknown, or when "Forever" leaves
  // nothing to draw a ceiling against; then only the prose hint renders.
  const retentionBar = $derived(
    retentionFootprint(systemFacts.value, rec.draftRetentionPolicy),
  );

  // A cleanup changes what is on disk, so the measured rate is re-read after it.
  const runRetentionCleanupNow = async () => {
    await c.runRetentionCleanupNow();
    await systemFacts.refresh();
  };
</script>

<SettingGroup id="settings-section-video" title="Video Output">
  <SettingRow
    label="Screen Capture Rate"
    description="How often a snapshot of your screen is captured. More frequent snapshots produce larger files."
    full
  >
    {#snippet control()}
      <div class="control-stack">
        <CaptureRateControl bind:value={rec.draftFrameRate} />
        {#if captureRateHint}
          <!-- The instrument's whole point: the stored value is fps, the value
               you care about is GB a day. Measured on this Mac — no figure at
               all until a complete capture day exists (G8). -->
          <p class="inst-cost">{captureRateHint}</p>
        {/if}
      </div>
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

<!-- Custom input 2 of 5 — the retention ladder. Same tile grid, same rows; the
     only thing it adds is the footprint bar, which puts the keep-window and the
     free space it has to fit into on ONE axis. -->
<SettingGroup title="Retention" hint="What survives, and what it costs.">
  <SettingRow
    label="Retention"
    description="Captures older than the chosen window are deleted automatically. Context and subjects Mnema has already distilled are never touched by it."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="retention-control">
        <RetentionPicker bind:value={rec.draftRetentionPolicy} />

        {#if retentionBar}
          <div>
            <div class="inst-bar" aria-hidden="true">
              <i class="is-kept" style:width="{retentionBar.keptPercent}%"></i>
              <i class="is-free" style:width="{100 - retentionBar.keptPercent}%"></i>
            </div>
            <div class="inst-key">
              <span><i class="is-kept"></i>{retentionBar.keptLabel}</span>
              <span><i class="is-free"></i>{retentionBar.freeLabel}</span>
            </div>
          </div>
        {/if}

        {#if retentionHint}
          <p class="inst-cost">{retentionHint}</p>
        {/if}

        <div class="row-actions">
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            onclick={runRetentionCleanupNow}
            disabled={retentionCleanupRunning}
            aria-busy={retentionCleanupRunning}
          >
            {#if retentionCleanupRunning}<ButtonSpinner />Running…{:else}Run cleanup now{/if}
          </button>
        </div>
        {#if retentionCleanupSummary}
          <div class="cleanup-result" aria-live="polite">
            <strong>Latest cleanup</strong>
            <p>
              {retentionCleanupSummary.deletedCaptureSegments} segment(s), {retentionCleanupSummary.deletedFrames}
              frame(s), {retentionCleanupSummary.deletedAudioSegments} audio segment(s).
            </p>
          </div>
        {/if}
        {#if retentionCleanupError}
          <p class="error-text">{retentionCleanupError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* The retention control stacks its ladder, the footprint bar, the consequence
     line, and the run-now action. */
  .retention-control {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  .retention-control .row-actions {
    justify-content: flex-start;
  }

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
