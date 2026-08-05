<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import {
    getMainSurfaceSetting,
    setMainSurfaceSetting,
    type MainSurface,
  } from "$lib/main-surface";

  const c = getSettingsController();
  const rec = c.rec;

  // `ui.main_surface` (frame 07/12): self-contained load + save-on-change —
  // this key lives in the `app_settings` kv table, outside the recording
  // settings drafts the controller owns.
  let mainSurface = $state<MainSurface>("timeline");
  let mainSurfaceLoaded = $state(false);
  $effect(() => {
    void getMainSurfaceSetting()
      .then((surface) => {
        mainSurface = surface;
        mainSurfaceLoaded = true;
      })
      .catch(() => {
        // Leave the control disabled rather than showing a possibly-wrong value.
      });
  });
  function onMainSurfaceChange(value: string): void {
    mainSurface = value === "overview" ? "overview" : "timeline";
    void setMainSurfaceSetting(mainSurface).catch(() => {
      // Best-effort; reload shows the persisted truth.
    });
  }
</script>

<SettingGroup id="settings-section-startup" title="Startup">
  <SettingRow
    label="Open Mnema on"
    description="The surface shown at launch"
  >
    {#snippet control()}
      <Segmented
        options={[
          { value: "timeline", label: "Timeline" },
          { value: "overview", label: "Overview" },
        ]}
        value={mainSurface}
        onValueChange={onMainSurfaceChange}
        disabled={!mainSurfaceLoaded}
        ariaLabel="Open Mnema on"
      />
    {/snippet}
  </SettingRow>
  <SettingRow
    label="Auto-start recording on launch"
    description="Begin capturing immediately when the app opens"
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftAutoStart} ariaLabel="Auto-start recording on launch" />
    {/snippet}
  </SettingRow>
</SettingGroup>
