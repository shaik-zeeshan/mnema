<script lang="ts">
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { LicenseDevices } from "$lib/licensing";
  import {
    licenseStatus,
    activateLicense,
    getLicenseDevices,
    refreshLicenseNow,
    resetLicenseDevices,
  } from "$lib/licensing-store.svelte";
  import {
    badgeFor,
    checkoutUrlFor,
    fmtLicenseDate,
    licensedOutOfWindow as isLicensedOutOfWindow,
    safeExternalUrl,
    showBuyFor,
    statusHintFor,
    statusInlineFor,
  } from "$lib/licensing-panel";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import IconExternal from "~icons/lucide/external-link";
  import IconCheck from "~icons/lucide/check";
  import IconLock from "~icons/lucide/lock";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconInfo from "~icons/lucide/info";
  import IconRetry from "~icons/lucide/rotate-ccw";

  // All presentation policy (chip, hint, inline, buy-vs-renew, external-URL
  // vetting) lives in `licensing-panel.ts`; this component only renders.
  const status = $derived(licenseStatus.value);
  const licensedOutOfWindow = $derived(isLicensedOutOfWindow(status));
  const showBuy = $derived(showBuyFor(status));
  const badge = $derived(badgeFor(status));
  const inline = $derived(statusInlineFor(status));
  const overCap = $derived(
    status?.kind === "licensed" && status.activation.state === "refusedOverCap" ? status.activation : null,
  );
  const INLINE_ICONS = { lock: IconLock, alert: IconAlert, cloud: IconCloudOff, info: IconInfo };

  let key = $state("");
  let activating = $state(false);
  let activateError = $state<string | null>(null);
  let activated = $state(false);

  const canActivate = $derived(key.trim().length > 0 && !activating);

  async function submitActivate() {
    const trimmed = key.trim();
    if (trimmed.length === 0 || activating) return;
    activating = true;
    activateError = null;
    activated = false;
    try {
      await activateLicense(trimmed);
      // On success the store updates `status` reactively — clear the input and
      // confirm the action explicitly (the status flip alone is easy to miss).
      key = "";
      activated = true;
    } catch (err) {
      activateError =
        typeof err === "string"
          ? err
          : ((err as Error)?.message ?? "This license key is invalid or corrupted.");
    } finally {
      activating = false;
    }
  }

  // Editing the key clears both the error and the success confirmation.
  function onKeyInput() {
    if (activateError) activateError = null;
    if (activated) activated = false;
  }

  // Manual Receipt Refresh: the status itself updates reactively via the
  // `license_status` event — this state only drives the row's transient note.
  let refreshing = $state(false);
  let refreshNote = $state<"ok" | "failed" | null>(null);
  let refreshNoteTimer: ReturnType<typeof setTimeout> | undefined;

  async function submitRefresh() {
    if (refreshing) return;
    refreshing = true;
    refreshNote = null;
    clearTimeout(refreshNoteTimer);
    try {
      await refreshLicenseNow();
      refreshNote = "ok";
    } catch {
      refreshNote = "failed";
    } finally {
      refreshing = false;
      refreshNoteTimer = setTimeout(() => (refreshNote = null), 4000);
    }
  }

  function openCheckout() {
    // Lapsed owners renew ($29); everyone else buys the license ($69).
    void openUrl(checkoutUrlFor(status)).catch((e) =>
      console.error("[License] open checkout failed", e),
    );
  }

  // Server-provided links (reset/buy from an over-cap 409) — https only.
  function openExternal(url: string) {
    const safe = safeExternalUrl(url);
    if (!safe) {
      console.error("[License] refusing to open non-https external url", url);
      return;
    }
    void openUrl(safe).catch((e) => console.error("[License] open external failed", e));
  }

  // Device COUNT (never a list) from the server, fetched lazily while the
  // panel is showing a licensed key; `null` (offline / no license) renders
  // nothing rather than stale numbers. Re-fetched on every status flip so a
  // successful reset/activation updates the count.
  let devices = $state<LicenseDevices | null>(null);
  $effect(() => {
    if (status?.kind === "licensed") {
      void getLicenseDevices().then((next) => {
        devices = next;
      });
    } else {
      devices = null;
    }
  });
  const hint = $derived(statusHintFor(status, devices));

  // "Free up my devices" (over-cap self-service): reset → the backend retries
  // activation itself; success surfaces as the status flipping to Activated.
  let resetting = $state(false);
  let resetError = $state<string | null>(null);
  let resetNotice = $state<string | null>(null);

  async function submitReset() {
    if (resetting) return;
    const ok = await confirm(
      "Free up your devices? This clears the license's device list so this Mac can activate. You can do this once every 30 days.",
      { title: "Free up my devices", kind: "warning" },
    );
    if (!ok) return;
    resetting = true;
    resetError = null;
    resetNotice = null;
    try {
      const result = await resetLicenseDevices();
      if (result.outcome === "reset") {
        // Activation was retried in the same breath; if it landed, the status
        // event flips this whole branch to Licensed. This notice only shows
        // when activation is still catching up.
        resetNotice = "Devices freed — finishing activation on this Mac…";
        void getLicenseDevices().then((next) => {
          devices = next;
        });
      } else {
        resetNotice =
          result.retryAtMs !== null
            ? `Your devices were already reset recently — you can reset again on ${fmtLicenseDate(result.retryAtMs)}.`
            : "Your devices were already reset recently — you can reset again later.";
      }
    } catch (err) {
      resetError =
        typeof err === "string"
          ? err
          : ((err as Error)?.message ?? "Could not free up your devices. Try again later.");
    } finally {
      resetting = false;
    }
  }
