<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import {
    getStartupSurface,
    setStartupSurface,
    type StartupSurface,
  } from "$lib/startup-surface";
  import { getEffectiveGlobalShortcut, type GlobalShortcutId } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";

  const c = getSettingsController();
  const rec = c.rec;

  // Shell preference, not a capture setting: it is stored in webview storage and
  // applies on the next cold start, so it saves on change instead of riding the
  // recording-settings draft/save cycle.
  let startupSurface = $state<StartupSurface>(getStartupSurface());

  function selectStartupSurface(next: string): void {
    startupSurface = next as StartupSurface;
    setStartupSurface(startupSurface);
  }

  // Platform-correct switcher keys (⌘1 / ⌘2 on macOS, Ctrl on Windows).
  const platform = detectKeyboardPlatform();
  function shortcutText(id: GlobalShortcutId): string {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "—";
  }
</script>

<SettingGroup id="settings-section-startup" title="Startup">
  <SettingRow
    label="Open Mnema on"
    description={`The surface a freshly launched window lands on. Switch anytime with ${shortcutText("openTimelineSurface")} / ${shortcutText("openOverviewSurface")}.`}
  >
    {#snippet control()}
      <Segmented
        options={[
          { value: "timeline", label: "Timeline" },
          { value: "overview", label: "Overview" },
        ]}
        value={startupSurface}
        onValueChange={selectStartupSurface}
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
