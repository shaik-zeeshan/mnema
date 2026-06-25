import { describe, expect, test } from "bun:test";
import {
  filterComboboxOptions,
  type ComboboxOption,
} from "../src/lib/components/combobox-filter";

const OPTIONS: ComboboxOption[] = [
  { value: "gpt-4o", label: "GPT-4o" },
  { value: "claude-opus", label: "Claude Opus" },
  { value: "llama-3", label: "Llama 3" },
];

describe("filterComboboxOptions", () => {
  test("matches case-insensitively on the label", () => {
    const result = filterComboboxOptions(OPTIONS, "claude");
    expect(result.map((o) => o.value)).toEqual(["claude-opus"]);

    // Upper-cased query against a lower-cased label substring still matches.
    expect(filterComboboxOptions(OPTIONS, "OPUS").map((o) => o.value)).toEqual([
      "claude-opus",
    ]);
  });

  test("matches against the value, so an id-typed query still hits", () => {
    // "gpt-4o" appears in the value but the label is "GPT-4o" — lowercasing
    // both sides means either path matches.
    expect(filterComboboxOptions(OPTIONS, "gpt-4o").map((o) => o.value)).toEqual(
      ["gpt-4o"],
    );
  });

  test("a query with no match returns an empty array", () => {
    expect(filterComboboxOptions(OPTIONS, "mistral")).toEqual([]);
  });

  test("an empty query returns all options unchanged", () => {
    expect(filterComboboxOptions(OPTIONS, "")).toEqual(OPTIONS);
  });

  test("a whitespace-only query is trimmed to empty and passes all through", () => {
    expect(filterComboboxOptions(OPTIONS, "   \t  ")).toEqual(OPTIONS);
  });

  test("trims surrounding whitespace before matching", () => {
    expect(
      filterComboboxOptions(OPTIONS, "  llama  ").map((o) => o.value),
    ).toEqual(["llama-3"]);
  });
});
