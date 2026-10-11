<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-cw";
  import IconMic from "~icons/lucide/mic";

  const c = getSettingsController();
  const audio = c.audio;

  const micState = $derived(audio.micState);
  const loadMicState = () => audio.loadMicState();
  const noDevices = $derived(micState !== null && micState.devices.length === 0);
</script>

{#snippet needsDevice()}
  <span class="mx-inline" data-tone="warn"><IconAlert width="13" height="13" />Select a device before saving Specific Device mode.</span>
{/snippet}

<SettingGroup id="settings-section-audio" title="Microphone" loading={audio.loadingMicState}>
  {#if micState === null}
    <div class="mic-pad">
      <div class="mx-empty mx-empty--compact" data-tone="danger">
        <span class="mx-empty__glyph"><IconAlert width="14" height="14" /></span>
        <b class="mx-empty__title">Couldn’t load microphone state</b>
        <p class="mx-empty__text">Devices and your preference will show here once it loads.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={loadMicState}>
            <IconRetry width="12" height="12" />Retry
          </button>
        </div>
      </div>
    </div>
  {:else}
    {#if noDevices}
      <div class="mic-pad">
        <div class="mx-empty mx-empty--compact">
          <span class="mx-empty__glyph"><IconMic width="14" height="14" /></span>
          <b class="mx-empty__title">No microphone devices found</b>
          <p class="mx-empty__text">Plug one in or connect a headset — it shows up here on its own.</p>
        </div>
      </div>
    {:else}
      <SettingRow label="Active device" description="The microphone currently in use.">
        {#snippet control()}
          {#if micState.effectiveDevice}
            <span class="mx-chip mx-chip--mic">{micState.effectiveDevice.name}</span>
          {:else}
            <span class="mx-chip">No active device</span>
          {/if}
          <ReloadButton onclick={loadMicState} busy={audio.loadingMicState} label="Reload microphone state" />
        {/snippet}
      </SettingRow>
    {/if}

    <SettingRow label="Preference" description="Which microphone capture should use.">
      {#snippet control()}
        <Segmented
          md
          bind:value={audio.draftPreferenceMode}
          ariaLabel="Microphone preference"
          options={[
            { value: "default", label: "System Default" },
            { value: "specific_device", label: "Specific Device" },
          ]}
        />
      {/snippet}
    </SettingRow>

    {#if audio.draftPreferenceMode === "specific_device"}
      {#if !noDevices}
        <SettingRow label="Device" description="Pick the microphone to lock to.">
          {#snippet control()}
            <div class="mic-device">
              <SelectMenu
                value={audio.draftDeviceId}
                onValueChange={(v) => (audio.draftDeviceId = v)}
                options={audio.micDeviceOptions}
                placeholder="Choose a microphone"
                ariaLabel="Device"
              />
              {#if !audio.draftDeviceId}{@render needsDevice()}{/if}
            </div>
          {/snippet}
        </SettingRow>
      {:else if !audio.draftDeviceId}
        <div class="mic-pad">{@render needsDevice()}</div>
      {/if}

      <SettingRow label="On Disconnect" description="What to do when the chosen microphone disconnects.">
        {#snippet control()}
          <Segmented
            md
            bind:value={audio.draftDisconnectPolicy}
            ariaLabel="On disconnect policy"
            options={[
              { value: "fallback_to_default", label: "Fallback to Default" },
              { value: "wait_for_same_device", label: "Wait for Same Device" },
            ]}
          />
        {/snippet}
      </SettingRow>
    {/if}
  {/if}

  {#if audio.micError}
    <div class="mic-pad">
      <span class="mx-inline" data-tone="danger" role="alert">
        <IconAlert width="13" height="13" />{audio.micError}
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => (audio.micError = null)}>Dismiss</button>
      </span>
    </div>
  {/if}
</SettingGroup>

<style>
  /* A full-width state between rows, on the rows' left edge. */
  .mic-pad {
    padding: var(--s-3) 0;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .mic-device {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--s-2);
    min-width: 220px;
  }
</style>
