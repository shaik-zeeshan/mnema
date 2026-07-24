<script lang="ts">
  // Triggers settings — the ONE global detector knob (issue #182): the Meeting
  // release grace (docs/triggers/CONTEXT.md — deliberately NOT per-trigger; the
  // per-trigger tunables live in each trigger's Advanced disclosure on
  // /triggers). Persists to the `triggers.meeting_release_grace_minutes` kv via
  // its own command pair; the meeting detector re-reads it per tick.
  import { invoke } from "@tauri-apps/api/core";
  import Stepper from "$lib/components/Stepper.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const MIN_GRACE = 1;
  const MAX_GRACE = 15;

  let graceRaw = $state("2");
  let loaded = $state(false);
  let saveError = $state<string | null>(null);
  // Non-reactive last-persisted value so the save effect only fires on a real
  // change (and never re-saves what the load just read).
  let lastSaved: number | null = null;

  $effect(() => {
    void (async () => {
      try {
        const minutes = await invoke<number>("get_meeting_release_grace_minutes");
        lastSaved = minutes;
        graceRaw = String(minutes);
      } catch {
        // Keep the default; the save path will surface real errors.
      } finally {
        loaded = true;
      }
    })();
  });

  $effect(() => {
    const raw = graceRaw;
    if (!loaded) return;
    const minutes = parseInt(raw, 10);
    if (!Number.isFinite(minutes) || minutes < MIN_GRACE || minutes > MAX_GRACE) return;
    if (minutes === lastSaved) return;
    lastSaved = minutes;
    void invoke("set_meeting_release_grace_minutes", { minutes }).then(
      () => {
        saveError = null;
      },
      (error) => {
        saveError = String(error);
      },
    );
  });
</script>

<SettingGroup
  id="settings-section-triggers"
  title="Triggers"
  hint="Your triggers live on the Triggers page; this is the one global detector setting."
>
  <SettingRow
    label="Meeting release grace"
    description="How long a conferencing app must stay off the microphone before a meeting counts as ended. Absorbs drop/rejoin gaps and back-to-back calls; default 2 minutes."
    divider={false}
  >
    {#snippet control()}
      <div class="grace-control">
        <Stepper
          id="triggers-release-grace"
          bind:value={graceRaw}
          min={MIN_GRACE}
          max={MAX_GRACE}
          step={1}
          unit="MIN"
          disabled={!loaded}
          ariaLabel="Meeting release grace in minutes"
        />
        {#if saveError}
          <p class="error-text" role="alert">{saveError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .grace-control {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 140px;
  }
</style>
