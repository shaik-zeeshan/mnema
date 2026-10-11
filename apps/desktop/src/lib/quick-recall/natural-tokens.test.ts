// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { mergePills, mirrorSegments, parseNatural } from "./natural-tokens";
import { operatorQuery, pillLabel, pillOperators } from "./filter-chips";

// Friday 2026-10-09, mid-afternoon.
const NOW = new Date(2026, 9, 9, 15, 0);
const APPS = ["Arc", "Zed", "Google Chrome", "Notes"];

const ops = (text: string, now = NOW) => {
  const { spans, residual } = parseNatural(text, now, APPS);
  return operatorQuery(residual, mergePills([], spans.map((s) => s.pill)));
};

describe("parseNatural → operators", () => {
  test("the mockup's sentence becomes the footer's operator query", () => {
    expect(ops("pricing yesterday afternoon in Arc")).toBe(
      "pricing app:Arc date:yesterday hours:12-18",
    );
  });

  test("an app needs in/on/from/using; a bare word stays text", () => {
    expect(ops("invoice in Arc")).toBe("invoice app:Arc");
    expect(ops("pricing notes")).toBe("pricing notes");
    expect(ops("from google chrome")).toBe('app:"Google Chrome"');
    expect(ops("in Arcade")).toBe("in Arcade");
  });

  test("weekdays resolve to the most recent past ISO day", () => {
    expect(ops("deck last Tuesday")).toBe("deck date:2026-10-06");
    expect(ops("on friday")).toBe("date:2026-10-02"); // today is Friday → a week back
    expect(ops("thursday")).toBe("date:yesterday");
    expect(ops("tuesday morning")).toBe("date:2026-10-06 hours:5-12");
  });

  test("named days, weeks and N days ago", () => {
    expect(ops("today")).toBe("date:today");
    expect(ops("this week")).toBe("date:this-week");
    expect(ops("last week")).toBe("date:last-week");
    expect(ops("3 days ago")).toBe("date:2026-10-06");
    expect(ops("this afternoon")).toBe("date:today hours:12-18");
    expect(ops("tonight")).toBe("date:today hours:18-24");
  });

  test("a part of day on its own is an hours filter", () => {
    expect(ops("standup in the afternoon")).toBe("standup hours:12-18");
  });

  test("sources", () => {
    expect(ops("what Ana said on a call")).toBe("what Ana said source:mic");
    expect(ops("on my screen")).toBe("source:screen");
    expect(ops("in a podcast")).toBe("source:system");
  });

  test("typed operators become pills only when the backend would accept them", () => {
    expect(ops("app:ar")).toBe("app:Arc");
    expect(ops("source:microphone")).toBe("source:mic");
    expect(ops("hours:12-18")).toBe("hours:12-18");
    const bad = parseNatural("pricing date:tuesdy", NOW, APPS);
    expect(bad.spans).toEqual([]);
    expect(bad.residual).toBe("pricing date:tuesdy");
  });

  test("residual keeps a question's punctuation tight", () => {
    expect(parseNatural("what did Ana say yesterday?", NOW, APPS).residual).toBe(
      "what did Ana say?",
    );
  });
});

describe("mergePills", () => {
  const yesterday = { kind: "date", value: "yesterday", label: "Yesterday", part: null };
  const afternoon = { kind: "date", value: null, label: "", part: "afternoon" };

  test("a part narrows the date already there, and a new date keeps the part", () => {
    const narrowed = mergePills([yesterday], [afternoon]);
    expect(narrowed).toEqual([{ ...yesterday, part: "afternoon" }]);
    expect(pillLabel(narrowed[0])).toBe("Yesterday · afternoon");
    expect(mergePills([afternoon], [yesterday])).toEqual([{ ...yesterday, part: "afternoon" }]);
  });

  test("one pill per kind, in app · date · source order", () => {
    const merged = mergePills(
      [{ kind: "source", source: "mic" }, { kind: "app", app: "Zed" }],
      [{ kind: "app", app: "Arc" }, yesterday],
    );
    expect(merged.map(pillOperators)).toEqual(["app:Arc", "date:yesterday", "source:mic"]);
  });
});

describe("mirrorSegments", () => {
  test("tints recognised phrases and underlines the rejected token", () => {
    const text = "pricing yesterday date:tuesdy";
    const { spans } = parseNatural(text, NOW, APPS);
    expect(mirrorSegments(text, spans, "date:tuesdy")).toEqual([
      { text: "pricing ", kind: "plain" },
      { text: "yesterday", kind: "live" },
      { text: " ", kind: "plain" },
      { text: "date:tuesdy", kind: "bad" },
    ]);
  });
});
