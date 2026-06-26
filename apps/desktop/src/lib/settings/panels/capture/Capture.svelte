<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  // Recording-wide save-block reasons (controller-derived). Surfaced inline so a
  // blocked save shows the specific guidance instead of failing silently.
  const recValidationErrors = $derived(c.recValidationErrors);

  const loadRecordingSettings = () => rec.loadRecordingSettings();

  // Near-the-control autosave cue, per owning recording domain: the source
  // toggles save through "capture_sources", segment duration through
  // "capture_timing", and the idle/voice-detection controls through "inactivity".
  const sourcesSaving = $derived(c.rec.savingRecDomains.capture_sources);
  const sourcesSaved = $derived(c.recSavedDomain === "capture_sources");
  const timingSaving = $derived(c.rec.savingRecDomains.capture_timing);
  const timingSaved = $derived(c.recSavedDomain === "capture_timing");
  const inactivitySaving = $derived(c.rec.savingRecDomains.inactivity);
  const inactivitySaved = $derived(c.recSavedDomain === "inactivity");
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
    <SettingRow label="Screen" description="Capture the display" saving={sourcesSaving} saved={sourcesSaved}>
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureScreen} ariaLabel="Screen" />
      {/snippet}
    </SettingRow>

    <SettingRow label="Microphone" description="Capture audio from microphone" saving={sourcesSaving} saved={sourcesSaved}>
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureMicrophone} ariaLabel="Microphone" />
      {/snippet}
    </SettingRow>

    <SettingRow
      label="System Audio"
      description={rec.draftCaptureScreen
        ? "Capture Mac system audio (macOS 15+)"
        : "Capture Mac system audio (macOS 15+). System Audio is unavailable — enable Screen first."}
      disabled={!rec.draftCaptureScreen}
      saving={sourcesSaving}
      saved={sourcesSaved}
    >
      {#snippet control()}
        <Switch
          bind:checked={rec.draftCaptureSystemAudio}
          disabled={!rec.draftCaptureScreen}
          ariaLabel="System Audio"
        />
      {/snippet}
    </SettingRow>

    <SettingRow
      label="Segment Duration"
      description="How long each recording segment is before a new one starts."
      full
      divider={false}
      saving={timingSaving}
      saved={timingSaved}
    >
      {#snippet control()}
        <Slider
          bind:value={rec.draftSegmentDuration}
          min={10}
          max={300}
          step={10}
          label="Duration"
          unit="s"
          formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60}s` : `${v}s`}
        />
      {/snippet}
    </SettingRow>
  {/if}
</SettingGroup>

{#if !rec.loadingRecSettings}
  <SettingGroup title="Inactivity" hint="Pause &amp; resume rules when you step away.">
    <SettingRow
      label="Pause capture when idle"
      description="Automatically pause recording after the system has been idle, and resume when system activity is detected"
      divider={rec.draftPauseCaptureOnInactivity}
      saving={inactivitySaving}
      saved={inactivitySaved}
    >
      {#snippet control()}
        <Switch bind:checked={rec.draftPauseCaptureOnInactivity} ariaLabel="Pause capture when idle" />
      {/snippet}
    </SettingRow>

    {#if rec.draftPauseCaptureOnInactivity}
      <SettingRow label="Idle timeout" full saving={inactivitySaving} saved={inactivitySaved}>
        {#snippet control()}
          <div class="control-stack">
          <Slider
            bind:value={rec.draftIdleTimeoutSeconds}
            min={5}
            max={300}
            step={5}
            label="Idle timeout"
            unit="s"
            formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m${v%60 > 0 ? ` ${v%60}s` : ""}` : `${v}s`}
          />
          <p class="group-hint">
            Capture pauses after <strong>{rec.draftIdleTimeoutSeconds}s</strong> of system-wide inactivity (no mouse, keyboard,
            or other input anywhere on the Mac). It resumes automatically when system activity is detected again.
          </p>
          </div>
        {/snippet}
      </SettingRow>

      <SettingRow label="Activity sources" full>
        {#snippet control()}
          <div class="audio-activity-notice">
            <span class="audio-activity-notice__icon">♪</span>
            <span class="audio-activity-notice__text">
              Activity is detected from keyboard/mouse input, on-screen changes, <em>and</em>
              source-specific audio. Microphone activity is speech-first when voice detection is
              enabled, while system audio uses the configured level threshold.
            </span>
          </div>
        {/snippet}
      </SettingRow>

      <SettingRow label="Microphone Voice Detection" full saving={inactivitySaving} saved={inactivitySaved}>
          {#snippet control()}
            <div class="control-stack">
            <RadioGroup
              bind:value={rec.draftMicrophoneVadAdapter}
              options={[
                {
                  value: "silero",
                  label: "Silero",
                  description: "Default speech detector. Falls back to WebRTC when unavailable.",
                },
                {
                  value: "webrtc",
                  label: "WebRTC",
                  description: "Lightweight local speech detector.",
                },
                {
                  value: "off",
                  label: "Off",
                  description: "Use legacy microphone peak-level activity.",
                },
              ]}
              disabled={!rec.draftCaptureMicrophone}
            />
            {#if !rec.draftCaptureMicrophone}
              <p class="group-hint group-hint--warn">Microphone capture is disabled — voice detection has no effect until enabled.</p>
            {:else if rec.draftMicrophoneVadAdapter === "off"}
              <p class="group-hint">Microphone inactivity uses the legacy peak-level detector.</p>
            {:else}
              <p class="group-hint">Microphone inactivity uses local speech detection. Raw peak levels remain visible in debug output.</p>
            {/if}
            </div>
          {/snippet}
        </SettingRow>

        {#if rec.draftMicrophoneVadAdapter === "off"}
          <SettingRow label="Microphone Activity Sensitivity" full saving={inactivitySaving} saved={inactivitySaved}>
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
                <p class="group-hint group-hint--warn">Microphone capture is disabled — this setting has no effect until enabled.</p>
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

        <SettingRow label="System Audio Activity Sensitivity" full saving={inactivitySaving} saved={inactivitySaved}>
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
              <p class="group-hint group-hint--warn">System audio capture is disabled — this setting has no effect until enabled.</p>
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

        <SettingRow label="Audio Activity Monitoring" full divider={false}>
          {#snippet control()}
            <div class="audio-activity-notice">
              <span class="audio-activity-notice__icon">♪</span>
              <span class="audio-activity-notice__text">
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
              </span>
            </div>
          {/snippet}
        </SettingRow>
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
