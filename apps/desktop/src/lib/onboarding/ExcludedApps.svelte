<!--
  Onboarding "Never recorded" (issue #195, slice 7).

  Ported from `docs/onboarding/mockups/input-components/parts/excluded.part.html`
  — that mockup is the design of record; behaviour and copy come from it.

  THE SUMMARY LINE IS THE CONTROL. There is no editor, no disclosure and no edit
  mode: what the app decided on the user's behalf is visible without a click, and
  the names in the sentence are the buttons that change it.

  Three properties that are load-bearing rather than decorative:
   · A struck name KEEPS ITS SLOT. Striking is the existing
     `set_privacy_excluded_app_enabled(sourceId, false)`, so the entry stays in
     `PrivacySettings.excluded_apps` and the sentence never reflows. Nothing can
     be removed from here at all — `remove_privacy_excluded_app` is not wired.
   · The icon is a mark on the baseline, not a tile. Sized `1em` and dropped
     `-.16em`, so at `line-height: 2.05` its margin box is smaller than the strut
     and adding icons changes no line height and no wrap point. Glyph + name is
     one unbreakable word, so a break can only land on a comma.
   · A rule for an app that is not installed renders PENDING (dashed empty icon
     slot), never as protecting — `isPendingExclusion` /
     `activeExclusionBundleIds` are the frontend mirror of `evaluate_privacy`.

  Motion: none ambient. Colour/underline transitions only, dropped under
  prefers-reduced-motion.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { isPendingExclusion, type PrivacyAppCandidate } from "$lib/app-privacy-exclusion";
  import {
    addAnnouncement,
    entryLabel,
    excludedNote,
    excludedSentence,
    resolveTypedApp,
    separatorBefore,
    strikeAnnouncement,
  } from "$lib/onboarding/excluded-apps";
  import type { ExcludedAppEntry } from "$lib/types";

  /**
   * The privacy controller's surface, declared structurally so this component
   * doesn't reach into `routes/`. `createAppPrivacyExclusionController()`
   * satisfies it — the wiring is `<ExcludedApps privacy={c.appPrivacyExclusion} />`.
   */
  interface ExcludedAppsStore {
    /** Plain rows, in stored order. A struck row is `enabled: false`, still here. */
    excludedApps: ExcludedAppEntry[];
    /** Installed apps, for name resolution and the add field's suggestions. */
    candidates: PrivacyAppCandidate[];
    appIconSrcForBundleId: (bundleId: string) => string | null;
    appIconFallback: (displayName: string | null, bundleId: string | null) => string;
    setPrivacyExcludedAppEnabled: (sourceId: string, enabled: boolean) => void;
    /**
     * Adds, or re-enables a struck rule with the same identity. A candidate with
     * an empty `bundleId` is the "not installed yet" rule:
     * `add_privacy_excluded_app({ bundleId: "", displayName })`.
     */
    addPrivacyAppCandidate: (candidate: PrivacyAppCandidate | null) => void;
  }

  let { privacy }: { privacy: ExcludedAppsStore } = $props();

  const entries = $derived(privacy.excludedApps);
  const sentence = $derived(excludedSentence(entries));
  const note = $derived(excludedNote(entries));

  const listId = $props.id();
  let adding = $state(false);
  let announcement = $state("");
  let addInput = $state<HTMLInputElement | null>(null);
  let addChip = $state<HTMLButtonElement | null>(null);

  function strike(entry: ExcludedAppEntry) {
    const next = !entry.enabled;
    announcement = strikeAnnouncement(entry, next);
    privacy.setPrivacyExcludedAppEnabled(entry.id, next);
  }

  async function openAdd() {
    adding = true;
    await tick();
    addInput?.focus();
  }

  async function closeAdd() {
    adding = false;
    await tick();
    addChip?.focus();
  }

  function commitAdd(typed: string) {
    const name = typed.trim();
    if (!name) return;
    const installed = resolveTypedApp(name, privacy.candidates);
    announcement = addAnnouncement(installed?.displayName ?? name, Boolean(installed));
    // A name that matches nothing installed is still a real rule: stored with an
    // empty bundle id, held pending, resolved on first sighting.
    privacy.addPrivacyAppCandidate(
      installed ?? { id: "", enabled: true, bundleId: "", displayName: name, running: false, iconPath: null },
    );
  }

  function onAddKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      void closeAdd();
      return;
    }
    if (event.key !== "Enter") return;
    // Without this the SAME Enter keystroke activates the name button that lands
    // under focus, and the app you just added is struck out instantly. The mockup
    // shipped that bug; it is only visible by pressing the key.
    event.preventDefault();
    commitAdd(event.currentTarget instanceof HTMLInputElement ? event.currentTarget.value : "");
    void closeAdd();
  }
</script>

<span class="ob-m">Never recorded</span>
<!-- One paragraph: separators are plain text between the buttons, so nothing
     gaps before a comma. Kept on one line on purpose — a newline here becomes a
     space and the sentence stops hugging its punctuation. -->
