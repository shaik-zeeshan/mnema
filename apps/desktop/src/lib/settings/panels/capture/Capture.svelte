<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import Choice from "$lib/components/Choice.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import IconWave from "~icons/lucide/audio-waveform";
  import IconFeather from "~icons/lucide/feather";
  import IconOff from "~icons/lucide/ban";
  import IconLock from "~icons/lucide/lock";
  import IconInfo from "~icons/lucide/info";
  import IconExternal from "~icons/lucide/external-link";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import PermissionRows from "./PermissionRows.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const sysAudio = c.systemAudioAccess;

  // Recording-wide save-block reasons (controller-derived). Surfaced inline so a
  // blocked save shows the specific guidance instead of failing silently.
  const recValidationErrors = $derived(c.recValidationErrors);

  const loadRecordingSettings = () => rec.loadRecordingSettings();

  // Kit `mx-seg--mono` stops (backend caps segments at 5 min). A persisted value
  // off the ladder (e.g. 90 s from the old slider) shows as its own stop.
  const SEGMENT_STOPS = [60, 120, 300];
  const segmentLabel = (s: number) => (s % 60 === 0 ? `${s / 60}m` : `${s}s`);
  const segmentOptions = $derived(
    [...new Set([...SEGMENT_STOPS, rec.draftSegmentDuration])]
      .sort((a, b) => a - b)
      .map((s) => ({ value: String(s), label: segmentLabel(s) })),
  );

  // Idle timeout select (mockup stops); an off-list persisted value stays listed.
  const IDLE_STOPS = [30, 60, 120, 300];
  const idleLabel = (s: number) =>
    s % 60 === 0 ? `${s / 60} minute${s === 60 ? "" : "s"}` : `${s} seconds`;
  const idleOptions = $derived(
    [...new Set([...IDLE_STOPS, rec.draftIdleTimeoutSeconds])]
      .sort((a, b) => a - b)
      .map((s) => ({ value: String(s), label: idleLabel(s) })),
  );

</script>

<SettingGroup
  id="settings-section-capture"
  title="Capture"
  hint="What gets recorded and how often segments roll over."
