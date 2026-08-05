<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { licenseStatus, refreshLicenseNow } from "$lib/licensing-store.svelte";
  import { LICENSE_CHECKOUT_URL } from "$lib/licensing";
  import { bannerFor, bannerVisible, days } from "$lib/licensing-banner";
  import { openSettings } from "$lib/surface-windows";

  // App-shell licensing banner. Renders off the shared `licenseStatus` store —
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

{#if banner?.kind === "readOnly"}
  <div class="license-banner license-banner--readonly" role="alert">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      Your trial has ended. Everything you recorded stays browsable and searchable. Buy a
      license to resume recording.
    </p>
    <div class="license-banner__actions">
      <button type="button" class="btn btn--sm btn--push license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy a license
      </button>
      <button type="button" class="btn btn--sm btn--push license-banner__btn" onclick={enterLicense}>
        Enter license
      </button>
    </div>
  </div>
{:else if banner?.kind === "revoked"}
  <div class="license-banner license-banner--readonly" role="alert">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      This license has been revoked. Everything you recorded stays browsable and searchable. Buy a
      license to resume recording.
    </p>
    <div class="license-banner__actions">
      <button type="button" class="btn btn--sm btn--push license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy a license
      </button>
      <button type="button" class="btn btn--sm btn--push license-banner__btn" onclick={enterLicense}>
        Enter license
      </button>
    </div>
  </div>
{:else if banner?.kind === "lapsed"}
  <div class="license-banner license-banner--readonly" role="alert">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      We couldn't confirm your license. Connect to the internet once to finish activation — your
      recorded history stays fully searchable; new recording is paused until then.
    </p>
    <div class="license-banner__actions">
      <button
        type="button"
        class="btn btn--sm btn--push license-banner__btn license-banner__btn--primary"
        disabled={rechecking}
        onclick={() => void recheck()}
      >
        {rechecking ? "Checking…" : "Re-check license"}
      </button>
      <button type="button" class="btn btn--sm btn--push license-banner__btn" onclick={enterLicense}>
        Enter license
      </button>
    </div>
  </div>
{:else if banner?.kind === "provisional" && visible}
  <div class="license-banner license-banner--warn" role="status">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      Activation still pending — connect to the internet within {days(banner.daysLeft)} to
      keep recording.
    </p>
    <div class="license-banner__actions">
      <button
        type="button"
        class="btn btn--sm btn--ghost license-banner__btn"
        aria-label="Dismiss"
        onclick={dismiss}
      >
        Dismiss
      </button>
    </div>
  </div>
{:else if banner?.kind === "trial" && visible}
  <div class="license-banner license-banner--{banner.tone}" role="status">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">{banner.message}</p>
    <div class="license-banner__actions">
      <button type="button" class="btn btn--sm btn--push license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy Mnema
      </button>
      <button
        type="button"
        class="btn btn--sm btn--ghost license-banner__btn"
        aria-label="Dismiss"
        onclick={dismiss}
      >
        Dismiss
      </button>
    </div>
  </div>
{/if}

<style>
  /* Bento Native: a banner is chrome under the toolbar, so it is an opaque
     surface step with one hairline seam — never a bordered card, never a
     shadow. Its tone lives in the dot and (for the firm end) the fill. */
  .license-banner {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: var(--s-8) var(--s-16);
    box-shadow: 0 var(--hairline) 0 var(--app-border);
    background: var(--app-surface-subtle);
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-text);
  }

  .license-banner__dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--tone-accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--tone-accent) 22%, transparent);
  }

  .license-banner__text {
    margin: 0;
    flex: 1 1 auto;
  }

  .license-banner__actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  /* The actions are the shared `.btn`; only the tone tint on the primary one
     is local, so a firm banner's call to action wears the banner's colour. */
  .license-banner__btn--primary {
    background: var(--tone-accent);
    background-image: none;
    border-color: transparent;
    color: var(--app-accent-contrast);
  }

  .license-banner__btn--primary:hover {
    background: var(--tone-accent);
    filter: brightness(1.08);
  }

  /* Tone ramp — info → warn → urgent → the firm Read-Only end. */
  .license-banner--info {
    --tone-accent: var(--app-accent);
  }
  .license-banner--warn {
    --tone-accent: var(--app-warn);
    background: var(--app-warn-bg);
    box-shadow: 0 var(--hairline) 0 var(--app-warn-border);
  }
  .license-banner--urgent,
  .license-banner--readonly {
    --tone-accent: var(--app-danger);
    background: var(--app-danger-bg);
    box-shadow: 0 var(--hairline) 0 var(--app-danger-border);
  }
</style>
