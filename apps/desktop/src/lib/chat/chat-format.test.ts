// @ts-nocheck — run under `bun test`; bun:test types aren't in the svelte-check
// tsconfig, so skip static checking here (same as journal-day.test.ts).
import { describe, expect, test } from "bun:test";
import { chatWhen, formatTokenCount, greeting, providerVia, providerWhere, stepsLine } from "./chat-format";
import type { AiProviderConfig } from "$lib/types/recording";

const p = (kind: AiProviderConfig["kind"], baseUrl = ""): AiProviderConfig =>
  ({ id: kind, kind, label: "", baseUrl }) as AiProviderConfig;

describe("providerWhere / providerVia", () => {
  test("local kinds and loopback endpoints are local", () => {
    expect(providerWhere(p("ollama"))).toBe("local");
    expect(providerWhere(p("openai_compatible", "http://localhost:8080/v1"))).toBe("local");
    expect(providerWhere(p("openai_compatible", "https://openrouter.ai/api/v1"))).toBe("cloud");
    expect(providerWhere(p("anthropic"))).toBe("cloud");
  });
  test("via names the reach", () => {
    expect(providerVia(p("anthropic"))).toBe("your API key");
    expect(providerVia(p("chatgpt"))).toBe("your ChatGPT sign-in");
    expect(providerVia(p("ollama", "http://localhost:11434"))).toBe("localhost:11434");
  });
});

describe("chatWhen", () => {
  const now = new Date(2026, 9, 9, 15, 0).getTime();
  test("clock time today and yesterday, weekday this week, date before", () => {
    expect(chatWhen(new Date(2026, 9, 9, 14, 52).getTime(), now)).toMatch(/52/);
    expect(chatWhen(new Date(2026, 9, 8, 18, 31).getTime(), now)).toMatch(/31/);
    expect(chatWhen(new Date(2026, 9, 5, 9, 0).getTime(), now)).toBe(
      new Date(2026, 9, 5).toLocaleDateString(undefined, { weekday: "short" }),
    );
    expect(chatWhen(new Date(2026, 8, 30, 9, 0).getTime(), now)).toMatch(/30/);
    expect(chatWhen(0, now)).toBe("");
  });
});

test("greeting and steps line", () => {
  expect(greeting(9)).toBe("Good morning.");
  expect(greeting(14)).toBe("Good afternoon.");
  expect(greeting(22)).toBe("Good evening.");
  expect(stepsLine(0, 900)).toBeNull();
  expect(stepsLine(1, null)).toBe("1 step");
  expect(stepsLine(4, 3400)).toBe("4 steps · 3.4s");
  expect([812, 6200, 14_200, 200_000].map(formatTokenCount)).toEqual(["812", "6.2k", "14k", "200k"]);
});
