<script lang="ts">
  // Subjects destination chrome (page 09): the "Search subjects…" field lives in
  // the title bar, not the content — the destination's one chrome control, on
  // the material the title bar already wears. The layout mounts this on
  // /subjects; the query is shared with the page through the slice's own store.
  import { subjectSearch } from "$lib/insights/subject-search-state.svelte";

  // Leaving /subjects unmounts this control — drop the query with it, so coming
  // back never lands on a filtered list with an empty-looking field.
  $effect(() => () => {
    subjectSearch.query = "";
  });
</script>

<label class="sbsearch">
  <svg
    class="sbsearch__glyph"
    width="12"
    height="12"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.8"
    stroke-linecap="round"
    aria-hidden="true"
  >
    <circle cx="11" cy="11" r="7.5" />
    <path d="m21 21-4.4-4.4" />
  </svg>
  <input
    type="search"
    class="sbsearch__input"
    placeholder="Search subjects…"
    aria-label="Search subjects"
    autocomplete="off"
    spellcheck="false"
    bind:value={subjectSearch.query}
  />
</label>

<style>
  /* Chrome, so it wears the material's own tint + rim — never an opaque plate
     (that is for content). Matches 09's title-bar field: 200 × 26. */
  .sbsearch {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    width: 200px;
    height: 26px;
    padding: 0 var(--s-8);
    border-radius: var(--r-md);
    background: var(--glass-tint);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .sbsearch:focus-within {
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border), var(--ring);
  }
  .sbsearch__glyph {
    flex: 0 0 auto;
    display: block;
    color: var(--app-text-subtle);
  }
  .sbsearch__input {
    flex: 1 1 auto;
    min-width: 0;
    border: 0;
    background: transparent;
    outline: none;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-text-strong);
  }
  .sbsearch__input::placeholder {
    color: var(--app-text-subtle);
  }
  .sbsearch__input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }
</style>
