// Pure, framework-free filter logic for the Combobox search box. Extracted out
// of Combobox.svelte so it can be unit-tested without Svelte/runes (the
// component re-exports it, so existing importers are unaffected).

export interface ComboboxOption {
  value: string;
  label: string;
}

/**
 * Case-insensitive substring filter over option labels (and values, so an
 * id-typed query still matches). The query is trimmed before matching, and an
 * empty/whitespace-only query passes every option through unchanged.
 */
export function filterComboboxOptions(
  options: ComboboxOption[],
  query: string,
): ComboboxOption[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return options;
  return options.filter(
    (o) =>
      o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q),
  );
}
