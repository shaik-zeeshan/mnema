<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("userContext");

  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import StatusLine from "./StatusLine.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
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
  const runUserContextDerivationNow = () => userContext.runUserContextDerivationNow();
  const wipeUserContext = () => userContext.wipeUserContext();

  // The store's `refreshUserContext()` is a bare status reload with no in-flight
  // flag, so the ReloadButton would otherwise stay enabled and be double-fireable.
  // Track a local flag and feed it to the button's `busy` prop (which both spins
  // and disables) so rapid double-clicks are hard-prevented, not just debounced.
  // The status sub-line, composed once so the flat StatusLine takes a string
  // rather than a block of markup. Counts are real reads; "Loading…" is the
  // only stand-in, and only before the first read lands.
  const statusMeta = $derived.by(() => {
    const s = userContextStatus;
    if (!s) return "Loading…";
    const parts = [
      `${s.activityCount} ${s.activityCount === 1 ? "Activity" : "Activities"}`,
      `${s.conclusionCount} ${s.conclusionCount === 1 ? "Conclusion" : "Conclusions"}`,
      `last run ${formatLastDerived(s.lastDerivedAtMs)}`,
    ];
    if (!s.engineAvailable) parts.push(aiRuntimeReasonLabel(s.reason));
    return parts.join(" · ");
  });

  let refreshing = $state(false);
  async function refreshUserContext() {
    if (refreshing) return;
    refreshing = true;
    try {
      await userContext.refreshUserContext();
    } finally {
      refreshing = false;
    }
  }
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
    {#snippet aside()}
      <Switch bind:checked={rec.draftUserContextEnabled} ariaLabel="Derive context continuously" />
    {/snippet}
    {#snippet control()}
      <!-- Flat consent disclosure: same weight, no bordered card. -->
      <div class="disclosure">
        <p>While on, the default model runs over your redacted screen text and transcripts as a background trickle to derive Activities and Conclusions. With a cloud default that means continuous outbound egress billed to your key; a local default keeps everything on this machine.</p>
        <p>The derived understanding deliberately outlives raw-capture retention. Turning this off pauses derivation; it does not erase what was already learned — use Wipe User Context below for that.</p>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Derivation status" full>
    {#snippet control()}
      <div class="uc-stack">
        <StatusLine
          title={userContextStatus?.engineAvailable ? "Deriving Activities" : "Derivation paused"}
          meta={statusMeta}
          ok={!!userContextStatus?.engineAvailable}
          okLabel="active"
          offLabel="paused"
        />

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
        <Segmented
          value={rec.draftUserContextBudgetTier}
          onValueChange={(value) =>
            (rec.draftUserContextBudgetTier = value as DerivationBudgetTier)}
          disabled={!userContextCloudDefault}
          ariaLabel="Derivation budget intensity"
          options={[
            { value: "light", label: "Light" },
            { value: "balanced", label: "Balanced" },
            { value: "thorough", label: "Thorough" },
          ]}
        />
        {#if userContextCloudDefault}
          <p class="group-hint">
            Light — slowest pacing, fewest tokens; understanding fills in gradually.
            Balanced — moderate pacing and token spend, a sensible default.
            Thorough — fastest pacing, most tokens; covers your history sooner.
          </p>
        {/if}
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
            aria-busy={userContextRunNowRunning}
            onclick={runUserContextDerivationNow}
          >
            {#if userContextRunNowRunning}<ButtonSpinner />Deriving…{:else}Run derivation now{/if}
          </button>
          <ReloadButton
            onclick={refreshUserContext}
            busy={refreshing}
            title="Refresh"
            label="Refresh derivation status"
          />
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
              class="btn btn--danger btn--sm user-context-wipe__btn"
              type="button"
              disabled={userContextWiping}
              aria-busy={userContextWiping}
              onclick={wipeUserContext}
            >
              {#if userContextWiping}<ButtonSpinner />Wiping…{:else}Wipe User Context{/if}
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

  /* Flat consent disclosure — same copy, no bordered card. */
  .disclosure {
    display: grid;
    gap: var(--s-6);
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
  }

  .disclosure p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: var(--t-meta);
    line-height: 1.5;
  }
</style>
