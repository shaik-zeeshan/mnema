// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { highlightSegments, residualTerms } from "./context-highlight";

describe("residualTerms", () => {
  test("tokenizes, lowercases, and dedupes", () => {
    expect(residualTerms("Stripe webhook stripe")).toEqual([
      "webhook",
      "stripe",
    ]);
  });

  test("strips surrounding quotes and punctuation", () => {
    expect(residualTerms('"webhook", (retry)!')).toEqual(["webhook", "retry"]);
  });

  test("drops sub-two-character terms and blanks", () => {
    expect(residualTerms("a — retry  ")).toEqual(["retry"]);
  });

  test("sorts longer terms first so overlapping terms prefer the longer", () => {
    expect(residualTerms("web webhook")).toEqual(["webhook", "web"]);
  });

  test("empty residual query yields no terms", () => {
    expect(residualTerms("")).toEqual([]);
  });
});

describe("highlightSegments", () => {
  const concat = (segments) => segments.map((s) => s.text).join("");

  test("marks case-insensitive occurrences", () => {
    const segments = highlightSegments("Stripe retries stripe events", [
      "stripe",
    ]);
    expect(segments).toEqual([
      { text: "Stripe", marked: true },
      { text: " retries ", marked: false },
      { text: "stripe", marked: true },
      { text: " events", marked: false },
    ]);
  });

  test("marks inside words (mirrors backend snippet marks)", () => {
    const segments = highlightSegments("StripeEvent", ["stripe"]);
    expect(segments).toEqual([
      { text: "Stripe", marked: true },
      { text: "Event", marked: false },
    ]);
  });

  test("longer term wins where terms overlap", () => {
    const segments = highlightSegments("a webhook b", ["webhook", "web"]);
    expect(segments).toEqual([
      { text: "a ", marked: false },
      { text: "webhook", marked: true },
      { text: " b", marked: false },
    ]);
  });

  test("no terms degrades to one unmarked segment", () => {
    expect(highlightSegments("plain text", [])).toEqual([
      { text: "plain text", marked: false },
    ]);
  });

  test("terms that never appear leave the text unmarked", () => {
    expect(highlightSegments("plain text", ["zzz"])).toEqual([
      { text: "plain text", marked: false },
    ]);
  });

  test("empty text yields no segments", () => {
    expect(highlightSegments("", ["stripe"])).toEqual([]);
  });

  test("regex metacharacters in terms are literal", () => {
    const segments = highlightSegments("uses c++ today", ["c++"]);
    expect(segments).toEqual([
      { text: "uses ", marked: false },
      { text: "c++", marked: true },
      { text: " today", marked: false },
    ]);
  });

  test("segments always reassemble the original text", () => {
    const text = "async fn handle_stripe_event(payload: WebhookPayload)";
    const segments = highlightSegments(text, ["stripe", "webhook", "retry"]);
    expect(concat(segments)).toBe(text);
  });
});
