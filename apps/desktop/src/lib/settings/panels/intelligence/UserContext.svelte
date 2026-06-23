<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import { formatLastDerived, distillationWithheldLine } from "$lib/settings/state/user-context.svelte";
  import type { DerivationBudgetTier } from "$lib/types";

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;
  const userContext = c.userContext;

  // Store-read aliases.
  const userContextStatus = $derived(userContext.userContextStatus);
  const userContextStatusError = $derived(userContext.userContextStatusError);
  const userContextRunNowRunning = $derived(userContext.userContextRunNowRunning);
  const userContextRunNowMessage = $derived(userContext.userContextRunNowMessage);
  const userContextWiping = $derived(userContext.userContextWiping);

  // Controller derived selectors.
  const userContextCloudDefault = $derived(c.userContextCloudDefault);
  const userContextLocalDefault = $derived(c.userContextLocalDefault);

  // Store action methods.
  const aiRuntimeReasonLabel = (reason: string | null | undefined) =>
    aiRuntime.aiRuntimeReasonLabel(reason);
  const refreshUserContext = () => userContext.refreshUserContext();
  const runUserContextDerivationNow = () => userContext.runUserContextDerivationNow();
  const wipeUserContext = () => userContext.wipeUserContext();
</script>

<SettingGroup
  id="settings-section-userContext"
  title="User Context"
  hint="A private, on-device understanding of your activity, derived continuously from your capture history by the default model. High-consent and off by default."
>
  <SettingRow
    label="Derive context continuously"
    description="Let Mnema build a private, on-device understanding of your activity by deriving from your capture history in the background, 24/7. Distinct from Ask AI — this is the high-consent continuous worker, off by default. Needs a provider and default model configured above."
    full
  >
    {#snippet control()}
      <div class="uc-stack">
        <Switch bind:checked={rec.draftUserContextEnabled} />
        <div class="privacy-disclosure">
          <p>While on, the default model runs over your redacted screen text and transcripts as a background trickle to derive Activities and Conclusions. With a cloud default that means continuous outbound egress billed to your key; a local default keeps everything on this machine.</p>
          <p>The derived understanding deliberately outlives raw-capture retention. Turning this off pauses derivation; it does not erase what was already learned — use Wipe User Context below for that.</p>
        </div>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Derivation status" full>
    {#snippet control()}
      <div class="uc-stack">
        <div
          class="model-status"
          class:model-status--available={userContextStatus?.engineAvailable}
        >
          <div>
            <div class="model-status__title">
              {userContextStatus?.engineAvailable ? "Deriving Activities" : "Derivation paused"}
            </div>
            <div class="model-status__meta">
              {#if userContextStatus}
                {userContextStatus.activityCount}
                {userContextStatus.activityCount === 1 ? "Activity" : "Activities"} ·
                {userContextStatus.conclusionCount}
                {userContextStatus.conclusionCount === 1 ? "Conclusion" : "Conclusions"} ·
                last run {formatLastDerived(userContextStatus.lastDerivedAtMs)}
                {#if !userContextStatus.engineAvailable}
                  · {aiRuntimeReasonLabel(userContextStatus.reason)}
                {/if}
              {:else}
                Loading…
              {/if}
            </div>
          </div>
          <span class="model-status__pill">
            {userContextStatus?.engineAvailable ? "active" : "paused"}
          </span>
        </div>

        {#if userContextStatus?.backfilling}
          <p class="group-hint" aria-live="polite">
            Building your understanding… deriving from your history in the background.
          </p>
        {:else if userContextStatus && userContextStatus.activityCount > 0}
          <p class="group-hint">
            Your understanding is up to date for the covered window.
          </p>
        {/if}

        {#if userContextStatus}
          <p class="group-hint">
            ≈ {userContextStatus.tokenUsage.totalTokens.toLocaleString()} tokens used,
            cumulative across {userContextStatus.tokenUsage.runCount}
            derivation {userContextStatus.tokenUsage.runCount === 1 ? "pass" : "passes"}
            (estimated from text length, not a billed count).
          </p>
        {/if}

        {#if distillationWithheldLine(userContextStatus?.lastDistillation)}
          <p class="group-hint">
            {distillationWithheldLine(userContextStatus?.lastDistillation)}
          </p>
        {/if}

        {#if userContextStatusError}
          <p class="error-text">{userContextStatusError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Derivation Budget"
    description="Paces background work over time so tokens are spent as a trickle, never a one-time bill. A higher tier covers more of your history per pass."
    full
  >
    {#snippet control()}
      <div class="uc-stack">
        <RadioGroup
          value={rec.draftUserContextBudgetTier}
          onValueChange={(value) =>
            (rec.draftUserContextBudgetTier = value as DerivationBudgetTier)}
          disabled={!userContextCloudDefault}
          label="Intensity"
          options={[
            {
              value: "light",
              label: "Light",
              description: "Slowest pacing, fewest tokens. Understanding fills in gradually.",
            },
            {
              value: "balanced",
              label: "Balanced",
              description: "Moderate pacing and token spend. A sensible default.",
            },
            {
              value: "thorough",
              label: "Thorough",
              description: "Fastest pacing, most tokens. Covers your history sooner.",
            },
          ]}
        />
        {#if userContextLocalDefault}
          <p class="group-hint">
            Budget tiers apply to a cloud default model. A local default uses fixed
            background pacing — no token spend, so there is no intensity to choose.
          </p>
        {:else if !userContextCloudDefault}
          <p class="group-hint">
            Set a default model above to choose an intensity. Budget tiers pace
            token spend for a cloud default — until a default model is configured
            there is nothing to pace.
          </p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="History Backfill" full>
    {#snippet control()}
      <div class="uc-stack">
        <p class="group-hint">
          Newest history is derived first. By default Mnema reaches back about
          {rec.draftUserContextBackfillWindowDays}
          {rec.draftUserContextBackfillWindowDays === 1 ? "day" : "days"}; recent activity
          drives your current understanding.
        </p>
        <Switch
          bind:checked={rec.draftUserContextBackfillGoDeeper}
          label="Go deeper — derive all of history"
          description="Extend backfill past the recent window to your entire history. Increases token spend over time (still a background trickle, not a one-time bill)."
        />
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Run derivation" full divider={false}>
    {#snippet control()}
      <div class="uc-stack">
        <div class="row-actions">
          <button
            class="btn btn--ghost btn--sm"
            type="button"
            disabled={userContextRunNowRunning || !userContextStatus?.engineAvailable}
            onclick={runUserContextDerivationNow}
          >
            {userContextRunNowRunning ? "Deriving" : "Run derivation now"}
          </button>
          <button
            class="btn btn--ghost btn--sm"
            type="button"
            onclick={refreshUserContext}
          >
            Refresh
          </button>
        </div>

        {#if userContextRunNowMessage}
          <p class="group-hint" aria-live="polite">{userContextRunNowMessage}</p>
        {/if}

        <div class="user-context-wipe">
          <p class="group-hint">
            This derived understanding deliberately outlives your raw-capture
            Retention Policy window — Mnema can keep what it learned about you
            long after the recordings it learned from have aged out. Wipe User
            Context is the only control that clears it.
          </p>
          <div class="row-actions">
            <button
              class="btn btn--ghost btn--sm user-context-wipe__btn"
              type="button"
              disabled={userContextWiping}
              onclick={wipeUserContext}
            >
              {userContextWiping ? "Wiping…" : "Wipe User Context"}
            </button>
          </div>
        </div>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Full-width rows stack a control over its disclosures, status, and action
     sub-blocks; the primitives only gap whole rows. */
  .uc-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
