<script lang="ts">
  // The Studio Shell's 256px right inspector, on the Settings surface.
  //
  // One panel, four subjects across the app; here the subject is the FOCUSED
  // SETTING — not navigation. The rail is deleted in this direction, and this
  // panel must never become its replacement: nothing in it scrolls the page.
  //
  // Every field is something Mnema genuinely holds. There is no per-setting
  // "previous value", "default", or "takes effect at" store behind Settings, so
  // those lines from the mockup are absent rather than invented (G8's rule,
  // applied to prose as well as to numbers). What IS real: the row's label and
  // description, its breadcrumb, the ⌘F terms that reach it, and the rows this
  // session has actually saved.

  import IconSliders from "~icons/lucide/sliders-horizontal";
  import { settingsInspector } from "../state/inspector.svelte";
  import { sectionBreadcrumb, rowIndexEntry } from "../settings-index";

  const subject = $derived(settingsInspector.subject);
  const crumb = $derived(subject?.section ? sectionBreadcrumb(subject.section) : null);
  const entry = $derived(subject ? rowIndexEntry(subject.section, subject.label) : null);
  const recent = $derived(settingsInspector.recent);

  function clock(atMs: number): string {
    return new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
</script>

<aside class="ss-insp" aria-label="Setting detail">
  <div class="ss-insp__h">
    <IconSliders aria-hidden="true" />
    <span>Detail</span>
    <span class="ss-insp__spacer"></span>
    <kbd class="kbd">⌥⌘I</kbd>
  </div>

  <div class="ss-insp__b">
    {#if subject}
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Setting</span>
        <span class="ss-kv__v ss-insp__name">{subject.label}</span>
      </div>
      {#if crumb}
        <div class="ss-kv">
          <span class="ss-kv__k">Section</span>
          <span class="ss-kv__v">{crumb.group} › {crumb.section}</span>
        </div>
      {/if}
      {#if subject.description}
        <div class="ss-kv ss-kv--stack">
          <span class="ss-kv__k">What it does</span>
          <span class="ss-kv__v">{subject.description}</span>
        </div>
      {/if}

      {#if entry?.synonyms?.length}
        <div class="ss-insp__sec"><span>Finds as</span></div>
        <div class="ss-kv ss-kv--stack">
          <span class="ss-kv__k">⌘F terms</span>
          <span class="ss-kv__v is-mono">{entry.synonyms.join(" · ")}</span>
        </div>
      {/if}
    {:else}
      <p class="ss-insp__empty">
        Nothing selected. Click or tab into a setting and its detail shows here —
        what it does, where it lives, and the words ⌘F finds it by.
      </p>
    {/if}

    <div class="ss-insp__sec"><span>Recent changes</span></div>
    {#if recent.length === 0}
      <p class="ss-insp__empty">Nothing saved yet this session.</p>
    {:else}
      {#each recent as change (change.label)}
        <div class="ss-kv">
          <span class="ss-kv__k">{clock(change.atMs)}</span>
          <span class="ss-kv__v">{change.label}</span>
        </div>
      {/each}
    {/if}
  </div>
</aside>

<style>
  /* Geometry is the skin's (`.ss-insp*`, `.ss-kv*`). Only the header's icon
     sizing and the one oversized subject line live here. */
  .ss-insp__h :global(svg) {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex: 0 0 auto;
  }

  .ss-insp__spacer {
    margin-left: auto;
  }

  /* The subject is the one thing in the panel worth reading at a glance. */
  .ss-insp__name {
    font-size: 15px;
    font-weight: 590;
    line-height: 1.3;
    letter-spacing: -0.01em;
  }
</style>
