<script lang="ts">
  // Context's side column. It LOOKS like a rail, and in this direction a rail is
  // the one thing that wears material — so this one deliberately does not: it
  // carries explanation and beliefs, which is content, so it is opaque plates
  // like everything else.
  //
  // Three plates: the two layers (authored vs inferred), the conclusions your
  // context is currently steering, and the guardrail. The steering card is
  // READ-ONLY — no chevron, no pin, no dismiss: those belong to a conclusion and
  // conclusions are corrected on Subjects.
  import type { Conclusion } from "$lib/types/recording";

  interface Props {
    /** Top-confidence VISIBLE conclusions, already sliced by the parent. */
    steering: Conclusion[];
  }

  let { steering }: Props = $props();
</script>

<aside class="side" aria-label="How Mnema uses this">
  <div class="plate rcard">
    <div class="card__h"><span class="t-label">How Mnema uses this</span></div>
    <div class="urow">
      <span class="g g-auth" aria-hidden="true">✎</span>
      <span class="ubody">
        <span class="uhead">Authored <i>· this page</i></span>
        <p>You asserted it. It steers your dossier up front and stays as written.</p>
        <span class="mark mark--auth">never fades</span>
      </span>
    </div>
    <div class="urow">
      <span class="g g-inf" aria-hidden="true">◆</span>
      <span class="ubody">
        <span class="uhead">Inferred <i>· your dossier</i></span>
        <p>Mnema concluded it from your activity. Confidence rises and fades over time.</p>
        <span class="mark">confidence rises &amp; fades</span>
      </span>
    </div>
  </div>

  <div class="plate rcard">
    <div class="card__h"><span class="t-label">Steering your dossier</span></div>
    {#if steering.length > 0}
      {#each steering as c (c.id)}
        <div class="steer">
          <div class="l">
            <span class="qchip">{c.subject}</span>
            <span class="t-meta subtle">supports</span>
            <span class="t-meta is-mono is-num conf">{Math.round(c.confidence * 100)}%</span>
          </div>
          <span class="st"><i aria-hidden="true">◆</i>{c.statement}</span>
        </div>
      {/each}
    {:else}
      <p class="quiet">
        As Mnema forms inferred conclusions, you'll see the ones your context is steering
        here.
      </p>
    {/if}
  </div>

  <div class="plate rcard guard">
    <span class="s" aria-hidden="true">
      <svg viewBox="0 0 24 24"><path d="M12 21.5s7.5-3.8 7.5-9.5V5.2L12 2.5 4.5 5.2V12c0 5.7 7.5 9.5 7.5 9.5z" /></svg>
    </span>
    <span>
      <span class="uhead">Sensitive Category Guardrail</span>
      <p>
        Health, politics, sexuality, religion, and similar are never inferred or surfaced —
        even if you mention them here. Mnema errs toward over-suppression.
      </p>
    </span>
  </div>
</aside>

<style>
  .side {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .rcard {
    border-radius: var(--r-panel);
    padding: 10px 12px 11px;
  }
  .card__h {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 7px;
  }
  .uhead {
    font: var(--w-semi) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .uhead i {
    font-style: normal;
    font-weight: var(--w-regular);
    color: var(--app-text-subtle);
  }
  .subtle {
    color: var(--app-text-subtle);
  }

  /* the two layers */
  .urow {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 6px 0;
  }
  .urow + .urow {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .urow .g {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    font: var(--w-medium) 11px / 1 var(--app-font-sans);
  }
  .g-auth {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }
  .g-inf {
    background: var(--app-info-bg);
    color: var(--app-info);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-info-border);
  }
  .ubody {
    flex: 1;
    min-width: 0;
  }
  .urow p {
    margin: 2px 0 0;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .mark {
    display: block;
    margin-top: 5px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .mark--auth {
    color: var(--app-accent);
  }

  /* steering — read-only, and it never claims a per-row causal link */
  .steer {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 0;
  }
  .steer + .steer {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .steer .l {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .steer .conf {
    margin-left: auto;
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }
  .steer .st {
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text);
  }
  .steer .st i {
    font-style: normal;
    margin-right: 5px;
    color: var(--app-info);
  }
  .qchip {
    display: inline-flex;
    align-items: center;
    height: 19px;
    padding: 0 8px;
    min-width: 0;
    border-radius: var(--r-pill);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .quiet {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }

  /* the guardrail */
  .guard {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }
  .guard .s {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--app-warn-bg);
    color: var(--app-warn);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-warn-border);
  }
  .guard .s svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linejoin: round;
  }
  .guard p {
    margin: 3px 0 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }
</style>
