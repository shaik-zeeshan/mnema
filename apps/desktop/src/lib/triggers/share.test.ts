// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (repo convention, see the sibling *.test.ts files).
// Trigger JSON share → import round-trip + rejection cases (issue #184).
// Run: bun test apps/desktop/src/lib/triggers/share.test.ts
import { describe, expect, test } from "bun:test";

import type { TriggerDefinition } from "./api";
import { parseTriggerJson, shareTriggerJson, TRIGGER_JSON_VERSION } from "./share";

function def(partial: Partial<TriggerDefinition> & Pick<TriggerDefinition, "condition">): TriggerDefinition {
  return {
    id: "trg-1",
    name: "My Trigger",
    prompt: "Write a recap.",
    enabled: true,
    version: 1,
    ...partial,
  };
}

function roundTrip(trigger: TriggerDefinition) {
  const parsed = parseTriggerJson(shareTriggerJson(trigger));
  if (!parsed.ok) throw new Error(`round-trip rejected: ${parsed.error}`);
  return parsed.draft;
}

describe("share → import round-trip", () => {
  test("meeting_ends with advanced minMeetingMinutes + cooldown", () => {
    const trigger = def({
      condition: { type: "meeting_ends", minMeetingMinutes: 12 },
      cooldownMinutes: 25,
    });
    expect(roundTrip(trigger)).toEqual({
      name: "My Trigger",
      condition: { type: "meeting_ends", minMeetingMinutes: 12 },
      prompt: "Write a recap.",
      cooldownMinutes: 25,
    });
  });

  test("app_opened with advanced awayGapMinutes", () => {
    const trigger = def({
      name: "Figma catch-up",
      condition: {
        type: "app_opened",
        bundleId: "com.figma.Desktop",
        appName: "Figma",
        awayGapMinutes: 45,
      },
    });
    expect(roundTrip(trigger)).toEqual({
      name: "Figma catch-up",
      condition: {
        type: "app_opened",
        bundleId: "com.figma.Desktop",
        appName: "Figma",
        awayGapMinutes: 45,
      },
      prompt: "Write a recap.",
    });
  });

  test("weekly schedule with a weekday set + cooldown", () => {
    const trigger = def({
      condition: {
        type: "schedule",
        cadence: "weekly",
        time: "9:30",
        weekdays: ["monday", "friday"],
      },
      cooldownMinutes: 60,
    });
    expect(roundTrip(trigger)).toEqual({
      name: "My Trigger",
      condition: {
        type: "schedule",
        cadence: "weekly",
        time: "9:30",
        weekdays: ["monday", "friday"],
      },
      prompt: "Write a recap.",
      cooldownMinutes: 60,
    });
  });

  test("weekday set imports in canonical Mon→Sun order, deduplicated", () => {
    const parsed = parseTriggerJson(
      JSON.stringify({
        version: 1,
        name: "N",
        prompt: "P",
        condition: {
          type: "schedule",
          cadence: "weekly",
          time: "18:00",
          weekdays: ["friday", "monday", "friday"],
        },
      }),
    );
    expect(parsed).toEqual({
      ok: true,
      draft: {
        name: "N",
        prompt: "P",
        condition: {
          type: "schedule",
          cadence: "weekly",
          time: "18:00",
          weekdays: ["monday", "friday"],
        },
      },
    });
  });

  test("legacy single-weekday payload (pre-multi-day, still version 1) imports as a one-day set", () => {
    const parsed = parseTriggerJson(
      JSON.stringify({
        version: 1,
        name: "N",
        prompt: "P",
        condition: { type: "schedule", cadence: "weekly", time: "9:30", weekday: "friday" },
      }),
    );
    expect(parsed).toEqual({
      ok: true,
      draft: {
        name: "N",
        prompt: "P",
        condition: { type: "schedule", cadence: "weekly", time: "9:30", weekdays: ["friday"] },
      },
    });
  });

  test("daily schedule, all defaults (no optional fields)", () => {
    const trigger = def({ condition: { type: "schedule", cadence: "daily", time: "18:00" } });
    const draft = roundTrip(trigger);
    expect(draft).toEqual({
      name: "My Trigger",
      condition: { type: "schedule", cadence: "daily", time: "18:00" },
      prompt: "Write a recap.",
    });
    expect("cooldownMinutes" in draft).toBe(false);
  });

  test("canonical JSON shape: exactly {version, name, condition, prompt, cooldownMinutes?}, no id/enabled/provider", () => {
    const json = JSON.parse(
      shareTriggerJson(def({ condition: { type: "meeting_ends" }, cooldownMinutes: 15 })),
    );
    expect(Object.keys(json).sort()).toEqual([
      "condition",
      "cooldownMinutes",
      "name",
      "prompt",
      "version",
    ]);
    expect(json.version).toBe(TRIGGER_JSON_VERSION);
  });

  test("unknown extra fields are stripped, known fields kept", () => {
    const parsed = parseTriggerJson(
      JSON.stringify({
        version: 1,
        name: "N",
        prompt: "P",
        condition: { type: "meeting_ends", minMeetingMinutes: 8, junk: true },
        apiKey: "sk-should-never-survive",
      }),
    );
    expect(parsed).toEqual({
      ok: true,
      draft: { name: "N", condition: { type: "meeting_ends", minMeetingMinutes: 8 }, prompt: "P" },
    });
  });

  test("absent version is treated as version 1", () => {
    const parsed = parseTriggerJson(
      JSON.stringify({ name: "N", prompt: "P", condition: { type: "meeting_ends" } }),
    );
    expect(parsed.ok).toBe(true);
  });
});

