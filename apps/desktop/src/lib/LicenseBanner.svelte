<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { licenseStatus } from "$lib/licensing-store.svelte";
  import { LICENSE_CHECKOUT_URL } from "$lib/licensing";
  import { openSettings } from "$lib/surface-windows";

  // App-shell licensing banner. Renders off the shared `licenseStatus` store —
  // no dedicated backend event (the `license_status` event already carries
  // `trial { daysLeft }` / `readOnly`). ponytail: `daysLeft` refreshes at
  // startup and on capture-start (the gate's recompute cadence), which is enough
  // for the final-week teach-in; a daily in-app timer is the upgrade path only
  // if long-running sessions need the count to tick down live.

  const status = $derived(licenseStatus.value);

  // Final-week trial banner: only when a trial is running with ≤7 days left.
  const trialDaysLeft = $derived(
    status?.kind === "trial" && status.daysLeft <= 7 ? status.daysLeft : null,
  );
  const isReadOnly = $derived(status?.kind === "readOnly");
  const isRevoked = $derived(status?.kind === "revoked");

  // Tone escalates subtly as expiry nears; ReadOnly is the firm (non-dismissible) end.
  const tone = $derived(
    isReadOnly
      ? "readonly"
      : trialDaysLeft !== null && trialDaysLeft <= 1
        ? "urgent"
        : trialDaysLeft !== null && trialDaysLeft <= 3
          ? "warn"
          : "info",
  );

  function days(n: number): string {
    return `${n} ${n === 1 ? "day" : "days"}`;
  }

  const trialMessage = $derived.by(() => {
    if (trialDaysLeft === null) return "";
    const lead =
      trialDaysLeft <= 1
        ? "Your free trial ends today."
        : `Free trial ends in ${days(trialDaysLeft)}.`;
    return `${lead} After that, Mnema switches to Read-Only Mode — your recorded history stays fully searchable; only new recording pauses until you buy.`;
  });

  // Dismissal is keyed to the current day-count so a fresh escalation (e.g. the
  // count dropping into the ≤3 or ==1 tier) re-surfaces the banner. ReadOnly is
  // never dismissible.
  let dismissedAtDays = $state<number | null>(null);
  const trialVisible = $derived(
    trialDaysLeft !== null && dismissedAtDays !== trialDaysLeft,
  );

  function dismiss() {
    dismissedAtDays = trialDaysLeft;
  }

  function openCheckout() {
    void openUrl(LICENSE_CHECKOUT_URL).catch((e) =>
      console.error("[LicenseBanner] open checkout failed", e),
    );
  }

  function enterLicense() {
    void openSettings("license");
  }
</script>

{#if isReadOnly}
  <div class="license-banner license-banner--readonly" role="alert">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      Your trial has ended. Everything you recorded stays browsable and searchable. Buy a
      license to resume recording.
    </p>
    <div class="license-banner__actions">
      <button type="button" class="license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy a license
      </button>
      <button type="button" class="license-banner__btn" onclick={enterLicense}>
        Enter license
      </button>
    </div>
  </div>
{:else if isRevoked}
  <div class="license-banner license-banner--readonly" role="alert">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">
      This license has been revoked. Everything you recorded stays browsable and searchable. Buy a
      license to resume recording.
    </p>
    <div class="license-banner__actions">
      <button type="button" class="license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy a license
      </button>
      <button type="button" class="license-banner__btn" onclick={enterLicense}>
        Enter license
      </button>
    </div>
  </div>
{:else if trialVisible}
  <div class="license-banner license-banner--{tone}" role="status">
    <span class="license-banner__dot" aria-hidden="true"></span>
    <p class="license-banner__text">{trialMessage}</p>
    <div class="license-banner__actions">
      <button type="button" class="license-banner__btn license-banner__btn--primary" onclick={openCheckout}>
        Buy Mnema
      </button>
      <button
        type="button"
        class="license-banner__btn license-banner__btn--dismiss"
        aria-label="Dismiss"
        onclick={dismiss}
      >
        Dismiss
      </button>
    </div>
  </div>
{/if}

<style>
  .license-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 14px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    font-size: var(--text-sm);
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
    line-height: 1.4;
  }

  .license-banner__actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .license-banner__btn {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid var(--app-border-strong);
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    white-space: nowrap;
  }

  .license-banner__btn:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }

  .license-banner__btn--primary {
    background: var(--tone-accent);
    border-color: var(--tone-accent);
    color: var(--app-accent-contrast);
  }

  .license-banner__btn--primary:hover {
    background: var(--tone-accent);
    filter: brightness(1.08);
  }

  .license-banner__btn--dismiss {
    color: var(--app-text-muted);
  }

  .license-banner__btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* Tone ramp — info → warn → urgent → the firm Read-Only end. */
  .license-banner--info {
    --tone-accent: var(--app-accent);
  }
  .license-banner--warn {
    --tone-accent: var(--app-warn);
    background: var(--app-warn-bg);
    border-bottom-color: var(--app-warn-border);
  }
  .license-banner--urgent,
  .license-banner--readonly {
    --tone-accent: var(--app-danger);
    background: var(--app-danger-bg);
    border-bottom-color: var(--app-danger-border);
  }
</style>
