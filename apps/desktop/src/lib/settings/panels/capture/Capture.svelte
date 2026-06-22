<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  const loadRecordingSettings = () => rec.loadRecordingSettings();
</script>

<SettingGroup
  id="settings-section-capture"
  title="Capture"
  hint="What gets recorded and how often segments roll over."
>
  {#snippet actions()}
    <button class="btn btn--ghost btn--sm" onclick={loadRecordingSettings} disabled={rec.loadingRecSettings}>
      {rec.loadingRecSettings ? "…" : "Reload"}
    </button>
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
        <Switch bind:checked={rec.draftCaptureScreen} />
      {/snippet}
    </SettingRow>

    <SettingRow label="Microphone" description="Capture audio from microphone">
      {#snippet control()}
        <Switch bind:checked={rec.draftCaptureMicrophone} />
      {/snippet}
    </SettingRow>

    <SettingRow
      label="System Audio"
      description={rec.draftCaptureScreen
        ? "Capture Mac system audio (macOS 15+)"
        : "Capture Mac system audio (macOS 15+). System Audio is unavailable — enable Screen first."}
      disabled={!rec.draftCaptureScreen}
    >
      {#snippet control()}
        <Switch
          bind:checked={rec.draftCaptureSystemAudio}
          disabled={!rec.draftCaptureScreen}
        />
      {/snippet}
    </SettingRow>

    <SettingRow
      label="Segment Duration"
      description="How long each recording segment is before a new one starts."
      full
      divider={false}
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
    >
      {#snippet control()}
        <Switch bind:checked={rec.draftPauseCaptureOnInactivity} />
      {/snippet}
    </SettingRow>

    {#if rec.draftPauseCaptureOnInactivity}
      <SettingRow label="Idle timeout" full>
        {#snippet control()}
          <div class="control-stack">
          <Slider
            bind:value={rec.draftIdleTimeoutSeconds}
            min={5}
            max={300}
            step={5}
            label="Idle timeout"
            unit="s"
            formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60 > 0 ? ` ${v%60}s` : ""}`.trim() : `${v}s`}
          />
          <p class="group-hint">
            Capture pauses after <strong>{rec.draftIdleTimeoutSeconds}s</strong> of system-wide inactivity (no mouse, keyboard,
            or other input anywhere on the Mac). It resumes automatically when system activity is detected again.
          </p>
          </div>
        {/snippet}
      </SettingRow>

      <SettingRow label="Activity Mode" full>
        {#snippet control()}
          <div class="control-stack">
          <RadioGroup
            bind:value={rec.draftActivityMode}
            options={[
              {
                value: "system_input_only",
                label: "Input only",
                description: "Only keyboard and mouse/pointer events count as activity. Recording pauses whenever direct input stops, even during video calls or media playback.",
              },
              {
                value: "system_input_or_screen",
                label: "Input or screen change",
                description: "Keyboard/mouse input AND visible on-screen changes (video calls, animations, media) both count as activity. Helps keep recordings running during calls or video playback with no direct input.",
              },
              {
                value: "system_input_or_screen_or_audio",
                label: "Input, screen, or audio",
                description: "All of the above, plus microphone and system audio levels. Sound picked up by the microphone or played through the system keeps capture active — useful for meetings, voice sessions, or any audio-driven workflow.",
              },
            ]}
          />
          <p class="group-hint">
            {#if rec.draftActivityMode === "system_input_or_screen_or_audio"}
              <strong>Audio mode</strong> monitors keyboard/mouse, on-screen changes, <em>and</em>
              source-specific audio activity. Microphone activity is speech-first when voice detection
              is enabled, while system audio still uses the configured level threshold.
            {:else if rec.draftActivityMode === "system_input_or_screen"}
              <strong>Screen change mode</strong> monitors on-screen activity in addition to input events — useful for
              keeping recordings active during video calls, live streams, or media playback where you may not be
              typing or moving the mouse.
            {:else}
              <strong>Input-only mode</strong> triggers the idle timeout strictly on keyboard and mouse inactivity.
              Suitable for general screen recording when you want pauses to match direct interaction gaps exactly.
            {/if}
          </p>
          </div>
        {/snippet}
      </SettingRow>

      {#if rec.draftActivityMode === "system_input_or_screen_or_audio"}
        <SettingRow label="Microphone Voice Detection" full>
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
              <p class="group-hint group-hint--warn">Microphone capture is disabled — this setting has no effect until enabled.</p>
            {:else}
              <p class="group-hint">
                {#if rec.draftMicrophoneVadAdapter !== "off"}
                  Tunes the compatibility peak-level fallback used when no speech adapter is available.
                {:else if rec.draftMicrophoneActivitySensitivity >= 80}
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
    {/if}
  </SettingGroup>
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
