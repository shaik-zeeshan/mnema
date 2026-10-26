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
  import IconAlert from "~icons/lucide/triangle-alert";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import PermissionRows from "./PermissionRows.svelte";
  import { captureControls } from "$lib/capture-controls.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const sysAudio = c.systemAudioAccess;
  // Sources are fixed for a session, as the tray's toggles are: a switch flipped
  // mid-recording would save but not reach the running capture until next start.
  const sourcesLocked = $derived(
    captureControls.running || captureControls.loadingStart || captureControls.loadingStop,
  );

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
  const IDLE_STOPS = [10, 30, 60, 120, 300];
  const idleLabel = (s: number) =>
    s % 60 === 0 ? `${s / 60} minute${s === 60 ? "" : "s"}` : `${s} seconds`;
  const idleOptions = $derived(
    [...new Set([...IDLE_STOPS, rec.draftIdleTimeoutSeconds])]
      .sort((a, b) => a - b)
      .map((s) => ({ value: String(s), label: idleLabel(s) })),
  );


  const SYS_AUDIO_DESC = "Capture Mac system audio (macOS 15+) — meetings, videos, and app sound.";

  // One-line level read for the sensitivity sliders (same bands as before).
  const LEVELS = {
    mic: [
      "Very low — only very loud audio triggers activity.",
      "Low — only louder audio keeps capture active.",
      "Medium — normal speech triggers activity. Recommended.",
      "High — quiet speech counts as activity.",
      "Very high — whispers and background noise keep capture active.",
    ],
    sysaudio: [
      "Very low — only very loud system audio triggers activity.",
      "Low — only louder system audio keeps capture active.",
      "Medium — typical media playback triggers activity. Recommended.",
      "High — moderate system audio counts as activity.",
      "Very high — quiet system sounds keep capture active.",
    ],
  };
  const sensitivityLevel = (kind: "mic" | "sysaudio", v: number) =>
    LEVELS[kind][Math.min(4, Math.max(0, Math.floor(v / 20)))];
</script>

<!-- Inactivity sensitivity: slider right, its level as the description; a
     source that is off adds a warn line under the copy (so the slider moves
     to `aside`). -->
{#snippet sensitivity(kind: "mic" | "sysaudio")}
  {@const mic = kind === "mic"}
  {@const on = mic ? rec.draftCaptureMicrophone : rec.draftCaptureSystemAudio}
  {@const value = mic ? rec.draftMicrophoneActivitySensitivity : rec.draftSystemAudioActivitySensitivity}
  {@const label = mic ? "Microphone activity sensitivity" : "System audio activity sensitivity"}
  {#snippet slider()}
    <div class="cap-range">
      <Slider
        {value}
        onValueChange={(v) => (mic ? (rec.draftMicrophoneActivitySensitivity = v) : (rec.draftSystemAudioActivitySensitivity = v))}
        min={0}
        max={100}
        step={1}
        ariaLabel={label}
        disabled={!on}
      />
      <span class="num cap-range__v">{value}%</span>
    </div>
  {/snippet}
  {#if on}
    <SettingRow {label} description={sensitivityLevel(kind, value)} source={kind} control={slider} />
  {:else}
    <SettingRow {label} description={sensitivityLevel(kind, value)} source={kind} full aside={slider}>
      {#snippet control()}
        <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />{mic ? "Microphone" : "System audio"} capture is off — this has no effect until you turn it on.</span>
      {/snippet}
    </SettingRow>
  {/if}
{/snippet}

{#snippet sysAudioSwitch()}
  <Switch bind:checked={rec.draftCaptureSystemAudio} disabled={sourcesLocked} ariaLabel="System Audio" />
{/snippet}

<SettingGroup
  id="settings-section-capture"
  title="Sources"
  hint={sourcesLocked ? "Stop recording to change sources." : "Three independent families — any can run alone."}
  loading={rec.loadingRecSettings}
>
  {#snippet actions()}
    <ReloadButton onclick={loadRecordingSettings} busy={rec.loadingRecSettings} label="Reload capture settings" />
  {/snippet}

  {#if recValidationErrors.length > 0}
    <div class="mx-notice cap-top" data-tone="danger" role="alert">
      <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
      <b class="mx-notice__title">Unsaved changes blocked</b>
      {#each recValidationErrors as err}
        <p class="mx-notice__text">{err}</p>
      {/each}
    </div>
  {/if}

  <SettingRow label="Screen" description="Capture the display." source="screen">
    {#snippet control()}
      <Switch bind:checked={rec.draftCaptureScreen} disabled={sourcesLocked} ariaLabel="Screen" />
    {/snippet}
  </SettingRow>

  <SettingRow label="Microphone" description="Capture audio from microphone." source="mic">
    {#snippet control()}
      <Switch bind:checked={rec.draftCaptureMicrophone} disabled={sourcesLocked} ariaLabel="Microphone" />
    {/snippet}
  </SettingRow>

  <!-- System audio is its own capture family on a Core Audio process tap
       (ADR 0052): no screen dependency, so no `disabled` coupling to Screen.
       "May be blocked" is inferred, never known: a denied tap records silence
       and says nothing, so this is the only signal the user gets. -->
  {#if sysAudio.mayBeBlocked && rec.draftCaptureSystemAudio}
    <SettingRow label="System Audio" description={SYS_AUDIO_DESC} source="sysaudio" full aside={sysAudioSwitch}>
      {#snippet labelExtra()}<span class="mx-chip mx-chip--info">may be blocked</span>{/snippet}
      {#snippet control()}
        <div class="cap-stack">
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
      {/snippet}
    </SettingRow>
  {:else}
    <SettingRow label="System Audio" description={SYS_AUDIO_DESC} source="sysaudio" control={sysAudioSwitch} />
  {/if}

  <PermissionRows />

  <SettingRow
    label="Segment Duration"
    description="How long each recording segment is before a new one starts."
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
</SettingGroup>

{#if !rec.loadingRecSettings}
  <SettingGroup title="Inactivity" hint="Pause and resume rules when you step away.">
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
            {/if}
            </div>
          {/snippet}
        </SettingRow>

        {#if rec.draftMicrophoneVadAdapter === "off"}
          {@render sensitivity("mic")}
        {/if}
        {@render sensitivity("sysaudio")}

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

{/if}

<style>
  /* A full-width notice between rows, on the rows' left edge. */
  .cap-notice {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3) 0;
  }

  .cap-top {
    margin: var(--s-3) 0;
  }

  .cap-stack {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    width: 100%;
  }

  /* Mockup `.st-range` + `.st-range__v`. */
  .cap-range {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 220px;
  }

  .cap-range__v {
    min-width: 34px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    text-align: right;
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

</style>
