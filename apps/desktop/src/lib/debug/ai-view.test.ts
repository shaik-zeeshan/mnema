// @ts-nocheck — exercised by `bun test`; `bun:test` types aren't in the
// svelte-check tsconfig, so skip static checking here.
import { describe, expect, it } from "bun:test";

globalThis.$state = (v) => v; // cli-access.svelte.ts is a runes module (see its test)
const { cliRows, durationStat, engineView, runRow, tokenStat } = await import("./ai-view");

const status = (over = {}) => ({
	enabled: true,
	hasProviders: true,
	configured: true,
	available: false,
	defaultModel: { provider: "anthropic", model: "claude-sonnet-4-5" },
	defaultProviderKind: "anthropic",
	reason: null,
	...over,
});

describe("engineView", () => {
	it("available → ok, provider + model", () => {
		expect(engineView(status({ available: true }), null)).toEqual({
			tone: "ok",
			title: "AI engine is available",
			provider: "Anthropic",
			model: "claude-sonnet-4-5",
		});
	});
	it("unreachable → warn, resumes on its own, never reconnect", () => {
		const v = engineView(status({ reason: "provider_unreachable:chatgpt-2" }), null);
		expect(v.tone).toBe("warn");
		expect(v.title).toBe("ChatGPT can’t be reached, resumes on its own");
		expect(v.title).not.toMatch(/reconnect|sign in/i);
		expect(engineView(status({ reason: "local_endpoint_unreachable", defaultProviderKind: "ollama" }), null).tone).toBe("warn");
	});
	it("needs_reconnect → danger", () => {
		expect(engineView(status({ reason: "needs_reconnect:chatgpt" }), null).tone).toBe("danger");
	});
	it("disabled → off", () => {
		expect(engineView(status({ enabled: false, reason: "ai_runtime_disabled" }), null)).toMatchObject({
			tone: "off",
			title: "AI is turned off",
		});
	});
	it("not set up → info with plain copy, unknown code → danger", () => {
		const v = engineView(status({ reason: "no_provider_key:openai" }), null);
		expect(v.tone).toBe("info");
		expect(v.title).not.toContain("no_provider_key");
		expect(engineView(status({ reason: "something_else" }), null).tone).toBe("danger");
	});
	it("no status: loading, or warn on a load error", () => {
		expect(engineView(null, null)).toBeNull();
		expect(engineView(null, "boom").tone).toBe("warn");
	});
});

describe("stats", () => {
	it("splits number and unit", () => {
		expect(tokenStat(18240)).toEqual(["18.2", "k"]);
		expect(tokenStat(612)).toEqual(["612", ""]);
		expect(durationStat(9800)).toEqual(["9.8", "s"]);
		expect(durationStat(640)).toEqual(["640", "ms"]);
	});

	it("picks the unit after rounding, never \"1000k\" or \"60.0 s\"", () => {
		expect(tokenStat(999_949)).toEqual(["999.9", "k"]);
		expect(tokenStat(999_950)).toEqual(["1", "M"]);
		expect(durationStat(59_949)).toEqual(["59.9", "s"]);
		expect(durationStat(59_950)).toEqual(["1.0", "min"]);
		expect(durationStat(999.6)).toEqual(["1.0", "s"]);
	});
});

const run = (over = {}) => ({
	id: 1, kind: "activity", windowStartMs: 0, windowEndMs: 900_000, status: "completed",
	activitiesDerived: 6, conclusionsDerived: 0, inputTokens: 2900, outputTokens: 200, provider: null, model: null,
	error: null, ungrounded: 0, guardrailSuppressed: 0, belowFormationBar: 0, resurfaceBlocked: 0,
	superseded: 0, supersedeDegraded: 0, supersedeBlocked: 0, createdAtMs: 900_000, ...over,
});

describe("runRow", () => {
	it("maps kind, chip and result", () => {
		const r = runRow(run());
		expect([r.kind, r.chip.label, r.result]).toEqual(["Activities", "done", "6 activities"]);
		expect(r.meta.startsWith("3.1k tok · ")).toBe(true);
		expect(runRow(run({ kind: "conclusion", conclusionsDerived: 1, ungrounded: 2 })).result).toBe("1 conclusion, 2 dropped");
		expect(runRow(run({ status: "failed", error: "x" })).chip).toEqual({ label: "failed", tone: "danger" });
		expect(runRow(run({ status: "skipped" })).chip.label).toBe("skipped");
	});
});

describe("cliRows", () => {
	it("newest first, refused for any non-success outcome", () => {
		const e = (cmd, outcome, t) => ({ toolIdentity: "Codex", normalizedToolIdentity: "codex", commandType: cmd,
			timestampUnixMs: t, resultCount: 0, scopeClass: "", outcome });
		const rows = cliRows([e("search", null, 1), e("show_text", "success", 2), e("timeline", "denied", 3)], 2);
		expect(rows.map((r) => [r.command, r.allowed])).toEqual([["timeline", false], ["show-text", true]]);
	});
});
