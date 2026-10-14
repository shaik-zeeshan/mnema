<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import { formatLastDerived, distillationWithheldLine } from "$lib/settings/state/user-context.svelte";
  import { POLICY_DAYS } from "$lib/insights/retention";
  import type { DerivationBudgetTier } from "$lib/types";
  import IconCheck from "~icons/lucide/check";
  import IconInfo from "~icons/lucide/info";
  import IconAlert from "~icons/lucide/triangle-alert";

  const c = getSettingsController();
  const rec = c.rec;
  const userContext = c.userContext;

  const st = $derived(userContext.userContextStatus);
  const running = $derived(userContext.userContextRunNowRunning);
  const withheld = $derived(distillationWithheldLine(st?.lastDistillation));
  const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
  const counts = $derived(
    st
      ? `${st.activityCount} ${st.activityCount === 1 ? "activity" : "activities"} · ${st.conclusionCount} ${
          st.conclusionCount === 1 ? "conclusion" : "conclusions"
        } · last run ${formatLastDerived(st.lastDerivedAtMs)} · ≈ ${compact.format(st.tokenUsage.totalTokens)} tokens so far (estimated)`
      : "Loading…",
  );
  const paused = $derived(!!st && !st.engineAvailable && !st.backfilling);
  const upToDate = $derived(!!st && st.engineAvailable && !st.backfilling && st.activityCount > 0);
  const hasLines = $derived(
    paused || upToDate || !!withheld || !!userContext.userContextStatusError || !!userContext.userContextRunNowMessage,
  );

  // The store's `refreshUserContext()` is a bare status reload with no in-flight
  // flag, so feed a local one to the button's `busy` (spins AND disables).
  let refreshing = $state(false);
  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      await userContext.refreshUserContext();
    } finally {
      refreshing = false;
    }
  }

  // Backfill reaches `backfillWindowDays` back (worker.rs `backfill_floor_ms`),
  // but nothing older than the retention window survives to be derived from.
  const keepDays = $derived(POLICY_DAYS[rec.draftRetentionPolicy] ?? Infinity);
  const reachDays = $derived(Math.min(rec.draftUserContextBackfillWindowDays, keepDays));
  const historyReach = $derived(
    rec.draftUserContextBackfillGoDeeper
      ? keepDays === Infinity
        ? "Reaches back to your first capture."
        : `Reaches back about ${keepDays} days — everything you keep.`
      : `Reaches back about ${reachDays} ${reachDays === 1 ? "day" : "days"}. Turn on to include everything you keep.`,
  );

  const budgetGate = $derived(
    c.userContextCloudDefault ? null : c.userContextLocalDefault ? "Local models use fixed pacing." : "Choose a cloud default model first.",
  );
</script>

{#snippet runActions()}
  <ReloadButton onclick={refresh} busy={refreshing} title="Refresh" label="Refresh derivation status" />
  <button
    class="mx-btn"
    type="button"
    disabled={running || !st?.engineAvailable}
    aria-busy={running}
    onclick={() => userContext.runUserContextDerivationNow()}
  >
    {#if running}<span class="mx-spin mx-spin--sm"></span>Deriving…{:else}Derive latest now{/if}
  </button>
{/snippet}

{#snippet statusLines()}
  <div class="uc-lines" aria-live="polite">
    {#if paused}
      <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" aria-hidden="true" />{c.aiRuntime.aiRuntimeReasonLabel(st?.reason)}</span>
    {:else if upToDate}
      <span class="mx-inline" data-tone="ok"><IconCheck width="13" height="13" aria-hidden="true" />Up to date for the covered window.</span>
    {/if}
    {#if withheld}<span class="mx-inline">{withheld}</span>{/if}
    {#if userContext.userContextRunNowMessage}<span class="mx-inline">{userContext.userContextRunNowMessage}</span>{/if}
    {#if userContext.userContextStatusError}
      <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" aria-hidden="true" />{userContext.userContextStatusError}</span>
    {/if}
  </div>
{/snippet}

{#snippet budget()}
  <Segmented
    md
    value={rec.draftUserContextBudgetTier}
    onValueChange={(value) => (rec.draftUserContextBudgetTier = value as DerivationBudgetTier)}
    disabled={!c.userContextCloudDefault}
    ariaLabel="Derivation budget intensity"
    options={[
      { value: "light", label: "Light" },
      { value: "balanced", label: "Balanced" },
      { value: "thorough", label: "Thorough" },
    ]}
  />
{/snippet}

{#snippet budgetGateLine()}
  <span class="mx-inline"><IconInfo width="13" height="13" aria-hidden="true" />{budgetGate}</span>
{/snippet}

<SettingGroup id="settings-section-userContext" title="User Context">
  <SettingRow
    label="Derive context continuously"
    description="Builds a quiet picture of your projects and people from captured activity."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftUserContextEnabled} ariaLabel="Derive context continuously" />
    {/snippet}
  </SettingRow>

  {#if st?.backfilling}
    <SettingRow
      label="Deriving activities"
      description="Building your understanding — deriving from your history in the background, newest first."
      full
      below={hasLines ? statusLines : undefined}
    >
      {#snippet aside()}{@render runActions()}{/snippet}
      {#snippet control()}
        <div class="uc-progress">
          <div class="mx-progress__meta"><b>Backfilling</b><span>{st.activityCount} activities so far</span></div>
          <div class="mx-progress mx-progress--indeterminate" role="progressbar" aria-label="Backfilling"><i></i></div>
        </div>
      {/snippet}
    </SettingRow>
  {:else}
    <SettingRow
      label={paused ? "Derivation paused" : "Deriving activities"}
      description={counts}
      control={runActions}
      below={hasLines ? statusLines : undefined}
    />
  {/if}

  <SettingRow
    label="Derivation Budget"
    description="Paces background work so tokens are spent as a trickle, never a one-time bill. Applies to a cloud default model."
    control={budget}
    below={budgetGate ? budgetGateLine : undefined}
  />

  <SettingRow
    label="Derive all of history"
    description={historyReach}
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftUserContextBackfillGoDeeper} ariaLabel="Go deeper — derive all of history" />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Wipe User Context"
    description="Permanently clears everything derived about you — it outlives capture retention. Captures are untouched. Asks first."
  >
    {#snippet control()}
      <button
        class="mx-btn mx-btn--danger"
        type="button"
        disabled={userContext.userContextWiping}
        aria-busy={userContext.userContextWiping}
        onclick={() => userContext.wipeUserContext()}
      >
        {#if userContext.userContextWiping}<span class="mx-spin mx-spin--sm"></span>Wiping…{:else}Wipe…{/if}
      </button>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .uc-lines {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }

  .uc-progress {
    width: 100%;
  }
</style>
