<script lang="ts">
  // The Context rail — a PRINT, not a panel of controls. Nothing here turns,
  // so it carries no instrument and it does not scroll with the list: it stays
  // put while the statements run past the fold.
  //
  // `steering` rows are honest or absent. There is no stored authored →
  // conclusion edge in the backend, so a row claims a link only when the parent
  // could derive one (an authored statement that names the subject). When it
  // could not, `fallback` carries the subjects the dossier holds on its own and
  // the copy stops claiming your statements steered them.
  interface SteerRow {
    subject: string;
    confidence: number;
  }

  const { steering, fallback }: { steering: SteerRow[]; fallback: SteerRow[] } =
    $props();

  const pct = (c: number): string => `${Math.round(c * 100)}%`;
</script>

<aside class="rail" aria-label="How Mnema uses this">
  <div class="rcard">
    <span class="t-label">How Mnema uses this</span>
    <div class="rrow">
      <span class="rglyph" aria-hidden="true">✎</span>
      <span>
        <span class="t-ui strong">Authored</span>
        <span class="t-meta">· this page</span>
        <p class="t-meta rrow__d">
          You asserted it. It steers your dossier up front and stays as written.
        </p>
        <span class="mark"><span class="mark__bar"></span>never fades</span>
      </span>
    </div>
    <div class="rrow">
      <span class="rglyph rglyph--muted" aria-hidden="true">◆</span>
      <span>
        <span class="t-ui strong">Inferred</span>
        <span class="t-meta">· your dossier</span>
        <p class="t-meta rrow__d">
          Mnema concluded it from your activity. Confidence rises and fades over
          time.
        </p>
        <span class="mark"
          ><span class="mark__meter"><i></i></span>confidence rises &amp; fades</span
        >
      </span>
    </div>
  </div>

  <div class="rcard">
    <span class="t-label">Steering your dossier</span>
    {#if steering.length > 0}
      {#each steering as row (row.subject)}
        <div class="steer">
          <span class="ti-chip ti-chip--acc steer__s">✎ {row.subject}</span>
          <span class="t-meta subtle">supports</span>
          <span class="steer__c is-num">{pct(row.confidence)}</span>
        </div>
      {/each}
      <p class="t-meta subtle steer__note">
        Read-only — a steering row shows that an authored statement is in play, not
        which frame proved anything.
      </p>
    {:else if fallback.length > 0}
      {#each fallback as row (row.subject)}
        <div class="steer">
          <span class="ti-chip steer__s">◆ {row.subject}</span>
          <span class="t-meta subtle">held at</span>
          <span class="steer__c is-num">{pct(row.confidence)}</span>
        </div>
      {/each}
      <p class="t-meta subtle steer__note">
        Read-only, and none of your statements names one of these yet — so these are
        the subjects your dossier holds on its own, not ones you steered.
      </p>
    {:else}
      <p class="t-meta steer__note">
        No steering yet. As Mnema forms inferred conclusions, you'll see how your
        authored context steers them here.
      </p>
    {/if}
  </div>

  <div class="rcard rcard--print">
    <span class="rcard__h">
      <span class="rglyph rglyph--muted" aria-hidden="true">⊘</span>
      <span class="t-label">Sensitive Category Guardrail</span>
    </span>
    <p class="t-meta rcard__p">
      Health, politics, sexuality, religion and similar are never inferred or
      surfaced — even if you mention them here. Mnema errs toward over-suppression.
    </p>
  </div>
</aside>

<style>
  .rail {
    flex: 0 0 268px;
    display: flex;
    flex-direction: column;
    gap: var(--s-12);
    overflow-y: auto;
    padding-bottom: var(--s-16);
  }
  .rcard {
    background: var(--ti-grp-fill);
    border-radius: var(--r-lg);
    padding: var(--s-12);
    flex: 0 0 auto;
  }
  .rcard--print {
    background: var(--app-surface-subtle);
  }
  .rcard__h {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }
  .rcard__p {
    margin: var(--s-6) 0 0;
  }
  .rrow {
    display: flex;
    gap: var(--s-8);
    padding: var(--s-8) 0;
    position: relative;
  }
  .rrow + .rrow::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: var(--hairline);
    background: var(--app-border);
  }
  .rrow__d {
    margin: 3px 0 0;
  }
  .rglyph {
    width: 18px;
    flex: 0 0 auto;
    text-align: center;
    color: var(--app-accent);
  }
  .rglyph--muted {
    color: var(--app-text-muted);
  }
  .mark {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-top: 5px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .mark__bar {
    width: 34px;
    height: 4px;
    border-radius: 2px;
    background: var(--app-accent);
  }
  .mark__meter {
    width: 34px;
    height: 4px;
    border-radius: 2px;
    overflow: hidden;
    background: var(--ti-track);
  }
  .mark__meter i {
    display: block;
    height: 100%;
    width: 60%;
    background: linear-gradient(
      90deg,
      var(--app-accent),
      color-mix(in srgb, var(--app-accent) 20%, transparent)
    );
  }
  .steer {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    padding: var(--s-6) 0;
    position: relative;
  }
  .steer + .steer::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: var(--hairline);
    background: var(--app-border);
  }
  .steer__s {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    display: inline-block;
    line-height: 22px;
  }
  .steer__c {
    margin-left: auto;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-muted);
  }
  .steer__note {
    margin: var(--s-8) 0 0;
  }
</style>