</script>

<SettingGroup
  id="settings-section-license"
  title="License & Trial"
  hint="Buy once, keep it forever. Verified offline."
>
  <SettingRow label="Status" description={hint ?? undefined} full={!status || !!inline || !!overCap}>
    {#snippet labelExtra()}
      {#if badge}
        <span class="mx-chip {badge.variant === 'plain' ? '' : `mx-chip--${badge.variant}`}">{badge.label}</span>
      {/if}
    {/snippet}
    {#snippet control()}
      {#if !status}
        <span class="mx-inline" role="status"><span class="mx-spin mx-spin--sm"></span>Checking license status…</span>
      {:else if overCap}
        <div class="lic-stack">
          <div class="mx-notice" data-tone="warn">
            <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
            <b class="mx-notice__title">Already active on its 3 devices</b>
            <p class="mx-notice__text">Free them up to move it here (once every 30 days), or buy another license.</p>
            <div class="mx-notice__acts">
              <button type="button" class="mx-btn mx-btn--sm" disabled={resetting} aria-busy={resetting} onclick={() => void submitReset()}>
                {#if resetting}<span class="mx-spin mx-spin--sm"></span>Freeing up…{:else}Free up my devices…{/if}
              </button>
              <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => openExternal(overCap.buyUrl)}>
                Buy another<IconExternal width="12" height="12" />
              </button>
              <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => openExternal(overCap.resetUrl)}>
                Help<IconExternal width="12" height="12" />
              </button>
            </div>
          </div>
          {#if resetError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{resetError}</span>
          {:else if resetNotice}
            <span class="mx-inline" role="status"><IconInfo width="13" height="13" />{resetNotice}</span>
          {/if}
          {#if inline}
            {@const Icon = INLINE_ICONS[inline.icon]}
            <span class="mx-inline" data-tone={inline.tone}><Icon width="13" height="13" />{inline.text}</span>
          {/if}
        </div>
      {:else if inline}
        {@const Icon = INLINE_ICONS[inline.icon]}
        <span class="mx-inline lic-under" data-tone={inline.tone}><Icon width="13" height="13" />{inline.text}</span>
      {/if}
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Refresh license status"
    description="Re-check with the license server — a renewal or freed-up device lands right away."
  >
    {#snippet control()}
      {#if refreshNote === "ok"}
        <span class="mx-inline" data-tone="ok" role="status"><IconCheck width="13" height="13" />Checked just now</span>
      {:else if refreshNote === "failed"}
        <span class="mx-inline" data-tone="danger" role="alert"><IconCloudOff width="13" height="13" />Failed to reach the server.</span>
      {/if}
      <button type="button" class="mx-btn" disabled={refreshing} aria-busy={refreshing} onclick={() => void submitRefresh()}>
        {#if refreshing}<span class="mx-spin mx-spin--sm"></span>Checking…{:else if refreshNote === "failed"}<IconRetry width="13" height="13" />Refresh{:else}Refresh{/if}
      </button>
    {/snippet}
  </SettingRow>

  {#if showBuy}
    <SettingRow
      label={licensedOutOfWindow ? "Renew" : "Buy Mnema"}
      description={licensedOutOfWindow
        ? "Renew ($29) to extend your update window for another year of new builds."
        : "$69 one-time purchase — includes a 1-year update window. Pay once, own it forever."}
    >
      {#snippet control()}
        <button type="button" class="mx-btn" onclick={openCheckout}>
          {licensedOutOfWindow ? "Renew ($29)" : "Buy ($69)"}<IconExternal width="12" height="12" />
        </button>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow
    label="Activate license"
    description="Paste the license key from your purchase email to unlock recording on this device."
    full
  >
    {#snippet control()}
      <div class="lic-stack">
        <div class="lic-keyfield">
          <input
            class="mx-input"
            type="text"
            autocomplete="off"
            spellcheck="false"
            placeholder="Paste your license key"
            aria-label="License key"
            aria-invalid={!!activateError}
            aria-describedby={activateError ? "license-activate-error" : undefined}
            disabled={activating}
            bind:value={key}
            oninput={onKeyInput}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void submitActivate();
              }
            }}
          />
          <button type="button" class="mx-btn" disabled={!canActivate} aria-busy={activating} onclick={() => void submitActivate()}>
            {#if activating}<span class="mx-spin mx-spin--sm"></span>Activating…{:else}Activate{/if}
          </button>
        </div>
        {#if activateError}
          <span class="mx-inline" data-tone="danger" id="license-activate-error" role="alert"><IconAlert width="13" height="13" />{activateError}</span>
        {:else if activated}
          <span class="mx-inline" data-tone="ok" role="status"><IconCheck width="13" height="13" />License activated — recording unlocked on this device.</span>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .lic-stack {
    display: grid;
    gap: var(--s-2);
    width: 100%;
  }

  /* An inline under the Status label sits 6px below it (mockup `.st-row .mx-inline`),
     not the full row's 12px control gap. */
  .lic-under {
    margin-top: -6px;
  }

  /* Mockup `.st-keyfield`: the key field takes the row, Activate rides beside it. */
  .lic-keyfield {
    display: flex;
    gap: var(--s-2);
    width: 100%;
  }

  .lic-keyfield .mx-input {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-base);
  }
</style>