describe("import rejection", () => {
  function errorOf(text: string): string {
    const parsed = parseTriggerJson(text);
    if (parsed.ok) throw new Error("expected rejection");
    return parsed.error;
  }

  test("empty clipboard", () => {
    expect(errorOf("")).toContain("Clipboard is empty");
  });

  test("garbage input", () => {
    expect(errorOf("not json at all {{{")).toContain("isn't valid JSON");
  });

  test("non-object JSON", () => {
    expect(errorOf("[1,2,3]")).toContain("isn't a trigger");
    expect(errorOf("42")).toContain("isn't a trigger");
  });

  test("unknown version", () => {
    const error = errorOf(
      JSON.stringify({ version: 2, name: "N", prompt: "P", condition: { type: "meeting_ends" } }),
    );
    expect(error).toContain("version 2");
    expect(error).toContain("version 1");
  });

  test("missing name / missing prompt", () => {
    expect(
      errorOf(JSON.stringify({ version: 1, prompt: "P", condition: { type: "meeting_ends" } })),
    ).toContain('"name"');
    expect(
      errorOf(JSON.stringify({ version: 1, name: "N", condition: { type: "meeting_ends" } })),
    ).toContain('"prompt"');
  });

  test("missing or non-object condition", () => {
    expect(errorOf(JSON.stringify({ version: 1, name: "N", prompt: "P" }))).toContain(
      '"condition"',
    );
    expect(
      errorOf(JSON.stringify({ version: 1, name: "N", prompt: "P", condition: "schedule" })),
    ).toContain('"condition"');
  });

  test("unknown condition type", () => {
    expect(
      errorOf(
        JSON.stringify({ version: 1, name: "N", prompt: "P", condition: { type: "on_email" } }),
      ),
    ).toContain('Unknown condition type "on_email"');
  });

  test("app_opened missing bundleId / appName", () => {
    expect(
      errorOf(
        JSON.stringify({
          version: 1,
          name: "N",
          prompt: "P",
          condition: { type: "app_opened", appName: "Figma" },
        }),
      ),
    ).toContain('"bundleId"');
    expect(
      errorOf(
        JSON.stringify({
          version: 1,
          name: "N",
          prompt: "P",
          condition: { type: "app_opened", bundleId: "com.figma.Desktop" },
        }),
      ),
    ).toContain('"appName"');
  });

  test("schedule bad cadence / bad time / weekly without weekdays", () => {
    const base = { version: 1, name: "N", prompt: "P" };
    expect(
      errorOf(JSON.stringify({ ...base, condition: { type: "schedule", cadence: "hourly", time: "18:00" } })),
    ).toContain('"cadence"');
    expect(
      errorOf(JSON.stringify({ ...base, condition: { type: "schedule", cadence: "daily", time: "25:99" } })),
    ).toContain('"time"');
    // No days at all / empty set / junk entries / bad legacy weekday.
    expect(
      errorOf(JSON.stringify({ ...base, condition: { type: "schedule", cadence: "weekly", time: "18:00" } })),
    ).toContain('"weekdays"');
    expect(
      errorOf(
        JSON.stringify({
          ...base,
          condition: { type: "schedule", cadence: "weekly", time: "18:00", weekdays: [] },
        }),
      ),
    ).toContain('"weekdays"');
    expect(
      errorOf(
        JSON.stringify({
          ...base,
          condition: {
            type: "schedule",
            cadence: "weekly",
            time: "18:00",
            weekdays: ["friday", "someday"],
          },
        }),
      ),
    ).toContain('"weekdays"');
    expect(
      errorOf(
        JSON.stringify({
          ...base,
          condition: { type: "schedule", cadence: "weekly", time: "18:00", weekday: "someday" },
        }),
      ),
    ).toContain('"weekdays"');
  });

  test("wrong-type minute fields", () => {
    const base = { version: 1, name: "N", prompt: "P" };
    expect(
      errorOf(
        JSON.stringify({ ...base, condition: { type: "meeting_ends", minMeetingMinutes: "5" } }),
      ),
    ).toContain('"minMeetingMinutes"');
    expect(
      errorOf(
        JSON.stringify({
          ...base,
          condition: {
            type: "app_opened",
            bundleId: "b",
            appName: "a",
            awayGapMinutes: 2.5,
          },
        }),
      ),
    ).toContain('"awayGapMinutes"');
    expect(
      errorOf(
        JSON.stringify({ ...base, condition: { type: "meeting_ends" }, cooldownMinutes: -10 }),
      ),
    ).toContain('"cooldownMinutes"');
  });
});