>
  {#snippet actions()}
    <ReloadButton onclick={loadRecordingSettings} busy={rec.loadingRecSettings} label="Reload capture settings" />
  {/snippet}

  {#if rec.loadingRecSettings}
    <SettingRow label="Capture" description="Recording settings are loading." divider={false}>
      {#snippet control()}
        <p class="loading-text">Loading settings…</p>
      {/snippet}
    </SettingRow>
  {:else}
    <SettingRow label="Screen" description="Capture the display">
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureScreen} ariaLabel="Screen" />
      {/snippet}
    </SettingRow>

    <SettingRow label="Microphone" description="Capture audio from microphone">
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureMicrophone} ariaLabel="Microphone" />
      {/snippet}
    </SettingRow>

    <!-- System audio is its own capture family on a Core Audio process tap
         (ADR 0052): no screen dependency, so no `disabled` coupling to Screen. -->
    <SettingRow
      label="System Audio"
      description="Capture Mac system audio (macOS 15+) — meetings, videos, and app sound."
    >
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureSystemAudio} ariaLabel="System Audio" />
      {/snippet}
    </SettingRow>

    <!-- Inferred, never known: a denied tap records silence and says nothing, so
         this is the only signal the user gets (ADR 0052). -->
    {#if sysAudio.mayBeBlocked}
      <div class="cap-notice">
        <div class="mx-notice" data-tone="info" role="status">
          <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
          <b class="mx-notice__title">No system sound yet</b>
          <p class="mx-notice__text">
            System audio is on, but no recording has captured a sound yet. If you denied the macOS
            prompt, allow Mnema under Privacy &amp; Security → Screen &amp; System Audio Recording. If
            your Mac has simply been quiet, ignore this.
          </p>
          <div class="mx-notice__acts">
            <button class="mx-btn mx-btn--sm" type="button" onclick={() => sysAudio.openSettings()}>
              Open System Settings<IconExternal width="12" height="12" />
            </button>
            <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => sysAudio.dismiss()}>
              Dismiss
            </button>
          </div>
        </div>
        {#if sysAudio.error}
          <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />{sysAudio.error}</span>
        {/if}
      </div>
    {/if}

    <PermissionRows />

    <SettingRow
      label="Segment Duration"
      description="How long each recording segment is before a new one starts."
      divider={false}
    >
      {#snippet control()}
        <Segmented
          md
          mono
          value={String(rec.draftSegmentDuration)}
          onValueChange={(v) => (rec.draftSegmentDuration = Number(v))}
          options={segmentOptions}
          ariaLabel="Segment duration"
        />
      {/snippet}
    </SettingRow>
  {/if}
</SettingGroup>

{#if !rec.loadingRecSettings}
  <SettingGroup title="Inactivity" hint="Pause &amp; resume rules when you step away.">
    <SettingRow
      label="Pause capture when idle"
      description="Pause after the whole Mac has been idle (no mouse, keyboard or other input), and resume when activity is detected."
    >
      {#snippet control()}
        <div class="cap-ctl">
          {#if rec.draftPauseCaptureOnInactivity}
            <SelectMenu
              value={String(rec.draftIdleTimeoutSeconds)}
              onValueChange={(v) => (rec.draftIdleTimeoutSeconds = Number(v))}
              options={idleOptions}
              ariaLabel="Idle timeout"
            />
          {/if}
          <Switch bind:checked={rec.draftPauseCaptureOnInactivity} ariaLabel="Pause capture when idle" />
        </div>
      {/snippet}
    </SettingRow>

    {#if rec.draftPauseCaptureOnInactivity}
      <div class="cap-notice">
        <div class="mx-notice">
          <span class="mx-notice__icon"><IconWave width="15" height="15" /></span>
          <b class="mx-notice__title">Activity sources</b>
          <p class="mx-notice__text">
            Keyboard and mouse input, on-screen changes, and source-specific audio. The microphone
            counts speech when voice detection is on; system audio uses its level threshold below.
          </p>
        </div>
      </div>

      <SettingRow label="Microphone Voice Detection" description="Decides when the microphone counts as active." full>
          {#snippet control()}
            <div class="control-stack">
            <Choice
              bind:value={rec.draftMicrophoneVadAdapter}
              ariaLabel="Microphone voice detection"
              options={[
                {
                  value: "silero",
                  label: "Silero",
                  tag: "default",
                  icon: IconWave,
                  description: "Neural speech detector. Falls back to WebRTC when unavailable.",
                },
                {
                  value: "webrtc",
                  label: "WebRTC",
                  icon: IconFeather,
                  description: "Lightweight local speech detector.",
                },
                {
                  value: "off",
                  label: "Off",
                  icon: IconOff,
                  description: "Use the microphone peak level only.",
                },
              ]}
              disabled={!rec.draftCaptureMicrophone}
            />
            {#if !rec.draftCaptureMicrophone}
              <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />Microphone capture is disabled — voice detection has no effect until enabled.</span>
            {:else if rec.draftMicrophoneVadAdapter === "off"}
              <p class="group-hint">Microphone inactivity uses the legacy peak-level detector.</p>
            {:else}
              <p class="group-hint">Microphone inactivity uses local speech detection. Raw peak levels remain visible in debug output.</p>
            {/if}
            </div>
          {/snippet}
        </SettingRow>

        {#if rec.draftMicrophoneVadAdapter === "off"}
          <SettingRow label="Microphone Activity Sensitivity" full>
            {#snippet control()}
              <div class="control-stack">
              <Slider
                bind:value={rec.draftMicrophoneActivitySensitivity}
                min={0}
                max={100}
                step={1}
                label="Mic sensitivity"
                unit="%"
                disabled={!rec.draftCaptureMicrophone}
              />
              {#if !rec.draftCaptureMicrophone}
                <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />Microphone capture is disabled — this setting has no effect until enabled.</span>
              {:else}
                <p class="group-hint">
                  {#if rec.draftMicrophoneActivitySensitivity >= 80}
                    <strong>Very high</strong> — whispers and background noise keep capture active.
                  {:else if rec.draftMicrophoneActivitySensitivity >= 60}
                    <strong>High</strong> — quiet speech counts as activity.
                  {:else if rec.draftMicrophoneActivitySensitivity >= 40}
                    <strong>Medium</strong> — normal speech triggers activity. Recommended.
                  {:else if rec.draftMicrophoneActivitySensitivity >= 20}
                    <strong>Low</strong> — only louder audio keeps capture active.
                  {:else}
                    <strong>Very low</strong> — only very loud audio triggers activity.
                  {/if}
                </p>
              {/if}
              </div>
            {/snippet}
          </SettingRow>
        {/if}

        <SettingRow label="System Audio Activity Sensitivity" full>
          {#snippet control()}
            <div class="control-stack">
            <Slider
              bind:value={rec.draftSystemAudioActivitySensitivity}
              min={0}
              max={100}
              step={1}
              label="System audio sensitivity"
              unit="%"
              disabled={!rec.draftCaptureSystemAudio}
            />
            {#if !rec.draftCaptureSystemAudio}
              <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />System audio capture is disabled — this setting has no effect until enabled.</span>
            {:else}
              <p class="group-hint">
                {#if rec.draftSystemAudioActivitySensitivity >= 80}
                  <strong>Very high</strong> — quiet system sounds keep capture active.
                {:else if rec.draftSystemAudioActivitySensitivity >= 60}
                  <strong>High</strong> — moderate system audio counts as activity.
                {:else if rec.draftSystemAudioActivitySensitivity >= 40}
                  <strong>Medium</strong> — typical media playback triggers activity. Recommended.
                {:else if rec.draftSystemAudioActivitySensitivity >= 20}
                  <strong>Low</strong> — only louder system audio keeps capture active.
                {:else}
                  <strong>Very low</strong> — only very loud system audio triggers activity.
                {/if}
              </p>
            {/if}
            </div>
          {/snippet}
        </SettingRow>

        <div class="cap-notice">
          <span class="mx-inline"><IconInfo width="13" height="13" /><span>
            {#if !rec.draftCaptureMicrophone && !rec.draftCaptureSystemAudio}
              Neither microphone nor system audio capture is enabled — audio activity detection
              will not function. Enable at least one source in <strong>Capture Sources</strong> above.
            {:else if !rec.draftCaptureMicrophone}
              Microphone capture is disabled — only system audio is monitored for activity.
            {:else if !rec.draftCaptureSystemAudio}
              System audio capture is disabled — only microphone audio is monitored for activity.
            {:else}
              Both microphone and system audio are monitored independently for activity.
            {/if}
          </span></span>
        </div>
    {/if}
  </SettingGroup>

  {#if recValidationErrors.length > 0}
    <SettingGroup title="Unsaved changes blocked" hint="Resolve these before recording settings can save.">
      <SettingRow label="Validation" full divider={false}>
        {#snippet control()}
          <div class="inline-validation">
            {#each recValidationErrors as err}
              <p class="inline-validation__item">
                <span class="inline-validation__icon">⚠</span>
                {err}
              </p>
            {/each}
          </div>
        {/snippet}
      </SettingRow>
    </SettingGroup>
  {/if}
{/if}

<style>
  /* A full-width notice between rows, on the row's own gutters. */
  .cap-notice {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: 12px 20px;
  }

  .cap-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }

  /* SettingRow's full-mode control slot is a flex row; stack the wide control
     above its contextual hint instead of laying them out side by side. */
  .control-stack {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    min-width: 0;
  }

  .control-stack :global(.group-hint) {
    margin: 0;
    text-align: left;
  }

</style>
