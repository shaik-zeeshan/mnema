// subject-search-state.svelte.ts — the one piece of state the Subjects page and
// the title bar share.
//
// Page 09 puts "Search subjects…" IN THE TITLE BAR, which the app shell mounts
// (`SubjectsTitlebarControls`), not the page. The two live in different
// component trees, so the query rides in a module-level rune store instead of
// props. Owned by the Subjects slice; nothing else reads it.
//
// ponytail: a plain `$state` object, not a class/store abstraction — one field,
// two readers.
export const subjectSearch = $state({ query: "" });
