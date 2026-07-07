<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { licenseStatus, activateLicense } from "$lib/licensing-store.svelte";
  import { LICENSE_CHECKOUT_URL } from "$lib/licensing";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import IconArrowUpRight from "~icons/lucide/arrow-up-right";

  const status = $derived(licenseStatus.value);

  // Lapsed owner (out of update window) → Renew variant; in-window owner → hide Buy.
  const licensedOutOfWindow = $derived(status?.kind === "licensed" && !status.inWindow);
  const showBuy = $derived(!(status?.kind === "licensed" && status.inWindow));

  function fmtDate(ms: number): string {
    return new Date(ms).toLocaleDateString(undefined, {
      year: "numeric",
      month: "long",
      day: "numeric",
    });
  }

  function days(n: number): string {
    return `${n} ${n === 1 ? "day" : "days"}`;
  }

  let key = $state("");
  let activating = $state(false);
  let activateError = $state<string | null>(null);

  const canActivate = $derived(key.trim().length > 0 && !activating);

  async function submitActivate() {
    const trimmed = key.trim();
    if (trimmed.length === 0 || activating) return;
    activating = true;
    activateError = null;
    try {
      await activateLicense(trimmed);
      // On success the store updates `status` reactively — clear the input.
      key = "";
    } catch (err) {
      activateError =
        typeof err === "string"
          ? err
          : ((err as Error)?.message ?? "This license key is invalid or corrupted.");
    } finally {
      activating = false;
    }
  }

  function openCheckout() {
    void openUrl(LICENSE_CHECKOUT_URL).catch((e) =>
      console.error("[License] open checkout failed", e),
    );
  }
</script>

<SettingGroup
  id="settings-section-license"
  title="License & Trial"
  hint="Buy once, keep it forever. Everything is verified offline — no account, no phoning home."
>
  <SettingRow label="Status" description="Your current trial or license state." full>
    {#snippet control()}
      <div class="license-status">
        {#if !status}
          <p class="group-hint">Checking license status…</p>
        {:else if status.kind === "trialNotStarted"}
          <p class="group-hint">
            Free trial — starts when you first record ({days(status.trialDays)}).
          </p>
        {:else if status.kind === "trial"}
          <p class="group-hint">Free trial — {days(status.daysLeft)} left.</p>
          <p class="group-hint">
            At expiry the app switches to Read-Only Mode — your recorded history stays fully
            searchable, only new recording pauses until you buy.
          </p>
        {:else if status.kind === "readOnly"}
          <p class="group-hint group-hint--warn">
            Trial ended — Read-Only Mode. Recorded history stays searchable; buy to resume
            recording.
          </p>
        {:else if status.kind === "licensed"}
          <p class="group-hint">Licensed to {status.email}.</p>
          {#if status.inWindow}
            <p class="group-hint">Updates included through {fmtDate(status.updateThroughMs)}.</p>
          {:else}
            <p class="group-hint group-hint--warn">
              Update window lapsed — you keep this version forever; renew for new builds.
            </p>
          {/if}
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  {#if showBuy}
    <SettingRow
      label={licensedOutOfWindow ? "Renew" : "Buy Mnema"}
      description={licensedOutOfWindow
        ? "Renew ($29) to extend your update window for another year of new builds."
        : "$69 one-time purchase — includes a 1-year update window. Pay once, own it forever."}
      full
    >
      {#snippet control()}
        <div class="row-actions">
          <button type="button" class="btn btn--primary btn--sm" onclick={openCheckout}>
            {licensedOutOfWindow ? "Renew ($29)" : "Buy ($69)"}
            <span aria-hidden="true"><IconArrowUpRight /></span>
          </button>
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow
    label="Activate license"
    description="Paste the license key from your purchase email to unlock recording on this device."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="license-activate">
        <input
          class="text-input"
          class:text-input--error={!!activateError}
          type="text"
          autocomplete="off"
          placeholder="Paste your license key"
          aria-label="License key"
          aria-invalid={!!activateError}
          aria-describedby={activateError ? "license-activate-error" : undefined}
          disabled={activating}
          bind:value={key}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void submitActivate();
            }
          }}
        />
        <div class="row-actions">
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            disabled={!canActivate}
            aria-busy={activating}
            onclick={() => void submitActivate()}
          >
            {#if activating}<ButtonSpinner />Activating{:else}Activate{/if}
          </button>
        </div>
        {#if activateError}
          <p class="error-text" id="license-activate-error" role="alert">{activateError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .license-status,
  .license-activate {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .license-status p {
    margin: 0;
  }
</style>