<p class="sentence">{sentence.lead}{#each entries as entry, i (entry.id)}{separatorBefore(i, entries.length)}<button
      class="nm"
      type="button"
      aria-pressed={entry.enabled}
      data-waiting={isPendingExclusion(entry) ? "1" : "0"}
      aria-label="{entryLabel(entry)}{isPendingExclusion(entry) ? ', not installed yet' : ''} — {entry.enabled
        ? 'never recorded, activate to record it'
        : 'recorded, activate to hide it again'}"
      onclick={() => strike(entry)}
    >{#if isPendingExclusion(entry)}<span class="gl wait" aria-hidden="true"></span>{:else}{@const src =
        privacy.appIconSrcForBundleId(entry.bundleId)}<span class="gl" aria-hidden="true">{#if src}<img
            src={src}
            alt=""
            loading="lazy"
          />{:else}<span class="mono">{privacy.appIconFallback(entry.displayName, entry.bundleId)}</span>{/if}</span
      >{/if}<span class="t">{entryLabel(entry)}</span></button
    >{#if isPendingExclusion(entry)}<span class="waiting-note">&nbsp;· when you install it</span>{/if}{/each}{sentence.tail}
    {#if adding}<input
      bind:this={addInput}
      class="add-input"
      list={listId}
      autocomplete="off"
      spellcheck="false"
      placeholder="Type any app name"
      aria-label="Add an app Mnema should never record"
      onkeydown={onAddKeydown}
    />{:else}<button bind:this={addChip} class="add" type="button" onclick={openAdd}>＋ Add an app</button>{/if}</p>

<p class="note">{note.text} <em class="hint">{note.hint}</em></p>

<datalist id={listId}>
  {#each privacy.candidates as candidate (candidate.id)}
    <option value={candidate.displayName}></option>
  {/each}
</datalist>

<p class="live" aria-live="polite">{announcement}</p>

<style>
  /* line-height is fixed and generous: it is what keeps an inline 1em glyph from
     ever inflating a line box, so icons cost zero vertical space. */
  .sentence {
    margin: 5px 0 0;
    font-size: var(--t-ui);
    line-height: 2.05;
    color: var(--app-text-muted);
  }

  /* Vertical padding only — a horizontal pad would put a gap before every comma
     and the line would stop reading as a sentence. */
  .nm {
    font: inherit;
    margin: 0;
    padding: 1px 0;
    border: 0;
    border-radius: 3px;
    background: none;
    color: var(--app-text-strong);
    cursor: pointer;
    white-space: nowrap; /* glyph + name is one unbreakable word */
    transition:
      color 0.12s,
      background 0.12s;
  }
  /* The "these words are clickable" signifier underlines the NAME only. Run it
     under the glyph too and at eight apps the underlines merge into one ragged
     rule that reads as a table, not a sentence. */
  .t {
    text-decoration: underline dashed var(--app-border-hover);
    text-underline-offset: 3px;
    transition: text-decoration-color 0.12s;
  }
  .nm:hover {
    background: var(--app-surface-hover);
  }
  .nm:hover .t {
    text-decoration-color: var(--app-danger);
  }
  .nm:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  /* Struck = this app is recorded like everything else. Click puts it back. */
  .nm[aria-pressed="false"] {
    color: var(--app-text-subtle);
    text-decoration: line-through;
  }
  .nm[aria-pressed="false"] .t {
    text-decoration: none;
  }
  .nm[aria-pressed="false"] .gl {
    filter: grayscale(1);
    opacity: 0.45;
  }
  .nm[aria-pressed="false"]:hover {
    color: var(--app-text-muted);
  }

  /* A mark, not a tile: no border, no fill, no box. Sized to the type and nudged
     so its optical centre lands on the cap-height centre of the words. */
  .gl {
    display: inline-block;
    box-sizing: border-box;
    width: 1em;
    height: 1em;
    margin-right: 0.3em;
    vertical-align: -0.16em;
    line-height: 1;
    text-align: center;
  }
  .gl img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  /* No resolved icon: the app's own monogram fallback, filling the same 1em box
     so the sentence's rhythm is identical either way. */
  .gl .mono {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    /* Full 1em so the letter's cap height matches the words beside it — a
       smaller monogram reads as a subscript, not as the app's mark. */
    font-size: 1em;
    font-weight: 600;
    color: var(--app-text-muted);
  }
  /* Not installed yet: there is no icon to draw, so the icon slot IS the waiting
     signal — same box, dashed and empty, rather than an extra badge. */
  .gl.wait {
    border: 1px dashed var(--app-warn);
    border-radius: 3px;
  }
  .nm[data-waiting="1"] .t {
    text-decoration-color: var(--app-warn);
  }
  .waiting-note {
    color: var(--app-text-subtle);
    font-size: var(--t-label);
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .add {
    font: inherit;
    font-size: var(--t-meta);
    padding: 3px 10px;
    vertical-align: 1px;
    color: var(--app-text-subtle);
    background: transparent;
    border: 1px dashed var(--app-border-strong);
    border-radius: 999px;
    white-space: nowrap;
    cursor: pointer;
    transition:
      color 0.12s,
      border-color 0.12s;
  }
  .add:hover {
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }
  .add:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .add-input {
    width: 190px;
    padding: 3px 9px;
    font: inherit;
    font-size: var(--t-meta);
    vertical-align: 1px;
    color: var(--app-text);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-accent);
    border-radius: 999px;
    outline: none;
    box-shadow: var(--app-ring);
  }
  .add-input::placeholder {
    color: var(--app-text-faint);
  }

  .note {
    margin: 7px 0 0;
    font-size: var(--t-meta);
    line-height: 1.7;
    color: var(--app-text-subtle);
  }
  .note .hint {
    color: var(--app-text-muted);
    font-style: normal;
  }

  .live {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .nm,
    .t,
    .add {
      transition: none;
    }
  }
</style>
