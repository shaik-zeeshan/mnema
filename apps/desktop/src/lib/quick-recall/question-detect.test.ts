// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig (no @types/bun dependency), so skip static checking here.
import { describe, expect, test } from "bun:test";
import { askEngine, askNotice, autoAnswerAllowed, isQuestion, nextForced, sameQuestion, scopedQuestion } from "./question-detect";

const settings = (kind: string, override = "") => ({
  access: { askAiModel: override },
  aiRuntime: {
    enabled: true,
    providers: [{ id: "p1", kind, label: "", baseUrl: "" }],
    defaultModel: { provider: "p1", model: "accounts/x/claude-sonnet-4-6" },
    mcpServers: [],
  },
});

describe("isQuestion", () => {
  test("a trailing ? or a leading question word", () => {
    expect(isQuestion("what did Ana say about retention?")).toBe(true);
    expect(isQuestion("retention numbers?")).toBe(true);
    expect(isQuestion("who was on the standup")).toBe(true);
    expect(isQuestion("summarize")).toBe(true);
    expect(isQuestion("tell me about pricing")).toBe(true);
  });
  test("searches, words that merely start alike, and a bare wh-word are not", () => {
    expect(isQuestion("pricing page")).toBe(false);
    expect(isQuestion("issue tracker")).toBe(false);
    expect(isQuestion("docs for speakrs")).toBe(false);
    expect(isQuestion("what")).toBe(false);
    expect(isQuestion("")).toBe(false);
  });
});

describe("scopedQuestion", () => {
  test("pills read back as their phrases, the ? stays last", () => {
    const pills = [{ kind: "app", app: "Arc" }, { kind: "date", value: "yesterday", label: "Yesterday", part: null }];
    expect(scopedQuestion("what did I read? ", pills)).toBe("what did I read in Arc yesterday?");
    expect(scopedQuestion("who was there", [])).toBe("who was there");
    expect(scopedQuestion("", [])).toBe("");
  });
});

describe("nextForced", () => {
  test("Tab flips, and flipping back to what detection does clears the force", () => {
    expect(nextForced(null, false)).toBe("ask");
    expect(nextForced("ask", false)).toBe(null);
    expect(nextForced(null, true)).toBe("search");
    expect(nextForced("search", true)).toBe(null);
  });
});

describe("askEngine + autoAnswerAllowed", () => {
  test("local = the default provider's kind; the override replaces only the model", () => {
    expect(askEngine(settings("ollama", "qwen3:14b"))).toEqual({ model: "qwen3:14b", local: true });
    expect(askEngine(settings("anthropic"))).toEqual({ model: "Claude Sonnet 4.6", local: false });
    expect(askEngine(null)).toEqual({ model: null, local: false });
  });
  test("always · local only · never", () => {
    const cloud = { model: "m", local: false };
    const local = { model: "m", local: true };
    expect(autoAnswerAllowed("always", cloud)).toBe(true);
    expect(autoAnswerAllowed("localOnly", cloud)).toBe(false);
    expect(autoAnswerAllowed("localOnly", local)).toBe(true);
    expect(autoAnswerAllowed("never", local)).toBe(false);
  });
});

describe("askNotice", () => {
  const s = settings("chatgpt");
  const cloud = { model: "gpt-5", local: false };
  test("available and online → nothing; offline with a cloud model → offline", () => {
    expect(askNotice(null, s, true, cloud)).toBe(null);
    expect(askNotice(null, s, false, { ...cloud, local: true })).toBe(null);
    expect(askNotice(null, s, false, cloud)?.title).toBe("You’re offline");
  });
  test("reconnect and unreachable name the provider, never the code", () => {
    const r = askNotice("needs_reconnect:p1", s, true, cloud);
    expect(r?.tone).toBe("danger");
    expect(r?.title).toBe("ChatGPT needs you to sign in again");
    expect(askNotice("provider_unreachable:p1", s, true, cloud)?.title).toBe("Can’t reach ChatGPT right now");
  });
  test("off, no provider, and any other code map to plain copy", () => {
    expect(askNotice("ask_ai_disabled", s, true, cloud)?.title).toBe("Ask AI is off");
    expect(askNotice("no_providers", s, true, cloud)?.title).toBe("Ask AI needs a provider");
    const other = askNotice("no_default_model", s, true, cloud);
    expect(other?.title).toBe("Ask AI isn’t set up yet");
    expect(other?.text).not.toContain("no_default_model");
  });
});

describe("sameQuestion", () => {
  test("ignores case, spacing and trailing punctuation, not words", () => {
    expect(sameQuestion("what did I do yesterday", "What did I  do yesterday?")).toBe(true);
    expect(sameQuestion("what did I do yesterday?", "what did I do yesterday ?!")).toBe(true);
    expect(sameQuestion("what did I do yesterday", "what did I do yesterday in Arc")).toBe(false);
    expect(sameQuestion("", "")).toBe(true);
  });
});
