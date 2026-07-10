<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { licenseStatus, activateLicense } from "$lib/licensing-store.svelte";
  import {
    badgeFor,
    checkoutUrlFor,
    licensedOutOfWindow as isLicensedOutOfWindow,
    safeExternalUrl,
    showBuyFor,
  } from "$lib/licensing-panel";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import IconArrowUpRight from "~icons/lucide/arrow-up-right";
  import IconCheck from "~icons/lucide/check";

  // All presentation policy (badge, buy-vs-renew, external-URL vetting) lives
  // in `licensing-panel.ts`; this component only renders.
  const status = $derived(licenseStatus.value);
  const licensedOutOfWindow = $derived(isLicensedOutOfWindow(status));
  const showBuy = $derived(showBuyFor(status));
  const badge = $derived(badgeFor(status));

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
</script>

<SettingGroup
  id="settings-section-license"
  title="License & Trial"
  hint="Buy once, keep it forever. Everything is verified offline — no account, no phoning home."
>
  <SettingRow label="Status" description="Your current trial or license state." full>
    {#snippet control()}
      <div class="license-status">
        {#if badge}
          <span class="badge badge--{badge.variant} badge--sm">{badge.label}</span>
        {/if}
        {#if !status}
          <p class="group-hint">Checking license status…</p>
        {:else if status.kind === "trialNotStarted"}
          <p class="group-hint">
            Free trial — starts when you first record ({days(status.trialDays)}).
          </p>
        {:else if status.kind === "trial"}
          <p class="license-status__lead">
            {days(status.daysLeft)} left <span class="license-status__muted">· ends {fmtDate(status.trialEndMs)}</span>
          </p>
          <p class="group-hint">
            At expiry the app switches to Read-Only Mode — your recorded history stays fully
            searchable, only new recording pauses until you buy.
          </p>
        {:else if status.kind === "readOnly"}
          <p class="group-hint group-hint--warn">
            Trial ended — Read-Only Mode. Recorded history stays searchable; buy to resume
            recording.
          </p>
        {:else if status.kind === "revoked"}
          <p class="group-hint group-hint--warn">
            This license has been revoked — Read-Only Mode. Your recorded history stays fully
            searchable; new recording is paused.
          </p>
        {:else if status.kind === "licensed"}
          <p class="license-status__lead">Licensed to {status.name || status.email}</p>
          {#if status.activation.state === "pending"}
            <p class="group-hint">
              Finishing activation… ({days(status.activation.provisionalDaysLeft)} to connect).
            </p>
          {:else if status.activation.state === "refusedOverCap"}
            {@const act = status.activation}
            <p class="group-hint group-hint--warn">
              This license is already active on its 3 devices. Reset your devices to move it here,
              or buy another license.
            </p>
            <div class="row-actions">
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={() => openExternal(act.resetUrl)}
              >
                Reset devices
                <span aria-hidden="true"><IconArrowUpRight /></span>
              </button>
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={() => openExternal(act.buyUrl)}
              >
                Buy another license
                <span aria-hidden="true"><IconArrowUpRight /></span>
              </button>
            </div>
          {:else if status.activation.state === "lapsed"}
            <p class="group-hint group-hint--warn">
              We couldn't confirm your license — connect to the internet once to finish activation.
              Your recorded history stays fully searchable; new recording is paused until activation
              completes.
            </p>
          {/if}
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
          oninput={onKeyInput}
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
        {:else if activated}
          <p class="success-text" role="status">
            <span class="success-text__icon" aria-hidden="true"><IconCheck /></span>
            License activated — recording unlocked on this device.
          </p>
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
    align-items: flex-start;
  }

  .license-status p {
    margin: 0;
  }

  /* The one-line state summary — carries slightly more weight than the muted
     explanation beneath it so "12 days left" / "Licensed to …" reads first. */
  .license-status__lead {
    font-size: var(--text-sm);
    color: var(--app-text);
  }

  .license-status__muted {
    color: var(--app-text-muted);
  }

  /* Success confirmation for activation — mirrors `.error-text`'s placement but
     in the affirmative accent, and slides up so the action is unmistakably
     acknowledged (STATES + MICRO-INTERACTIONS). */
  .success-text {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-accent);
    animation: license-activated 180ms ease-out;
  }

  .success-text__icon {
    display: inline-flex;
    width: 15px;
    height: 15px;
  }

  @keyframes license-activated {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .success-text {
      animation: none;
    }
  }
</style>
