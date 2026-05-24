<script lang="ts">
  // Reframes the thin stepper rail into the product's `capture -> index ->
  // recall` arc: each phase groups its subsystem bays, the active subsystem
  // lights beneath its phase. Preserves the rail's interaction contract —
  // click-back to completed bays, future bays disabled, `aria-current` on the
  // active bay, focus-visible ring, and a light-theme active override.
  type ArcStepState = "done" | "active" | "future";
  type ArcStep = { id: string; label: string; num: string; state: ArcStepState };
  type ArcPhase = { id: string; label: string; steps: ArcStep[] };

  let {
    phases,
    navDisabled = false,
    onNavigate,
  }: {
    phases: ArcPhase[];
    navDisabled?: boolean;
    onNavigate: (id: string) => void;
  } = $props();

  function phaseStateFor(phase: ArcPhase): ArcStepState {
    if (phase.steps.some((step) => step.state === "active")) return "active";
    if (phase.steps.every((step) => step.state === "done")) return "done";
    return "future";
  }
</script>

<nav class="arc" aria-label="Setup progress">
  {#each phases as phase, phaseIndex (phase.id)}
    {@const phaseState = phaseStateFor(phase)}
    <div class="arc__phase arc__phase--{phaseState}">
      <span class="arc__phase-label">{phase.label}</span>
      <div class="arc__subs">
        {#each phase.steps as step (step.id)}
          <button
            type="button"
            class="arc__sub arc__sub--{step.state}"
            disabled={navDisabled || step.state === "future"}
            aria-current={step.state === "active" ? "step" : undefined}
            title={step.label}
            onclick={() => { if (step.state !== "future") onNavigate(step.id); }}
          >
            <span class="arc__num">{step.num}</span>
            <span class="arc__lbl">{step.label}</span>
          </button>
        {/each}
      </div>
    </div>
    {#if phaseIndex < phases.length - 1}
      <span class="arc__link" class:arc__link--lit={phaseState === "done"} aria-hidden="true">→</span>
    {/if}
  {/each}
</nav>

<style>
  .arc {
    display: flex;
    align-items: stretch;
    gap: 6px;
    padding: 6px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 8px;
  }
  .arc__phase {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 3px 4px;
  }
  .arc__phase-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding-left: 3px;
    color: var(--app-text-faint);
    font-size: 8.5px;
    font-weight: 800;
    letter-spacing: 0.22em;
    text-transform: uppercase;
    transition: color 0.15s;
  }
  .arc__phase-label::before {
    content: "";
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.5;
  }
  .arc__phase--active .arc__phase-label,
  .arc__phase--done .arc__phase-label {
    color: var(--app-accent);
  }
  .arc__phase--active .arc__phase-label::before {
    opacity: 1;
    box-shadow: 0 0 5px var(--app-accent-glow);
  }

  .arc__subs {
    display: flex;
    gap: 3px;
  }
  .arc__sub {
    flex: 1 1 0;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 24px;
    padding: 4px 7px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .arc__sub:disabled {
    cursor: default;
  }
  .arc__sub--future {
    color: var(--app-text-faint);
  }
  .arc__sub--done {
    color: var(--app-text);
  }
  .arc__sub--done:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .arc__sub--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }
  .arc__num {
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }
  .arc__sub--active .arc__num {
    opacity: 1;
  }
  .arc__lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .arc__sub:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .arc__link {
    align-self: center;
    color: var(--app-text-faint);
    font-size: 11px;
    transition: color 0.15s;
  }
  .arc__link--lit {
    color: var(--app-accent);
    opacity: 0.7;
  }

  :global([data-theme="light"]) .arc__sub--active {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .arc__phase--active .arc__phase-label,
  :global([data-theme="light"]) .arc__phase--done .arc__phase-label {
    color: var(--app-accent-strong);
  }

  @media (max-width: 600px) {
    .arc__lbl {
      display: none;
    }
    .arc__phase-label {
      font-size: 8px;
      letter-spacing: 0.16em;
    }
  }
</style>
