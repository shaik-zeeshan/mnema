<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { licenseStatus, refreshLicenseNow } from "$lib/licensing-store.svelte";
  import { LICENSE_CHECKOUT_URL } from "$lib/licensing";
  import { bannerFor, bannerVisible, days } from "$lib/licensing-banner";
  import { openSettings } from "$lib/surface-windows";
  import { captureControls } from "$lib/capture-controls.svelte";

  // App-shell banner (kit `mx-banner`, one at a time): licensing first, then
  // the low-disk recording pause. Renders off the shared `licenseStatus` store —
  // no dedicated backend event (the `license_status` event already carries
  // `trial { daysLeft }` / `readOnly`). All policy (precedence, thresholds,
  // tone, dismissal keying) lives in `licensing-banner.ts`; this component only
  // renders. ponytail: `daysLeft` refreshes at startup and on capture-start
  // (the gate's recompute cadence), which is enough for the final-week
  // teach-in; a daily in-app timer is the upgrade path only if long-running
  // sessions need the count to tick down live.

  const banner = $derived(bannerFor(licenseStatus.value));

  // Dismissal is keyed per-kind to the banner's day-count so a fresh escalation
  // (e.g. 3 → 2) re-surfaces it. Firm banners have a null key — never dismissible.
  let dismissedTrialKey = $state<number | null>(null);
  let dismissedProvisionalKey = $state<number | null>(null);
  const visible = $derived(
    bannerVisible(
      banner,
      banner?.kind === "provisional" ? dismissedProvisionalKey : dismissedTrialKey,
    ),
  );

  function dismiss() {
    if (banner?.kind === "trial") dismissedTrialKey = banner.dismissKey;
    else if (banner?.kind === "provisional") dismissedProvisionalKey = banner.dismissKey;
  }

  function openCheckout() {
    void openUrl(LICENSE_CHECKOUT_URL).catch((e) =>
      console.error("[LicenseBanner] open checkout failed", e),
    );
  }

  function enterLicense() {
    void openSettings("license");
  }

  // "Re-check license": manual Receipt Refresh — forces a re-activation; a
  // heal flips the banner away via the `license_status` event.
  let rechecking = $state(false);

  async function recheck() {
    if (rechecking) return;
    rechecking = true;
    try {
      await refreshLicenseNow();
    } catch (e) {
      console.error("[LicenseBanner] re-check failed", e);
    } finally {
      rechecking = false;
    }
  }
</script>

{#snippet dismissButton()}
  <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Dismiss" onclick={dismiss}>
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12" /></svg>
  </button>
{/snippet}

{#if banner?.kind === "readOnly" || banner?.kind === "revoked"}
  <div class="mx-banner" data-tone="danger" role="alert">
    <p>
      <b>{banner.kind === "readOnly" ? "Your trial has ended." : "This license has been revoked."}</b>
      Everything you recorded stays browsable and searchable. Buy a license to resume recording.
    </p>
    <div class="mx-banner__acts">
      <button type="button" class="mx-btn mx-btn--sm" onclick={openCheckout}>Buy a license</button>
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={enterLicense}>Enter license</button>
    </div>
  </div>
{:else if banner?.kind === "lapsed"}
  <div class="mx-banner" data-tone="danger" role="alert">
    <p>
      <b>We couldn't confirm your license.</b> Connect to the internet once to finish activation — your
      recorded history stays fully searchable; new recording is paused until then.
    </p>
    <div class="mx-banner__acts">
      <button type="button" class="mx-btn mx-btn--sm" aria-busy={rechecking} disabled={rechecking} onclick={() => void recheck()}>
        {#if rechecking}<span class="mx-spin mx-spin--sm"></span>Checking…{:else}Re-check license{/if}
      </button>
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={enterLicense}>Enter license</button>
    </div>
  </div>
{:else if banner?.kind === "provisional" && visible}
  <div class="mx-banner" data-tone="warn" role="status">
    <p><b>Activation still pending.</b> Connect to the internet within {days(banner.daysLeft)} to keep recording.</p>
    <div class="mx-banner__acts">{@render dismissButton()}</div>
  </div>
{:else if banner?.kind === "trial" && visible}
  <div class="mx-banner" data-tone={banner.tone === "urgent" ? "danger" : banner.tone} role="status">
    <p>{banner.message}</p>
    <div class="mx-banner__acts">
      <button type="button" class="mx-btn mx-btn--sm" onclick={openCheckout}>Buy Mnema</button>
      {@render dismissButton()}
    </div>
  </div>
{:else if captureControls.isLowDiskSuspended}
  <div class="mx-banner" data-tone="warn" role="status">
    <p><b>Recording paused — your disk is almost full.</b> Free up space and it resumes on its own.</p>
    <div class="mx-banner__acts">
      <button type="button" class="mx-btn mx-btn--sm" onclick={() => void openSettings("storage")}>Open Storage</button>
    </div>
  </div>
{/if}
