<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

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
    label="Screen Frame Rate"
    description="Higher frame rates produce larger files."
    full
  >
    {#snippet control()}
      <Slider
        bind:value={rec.draftFrameRate}
        min={1}
        max={120}
        step={1}
        label="Frame rate"
        unit=" fps"
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Screen Resolution" full divider={false}>
    {#snippet control()}
      <div class="control-stack">
        {#if nativeCaptureUnsupported}
          <div class="resolution-unsupported-notice">
            <span class="resolution-unsupported-notice__icon">ℹ</span>
            <span class="resolution-unsupported-notice__text">
              Native screen capture is unsupported on this system. Resolution settings are saved,
              but only apply when native screen capture is available.
            </span>
          </div>
        {:else if onlyOriginalResolutionSupported}
          <div class="resolution-locked-notice">
            <span class="resolution-locked-notice__icon">ℹ</span>
            <span class="resolution-locked-notice__text">
              Preset and custom resolutions require macOS 15 or later (ScreenCaptureKit).
              Only <strong>Original</strong> resolution is available on this system.
            </span>
          </div>
        {:else if resolutionSupportPending}
          <div class="resolution-pending-notice">
            <span class="resolution-pending-notice__icon">⏳</span>
            <span class="resolution-pending-notice__text">
              Checking capture support… Preset and Custom are disabled until support is confirmed.
            </span>
          </div>
        {:else if captureSupportFailed}
          <div class="resolution-warn-notice">
            <span class="resolution-warn-notice__icon">⚠</span>
            <span class="resolution-warn-notice__text">
              Could not determine capture support for this system. You can still edit and save —
              the backend will validate the chosen resolution.
            </span>
          </div>
        {:else if nonOriginalResolutionSupported}
          <div class="resolution-supported-notice">
            <span class="resolution-supported-notice__icon">✓</span>
            <span class="resolution-supported-notice__text">
              Native capture supports Preset and Custom output resolutions.
            </span>
          </div>
        {/if}

        <RadioGroup
          bind:value={rec.draftResolutionMode}
          disabledValues={nonOriginalResolutionDisabled ? ["preset", "custom"] : []}
          options={[
            { value: "original", label: "Original", description: "Capture at the display's native resolution" },
            { value: "preset", label: "Preset", description: "Select a standard output resolution" },
            { value: "custom", label: "Custom", description: "Enter exact width and height in pixels" },
          ]}
        />

        {#if rec.draftResolutionMode === "preset"}
          <div class="resolution-preset-grid">
            {#each (["1080p", "720p", "540p"] as const) as preset}
              {@const presetMeta = { "1080p": { w: 1920, h: 1080 }, "720p": { w: 1280, h: 720 }, "540p": { w: 960, h: 540 } }[preset]}
              <button
                class="preset-chip"
                class:preset-chip--active={rec.draftResolutionPreset === preset}
                onclick={() => { rec.draftResolutionPreset = preset; }}
                type="button"
              >
                <span class="preset-chip__label">{preset}</span>
                <span class="preset-chip__dim">{presetMeta.w}×{presetMeta.h}</span>
              </button>
            {/each}
          </div>
        {/if}

        {#if rec.draftResolutionMode === "custom"}
          <div class="custom-resolution-inputs">
            <div class="custom-res-field">
              <label class="custom-res-label" for="res-width">Width (px)</label>
              <input
                id="res-width"
                type="text"
                inputmode="numeric"
                class="text-input custom-res-input"
                class:text-input--empty={rec.customWidthRaw && rec.draftCustomWidth === null}
                bind:value={rec.customWidthRaw}
                placeholder="e.g. 1920"
                autocomplete="off"
              />
            </div>
            <span class="custom-res-sep" aria-hidden="true">×</span>
            <div class="custom-res-field">
              <label class="custom-res-label" for="res-height">Height (px)</label>
              <input
                id="res-height"
                type="text"
                inputmode="numeric"
                class="text-input custom-res-input"
                class:text-input--empty={rec.customHeightRaw && rec.draftCustomHeight === null}
                bind:value={rec.customHeightRaw}
                placeholder="e.g. 1080"
                autocomplete="off"
              />
            </div>
          </div>

          {#if customResolutionErrors.length > 0}
            <div class="inline-validation">
              {#each customResolutionErrors as err}
                <p class="inline-validation__item">
                  <span class="inline-validation__icon">⚠</span>
                  {err}
                </p>
              {/each}
            </div>
          {/if}
        {/if}

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
        <!-- Mode selector (preset chips + custom) -->
        <div class="bitrate-mode-chips">
          {#each (["low", "medium", "high"] as const) as bp}
            {@const meta = { low: { mbps: "~3", hint: "Lower quality, smallest file" }, medium: { mbps: "~8", hint: "Balanced quality and size" }, high: { mbps: "~20", hint: "High quality, larger file" } }[bp]}
            <button
              type="button"
              class="bitrate-chip"
              class:bitrate-chip--active={rec.draftBitrateMode === "preset" && rec.draftBitratePreset === bp}
              onclick={() => { rec.draftBitrateMode = "preset"; rec.draftBitratePreset = bp; }}
            >
              <span class="bitrate-chip__label">{bp}</span>
              <span class="bitrate-chip__mbps">{meta.mbps} Mbps</span>
            </button>
          {/each}
          <button
            type="button"
            class="bitrate-chip"
            class:bitrate-chip--active={rec.draftBitrateMode === "custom"}
            onclick={() => { rec.draftBitrateMode = "custom"; }}
          >
            <span class="bitrate-chip__label">Custom</span>
            <span class="bitrate-chip__mbps">1–40 Mbps (integer)</span>
          </button>
        </div>

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
              {' '}At {rec.draftFrameRate} fps{rec.draftResolutionMode === "preset" ? ` / ${rec.draftResolutionPreset}` : rec.draftResolutionMode === "original" ? " / original resolution" : ""}.
            {/if}
          </p>
        {/if}

        {#if rec.draftBitrateMode === "custom"}
          <div class="custom-bitrate-row">
            <div class="custom-res-field">
              <label class="custom-res-label" for="bitrate-mbps">Bitrate (Mbps, whole number)</label>
              <div class="custom-bitrate-input-wrap">
                <input
                  id="bitrate-mbps"
                  type="text"
                  inputmode="numeric"
                  class="text-input custom-bitrate-input"
                  class:text-input--empty={rec.draftCustomMbpsRaw && rec.draftCustomMbps === null}
                  bind:value={rec.draftCustomMbpsRaw}
                  placeholder="e.g. 12"
                  autocomplete="off"
                />
                <span class="custom-bitrate-unit">Mbps</span>
              </div>
            </div>
          </div>

          {#if customBitrateErrors.length > 0}
            <div class="inline-validation">
              {#each customBitrateErrors as err}
                <p class="inline-validation__item">
                  <span class="inline-validation__icon">⚠</span>
                  {err}
                </p>
              {/each}
            </div>
          {:else if rec.draftCustomMbps !== null}
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
                At {rec.draftFrameRate} fps{rec.draftResolutionMode === "preset" ? ` / ${rec.draftResolutionPreset}` : rec.draftResolutionMode === "original" ? " / original resolution" : ""}.
              {/if}
            </p>
          {/if}
        {/if}

        <div class="bitrate-compat-notice">
          <span class="bitrate-compat-notice__icon">ℹ</span>
          <span class="bitrate-compat-notice__text">
            Bitrate is applied only on macOS 15+ (ScreenCaptureKit path).
            On older macOS the system default bitrate is used regardless of this setting.
          </span>
        </div>
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
</style>
