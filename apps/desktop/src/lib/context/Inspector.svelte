<script lang="ts">
  // "How Mnema uses this" — the 256px panel, and the reason this page exists.
  //
  // The whole design is one distinction: what you ASSERTED versus what Mnema
  // CONCLUDED. So the panel leads with both cards and marks which one the thing
  // you are looking at is. Under it: what your writing is steering, the sensitive
  // category guardrail, and the honest map of what each delete control erases.
  //
  // While a row is being edited the panel becomes that row's record instead —
  // including the two fields authored context deliberately does NOT have.
  import IconLayers from "~icons/lucide/layers";
  import { relativeTime, type ContextData } from "./context-data.svelte";

  interface Props {
    data: ContextData;
  }

  let { data }: Props = $props();

  const editing = $derived(data.focus.kind === "editing" ? data.focus.item : null);
  // A dismissed row is the one thing on this page that is inferred.
  const isInferred = $derived(data.focus.kind === "dismissed");

  function truncate(text: string, max = 64): string {
    return text.length <= max ? text : `${text.slice(0, max).trimEnd()}…`;
  }
</script>

<aside class="ss-insp" aria-label="How Mnema uses this">
  <div class="ss-insp__h">
    <span class="ic" aria-hidden="true"><IconLayers /></span>
    <span>How Mnema uses this</span>
  </div>
  <div class="ss-insp__b">
    {#if editing}
      <div class="ss-insp__sec"><span>Editing</span></div>
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Statement</span>
        <span class="ss-kv__v">{truncate(editing.text)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Topic</span>
        <span class="ss-kv__v is-mono">{editing.topic ?? "none"}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Added</span>
        <span class="ss-kv__v is-mono">{relativeTime(editing.createdAtMs)}</span>
      </div>
      <!-- Both absent by construction, and named so the absence reads as a
           design fact rather than a missing read. -->
      <div class="ss-kv">
        <span class="ss-kv__k">Confidence</span>
        <span class="ss-kv__v dim">none — authored context carries no confidence</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Evidence</span>
        <span class="ss-kv__v dim">none — you asserted it, so there is nothing to ground</span>
      </div>

      <div class="ss-insp__sec"><span>Restore, precisely</span></div>
      <div class="prose">
        <p class="ss-conseq">
          Restoring clears the dismissal so the belief is allowed to form again. If your activity no
          longer supports it, nothing comes back — and that is the correct outcome, not a failure.
        </p>
      </div>

      <div class="ss-insp__sec"><span>Guardrail</span></div>
      <div class="guard">
        <p class="t-meta">
          Health, politics, sexuality, religion and similar are never inferred or surfaced — even if
          you mention them here.
        </p>
      </div>
    {:else}
      <div class="vsblock">
        <div class="vs" class:is-this={!isInferred}>
          <div class="vs__h">
            <span class="t-ui strong">Authored</span><span class="t-label dim">· this page</span>
          </div>
          <p class="t-meta">You asserted it. It steers your dossier up front and stays as written.</p>
          <span class="t-label is-mono acc">never fades</span>
        </div>
        <div class="vs" class:is-this={isInferred}>
          <div class="vs__h">
            <span class="t-ui strong">Inferred</span><span class="t-label dim">· Subjects</span>
          </div>
          <p class="t-meta">
            Mnema concluded it from your activity. Confidence rises and fades over time.
          </p>
          <span class="t-label is-mono dim">confidence rises &amp; fades</span>
        </div>
      </div>

      <!-- Rendered only when an authored topic actually shares a subject with a
           live belief. Nothing in the schema records which sentence shaped which
           conclusion, so the footnote names the join rather than implying one. -->
      {#if data.steerLinks.length > 0}
        <div class="ss-insp__sec"><span>Steering your dossier</span></div>
        <div class="steer">
          {#each data.steerLinks as link (link.topic)}
            <div>
              <p class="t-meta"><span class="is-mono acc">✎ {link.topic}</span> shares a subject with</p>
              <p class="t-meta belief">
                ◆ {link.statement}
                <span class="is-mono dim">{Math.round(link.confidence * 100)}%</span>
              </p>
            </div>
          {/each}
          <p class="t-meta note">
            Matched by subject — Mnema does not record which line shaped which belief.
          </p>
        </div>
      {/if}

      <div class="ss-insp__sec"><span>Sensitive category guardrail</span></div>
      <div class="guard">
        <p class="t-meta">
          Health, politics, sexuality, religion, and similar are never inferred or surfaced — even if
          you mention them here. Mnema errs toward over-suppression.
        </p>
      </div>

      {#if data.showDismissed}
        <div class="ss-insp__sec"><span>Restore, precisely</span></div>
        <div class="prose">
          <p class="ss-conseq">
            Restoring clears the dismissal so the belief is allowed to form again. If your activity
            no longer supports it, nothing comes back — and that is the correct outcome, not a
            failure.
          </p>
        </div>
      {/if}

      <div class="ss-insp__sec"><span>What deletes what</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">Delete</span>
        <span class="ss-kv__v">Removes that one sentence, immediately</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Retention</span>
        <span class="ss-kv__v">Never touches this page — it deletes recordings, not conclusions</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Wipe</span>
        <span class="ss-kv__v"
          >Settings › Intelligence › Wipe User Context clears everything derived, and this ledger
          with it</span
        >
      </div>
    {/if}
  </div>
</aside>

<style>
  .ic {
    display: flex;
    font-size: 11px;
  }

  .vsblock {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding: var(--s-6) var(--s-10) 0;
  }

  .vs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 7px var(--s-8);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
  }

  .vs.is-this {
    background: var(--app-accent-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }

  .vs__h {
    display: flex;
    align-items: baseline;
    gap: var(--s-6);
  }

  .vs p,
  .steer p {
    margin: 0;
  }

  .strong {
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }

  .dim {
    color: var(--app-text-subtle);
  }

  .acc {
    color: var(--app-accent-strong);
  }

  .steer {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding: 2px var(--s-10) 0;
  }

  .belief {
    color: var(--app-text-strong);
    margin-top: 1px;
  }

  .note {
    color: var(--app-text-subtle);
  }

  .guard {
    margin: var(--s-8) var(--s-10) 0;
    padding: var(--s-8) 9px;
    border-radius: var(--r-md);
    background: var(--app-warn-bg);
    border: var(--hairline) solid var(--app-warn-border);
  }

  .guard p {
    margin: 0;
    color: var(--app-warn-strong);
  }

  .prose {
    padding: 4px var(--s-10) 0;
  }

  .prose p {
    margin: 0;
  }
</style>
