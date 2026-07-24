// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (repo convention, see the sibling *.test.ts files).
// Document View turn selection (ADR 0058, issue #182): which transcript turn of
// an origin=trigger conversation renders as the titled document.
// Run: bun test apps/desktop/src/lib/insights/document-view.test.ts
import { describe, expect, test } from "bun:test";

import { triggerDocTurnIndex } from "./conversation";

describe("triggerDocTurnIndex — the trigger Document header turn", () => {
  test("clean first-attempt success: turn 0 is the document", () => {
    expect(triggerDocTurnIndex([{ phase: "done" }])).toBe(0);
  });

  test("in-flight first run still owns the document while streaming", () => {
    expect(triggerDocTurnIndex([{ phase: "streaming" }])).toBe(0);
    expect(triggerDocTurnIndex([{ phase: "seeding" }])).toBe(0);
  });

  test("transient first-attempt failure recovered by the retry: the COMPLETED report at turnIndex 1 is the document, not the errored turn 0", () => {
    // Backend appends a fresh turn per attempt and persists an errored attempt
    // as an errored turn (ask_ai.rs), so a retry-recovered run lands its report
    // at index 1. The document header must follow the report, or the automated
    // firing prompt leaks as a right-aligned user bubble.
    expect(triggerDocTurnIndex([{ phase: "error" }, { phase: "done" }])).toBe(1);
  });

  test("Run Again after several failed attempts: the document is the first non-error turn", () => {
    expect(
      triggerDocTurnIndex([{ phase: "error" }, { phase: "error" }, { phase: "done" }]),
    ).toBe(2);
  });

  test("every attempt errored: no turn takes the document chrome", () => {
    expect(triggerDocTurnIndex([{ phase: "error" }, { phase: "error" }])).toBe(-1);
  });

  test("document then follow-ups: only the report turn is the document", () => {
    expect(
      triggerDocTurnIndex([{ phase: "done" }, { phase: "done" }, { phase: "done" }]),
    ).toBe(0);
  });
});
