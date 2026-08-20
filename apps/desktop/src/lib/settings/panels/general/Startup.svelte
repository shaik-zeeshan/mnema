<script lang="ts">
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  // Launch at login has NO settings field, deliberately: the login item is the
  // source of truth, and a mirrored boolean would disagree with reality the
  // moment someone removes Mnema in System Settings > General > Login Items.
  // So this switch reads and writes the OS directly. It is also the row that
  // makes the one above it mean anything — "auto-start recording on launch"
  // only fires if something launched the app.
  let launchAtLogin = $state(false);
  let launchAtLoginReady = $state(false);
  let launchAtLoginError = $state<string | null>(null);

  $effect(() => {
    void (async () => {
      try {
        launchAtLogin = await isEnabled();
      } catch (error) {
        launchAtLoginError = error instanceof Error ? error.message : String(error);
      } finally {
        launchAtLoginReady = true;
      }
    })();
  });

  async function setLaunchAtLogin(next: boolean): Promise<void> {
    const previous = launchAtLogin;
    launchAtLogin = next;
    launchAtLoginError = null;
    try {
      await (next ? enable() : disable());
    } catch (error) {
      // Snap back rather than leaving the switch claiming something the OS
      // never accepted.
      launchAtLogin = previous;
      launchAtLoginError = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<SettingGroup id="settings-section-startup" title="Startup">
  <SettingRow
    label="Launch Mnema at login"
    description={launchAtLoginError
      ? `Could not change the login item: ${launchAtLoginError}`
      : "Open Mnema automatically when you sign in"}
  >
    {#snippet control()}
      <Switch
        checked={launchAtLogin}
        disabled={!launchAtLoginReady}
        ariaLabel="Launch Mnema at login"
        onCheckedChange={setLaunchAtLogin}
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
