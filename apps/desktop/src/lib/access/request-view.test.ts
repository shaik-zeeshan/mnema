// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";
import {
  allowLabel,
  fitAccessWindowHeight,
  identitySource,
  isWiden,
  requestAge,
  SCOPES,
  scopeDisabled,
  scopeHint,
  scopeTag,
  splitCommand,
} from "./request-view";

const req = (over = {}) => ({
  requestId: "r1",
  client: { label: "Claude Code", source: "inferred" },
  command: 'search --query "x"',
  minimumScope: "lastDay",
  preferredScope: "last7Days",
  currentScope: null,
  createdAt: "2026-10-10T10:00:00Z",
  ...over,
});

describe("CLI Access request view", () => {
  it("tags a first request: only the ask", () => {
    expect(SCOPES.map((s) => scopeTag(req(), s))).toEqual(["", "asked for", ""]);
  });

  it("tags a widen: current, then needed", () => {
    const r = req({ minimumScope: "last7Days", preferredScope: "last7Days", currentScope: "lastDay" });
    expect(isWiden(r)).toBe(true);
    expect(SCOPES.map((s) => scopeTag(r, s))).toEqual(["current", "needed", ""]);
    expect(scopeHint(r, "last7Days")).toBe("Deny keeps Last day — this command fails.");
    expect(allowLabel(r, "last7Days")).toBe("Allow last 7 days");
  });

  it("is not a widen when the tool already holds what it asks", () => {
    const r = req({ preferredScope: "lastDay", currentScope: "allRetained" });
    expect(isWiden(r)).toBe(false);
    expect(allowLabel(r, "lastDay")).toBe("Allow access");
  });

  it("disables only scopes below the minimum and names a raised minimum", () => {
    const r = req({ minimumScope: "last7Days", preferredScope: "allRetained" });
    expect(SCOPES.map((s) => scopeDisabled(r, s))).toEqual([true, false, false]);
    expect(SCOPES.map((s) => scopeTag(r, s))).toEqual(["", "needed", "asked for"]);
  });

  it("names the ask until the selection covers it; full history escalates Allow", () => {
    expect(scopeHint(req(), "lastDay")).toContain("asked for your last 7 days");
    expect(scopeHint(req(), "last7Days")).toBe("");
    expect(allowLabel(req(), "lastDay")).toBe("Allow access");
    expect(allowLabel(req(), "allRetained")).toBe("Allow full-history access");
  });

  it("maps identity provenance to the chip", () => {
    expect(identitySource("explicit")).toMatchObject({ chip: "Declared", warn: false });
    expect(identitySource("env")).toMatchObject({ chip: "From env", warn: false });
    expect(identitySource("inferred")).toMatchObject({ chip: "Inferred", warn: true });
    expect(identitySource("defaulted")).toMatchObject({ chip: "Unnamed", warn: true });
  });

  it("splits the command and ages the request", () => {
    expect(splitCommand('search --query "x"')).toEqual({ head: "mnema search", rest: '--query "x"' });
    expect(splitCommand("speakers")).toEqual({ head: "mnema speakers", rest: "" });
    const t = Date.parse("2026-10-10T10:00:00Z");
    expect(requestAge("2026-10-10T10:00:00Z", t + 2000)).toBe("just now");
    expect(requestAge("2026-10-10T10:00:00Z", t + 30_000)).toBe("Requested 30s ago");
    expect(requestAge("2026-10-10T10:00:00Z", t + 120_000)).toBe("Requested 2 min ago");
    expect(requestAge("nope", t)).toBeNull();
  });
});

describe("fitAccessWindowHeight", () => {
  it("fits a new view exactly, shrinking a short one", () => {
    expect(fitAccessWindowHeight(600, 280, false, 900)).toBe(280);
    expect(fitAccessWindowHeight(280, 640, false, 900)).toBe(640);
  });
  it("only grows within one view", () => {
    expect(fitAccessWindowHeight(640, 600, true, 900)).toBeNull();
    expect(fitAccessWindowHeight(600, 672, true, 900)).toBe(672);
  });
  it("caps at the screen and ignores sub-pixel noise", () => {
    expect(fitAccessWindowHeight(600, 1400, false, 900)).toBe(900);
    expect(fitAccessWindowHeight(600, 601, false, 900)).toBeNull();
  });
});
