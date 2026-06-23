<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";

  const c = getSettingsController();
  const audio = c.audio;

  const micState = $derived(audio.micState);
  const micDeviceOptions = $derived(audio.micDeviceOptions);
  const loadingMicState = $derived(audio.loadingMicState);
  const loadMicState = () => audio.loadMicState();
</script>

<SettingGroup id="settings-section-audio" title="Microphone Controller">
  {#snippet actions()}
    <ReloadButton onclick={loadMicState} busy={loadingMicState} label="Reload microphone state" />
  {/snippet}

  {#if loadingMicState}
    <SettingRow label="Microphone" description="Microphone state is loading." divider={false}>
      {#snippet control()}
        <p class="loading-text">Loading microphone state…</p>
      {/snippet}
    </SettingRow>
  {:else if micState}
    <SettingRow label="Active Device" description="The microphone currently in use." full>
      {#snippet control()}
        <div class="control-stack">
          <div class="effective-device" class:effective-device--none={!micState.effectiveDevice}>
            <span class="effective-device__dot" class:effective-device__dot--on={!!micState.effectiveDevice}></span>
            <span class="effective-device__label">
              {#if micState.effectiveDevice}
                {micState.effectiveDevice.name}
                {#if micState.effectiveDevice.isDefault}
                  <span class="badge badge--neutral badge--sm">default</span>
                {/if}
              {:else}
                No active device
              {/if}
            </span>
          </div>
        </div>
      {/snippet}
    </SettingRow>

    <SettingRow label="Available Devices" description="Microphones detected on this Mac." full>
      {#snippet control()}
        <div class="control-stack">
          {#if micState.devices.length > 0}
            <ul class="device-list">
              {#each micState.devices as device (device.id)}
                <li class="device-item" class:device-item--active={micState.effectiveDevice?.id === device.id}>
                  <span class="device-item__dot" class:device-item__dot--active={micState.effectiveDevice?.id === device.id}></span>
                  <span class="device-item__name">{device.name}</span>
                  <div class="device-item__badges">
                    {#if device.isDefault}
                      <span class="badge badge--neutral badge--sm">default</span>
                    {/if}
                    {#if micState.effectiveDevice?.id === device.id}
                      <span class="badge badge--ok badge--sm">active</span>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="empty-state">No microphone devices found.</p>
          {/if}
        </div>
      {/snippet}
    </SettingRow>

    <SettingRow label="Preference" description="Which microphone capture should use." full>
      {#snippet control()}
        <RadioGroup
          bind:value={audio.draftPreferenceMode}
          options={[
            { value: "default", label: "System Default", description: "Use the currently selected system microphone" },
            { value: "specific_device", label: "Specific Device", description: "Lock to a particular microphone" },
          ]}
        />
      {/snippet}
    </SettingRow>

    {#if audio.draftPreferenceMode === "specific_device"}
      <SettingRow label="Device" description="Pick the microphone to lock to." full>
        {#snippet control()}
          <div class="control-stack">
            {#if micDeviceOptions.length > 0}
              <RadioGroup
                value={audio.draftDeviceId ?? ""}
                onValueChange={(v) => (audio.draftDeviceId = v)}
                options={micDeviceOptions}
              />
            {:else}
              <p class="empty-state">No microphone devices to choose from.</p>
            {/if}
            {#if !audio.draftDeviceId}
              <p class="group-hint group-hint--warn">Select a device before saving Specific Device mode.</p>
            {/if}
          </div>
        {/snippet}
      </SettingRow>
    {/if}

    <SettingRow
      label="On Disconnect"
      description="What to do when the chosen microphone disconnects."
      full
      divider={!!audio.micError}
    >
      {#snippet control()}
        <RadioGroup
          bind:value={audio.draftDisconnectPolicy}
          options={[
            { value: "fallback_to_default", label: "Fallback to Default", description: "Switch to system default when device disconnects" },
            { value: "wait_for_same_device", label: "Wait for Same Device", description: "Pause microphone capture until the device reconnects" },
          ]}
        />
      {/snippet}
    </SettingRow>

    {#if audio.micError}
      <SettingRow label="Error" warn full divider={false}>
        {#snippet control()}
          <div class="inline-error">
            <span class="inline-error__icon">⚠</span>
            <span class="inline-error__msg">{audio.micError}</span>
            <button class="btn btn--ghost btn--sm" onclick={() => audio.micError = null}>×</button>
          </div>
        {/snippet}
      </SettingRow>
    {/if}
  {:else}
    <SettingRow label="Microphone" description="Microphone state could not be loaded." full divider={false}>
      {#snippet control()}
        <div class="control-stack">
          <p class="empty-state">Failed to load microphone state.</p>
          <button class="btn btn--ghost btn--sm" onclick={loadMicState}>Retry</button>
        </div>
      {/snippet}
    </SettingRow>
  {/if}
</SettingGroup>

<style>
  /* SettingRow's full-mode control slot is a flex row; stack the device banner,
     list, select + hint, and error block vertically. */
  .control-stack {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    min-width: 0;
    align-items: flex-start;
  }

  .control-stack :global(.group-hint) {
    margin: 0;
  }

  .control-stack :global(.device-list),
  .control-stack :global(.effective-device) {
    width: 100%;
  }
</style>
