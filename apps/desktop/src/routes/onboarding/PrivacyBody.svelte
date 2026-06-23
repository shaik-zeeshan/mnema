<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // The privacy controller instance + draft list are owned by the onboarding
  // controller (constructed exactly as the legacy onboarding page did). Its
  // `onSettingsUpdated` callback re-syncs `draftExcludedApps` after every
  // add/remove/recommend command, so binding to `controller.draftExcludedApps`
  // stays authoritative without any extra wiring here. We read
  // `controller.appPrivacyExclusion` reactively (it is a stable readonly
  // instance, but referencing it through `controller` keeps Svelte happy).
  const privacy = $derived(controller.appPrivacyExclusion);
</script>

<div class="group">
  <div class="group-title">Excluded apps</div>
  <p class="note">
    Mnema records visible content from non-excluded apps, including
    private/incognito browser windows. Exclude apps you never want captured —
    password managers, messaging, and the like.
  </p>

  <AppPrivacyExclusion
    controller={privacy}
    comboboxListId="onboarding-privacy-app-combobox-list"
  />
</div>

{#if privacy.pendingRecommendedApps.length > 0}
  <div class="ctl">
    <div class="count">
      <b>{controller.draftExcludedApps.length}</b>
      {controller.draftExcludedApps.length === 1 ? "app" : "apps"} excluded
    </div>
    <button
      class="btn sm"
      type="button"
      disabled={privacy.commandInFlight}
      onclick={() => void privacy.applyAllRecommendedPrivacyApps()}
    >
      Apply recommended
    </button>
  </div>
{/if}

{#if controller.errorMessage}
  <div class="privacy-err" role="alert">{controller.errorMessage}</div>
{/if}

<style>
  /* Surfaces a failed exclusion add/remove/recommend — the privacy controller
     routes those failures to `controller.errorMessage`. Without this, a failed
     "Apply recommended" silently leaves apps un-excluded (a privacy regression).
     Terminal/green danger tokens. */
  .privacy-err {
    margin-top: 10px;
    padding: 10px 14px;
    font-size: 11px;
    line-height: 1.5;
    color: var(--app-danger);
    background: var(--app-danger-bg);
    border: 1px solid var(--app-danger-border);
    border-radius: 8px;
  }
</style>
