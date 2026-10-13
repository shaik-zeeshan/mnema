// Pure view logic for the developer window's AI tab: engine status copy and
// tone, last-turn stat values, User Context run rows and CLI request rows.

import { formatClock, formatClockRange } from "$lib/time-format";
import { aiProviderKindLabel, aiRuntimeReasonLabelFor } from "$lib/settings/state/ai-providers";
import { formatCommand, type BrokerAuditEvent } from "$lib/settings/state/cli-access.svelte";
import type { AiRuntimeStatus, DerivationRun, UserContextDigest } from "$lib/types";

export type AiTone = "ok" | "info" | "warn" | "danger" | "off";

export interface EngineView {
	tone: AiTone;
	title: string;
	provider: string | null;
	model: string | null;
}

/** `anthropic-2` → "Anthropic". Instance ids are `<kind>` or `<kind>-<n>`. */
export function providerName(id: string): string {
	return aiProviderKindLabel(id.replace(/-\d+$/, ""));
}

const NOT_SET_UP = ["no_providers", "no_default_model", "no_provider_key:", "provider_not_connected:", "no_base_url"];

/** Reason code → copy + tone, per SHELL.md "Engine status". */
export function engineView(status: AiRuntimeStatus | null, error: string | null): EngineView | null {
	if (!status) {
		return error ? { tone: "warn", title: "Couldn’t check the AI engine", provider: null, model: null } : null;
	}
	const model = status.defaultModel ?? null;
	const base = {
		provider: status.defaultProviderKind
			? aiProviderKindLabel(status.defaultProviderKind)
			: model
				? providerName(model.provider)
				: null,
		model: model?.model ?? null,
	};
	const reason = status.reason ?? "";
	if (!status.enabled || reason === "ai_runtime_disabled") return { ...base, tone: "off", title: "AI is turned off" };
	if (!status.hasProviders) return { ...base, tone: "off", title: "AI isn’t set up" };
	if (status.available || !reason) return { ...base, tone: "ok", title: "AI engine is available" };
	if (reason.startsWith("provider_unreachable:")) {
		const name = providerName(reason.slice("provider_unreachable:".length));
		return { ...base, tone: "warn", title: `${name} can’t be reached, resumes on its own` };
	}
	if (reason === "local_endpoint_unreachable") {
		return { ...base, tone: "warn", title: `${base.provider ?? "The local engine"} can’t be reached, resumes on its own` };
	}
	if (reason.startsWith("needs_reconnect:")) {
		const name = providerName(reason.slice("needs_reconnect:".length));
		return { ...base, tone: "danger", title: `${name} signed you out, sign in again in Settings` };
	}
	const title = aiRuntimeReasonLabelFor(reason, providerName).replace(/\.$/, "");
	if (NOT_SET_UP.some((code) => reason.startsWith(code))) return { ...base, tone: "info", title };
	return { ...base, tone: "danger", title };
}

/** A stat value split into number and unit: 18240 → ["18.2", "k"]. */
export function tokenStat(n: number): [string, string] {
	if (n < 1000) return [String(n), ""];
	if (n < 1_000_000) return [(n / 1000).toFixed(1).replace(/\.0$/, ""), "k"];
	return [(n / 1_000_000).toFixed(1).replace(/\.0$/, ""), "M"];
}

export function durationStat(ms: number): [string, string] {
	if (ms < 1000) return [String(Math.round(ms)), "ms"];
	if (ms < 60_000) return [(ms / 1000).toFixed(1), "s"];
	return [(ms / 60_000).toFixed(1), "min"];
}

/** "3.1k" — the compact token count used inside a log row. */
export function compactTokens(n: number): string {
	const [v, u] = tokenStat(n);
	return v + u;
}

const RUN_KIND: Record<string, string> = {
	activity: "Activities",
	conclusion: "Conclusions",
	confidence: "Confidence",
	backfill: "Backfill",
};

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export interface RunRow {
	kind: string;
	window: string;
	chip: { label: string; tone: "" | "info" | "danger" };
	result: string;
	meta: string;
}

export function runRow(run: DerivationRun): RunRow {
	const window =
		run.windowStartMs != null && run.windowEndMs != null ? formatClockRange(run.windowStartMs, run.windowEndMs) : "";
	const chip =
		run.status === "completed"
			? { label: "done", tone: "" as const }
			: run.status === "failed"
				? { label: "failed", tone: "danger" as const }
				: run.status === "running"
					? { label: "running", tone: "info" as const }
					: { label: run.status, tone: "" as const };
	const dropped = run.ungrounded + run.guardrailSuppressed + run.belowFormationBar + run.resurfaceBlocked;
	let result: string;
	if (run.status === "failed") result = "Couldn’t finish";
	else if (run.status === "running") result = "Working…";
	else if (run.status === "skipped") result = "Nothing captured";
	else {
		const made =
			run.kind === "conclusion" || run.kind === "confidence"
				? plural(run.conclusionsDerived, "conclusion", "conclusions")
				: plural(run.activitiesDerived, "activity", "activities");
		result = dropped > 0 ? `${made}, ${dropped} dropped` : made;
	}
	return {
		kind: RUN_KIND[run.kind] ?? run.kind,
		window,
		chip,
		result,
		meta: `${compactTokens(run.inputTokens + run.outputTokens)} tok · ${formatClock(run.createdAtMs)}`,
	};
}

/** "digest fresh · built 13:10" — fresh means it covers today. */
export function digestLine(digest: UserContextDigest | null | undefined, nowMs = Date.now()): string {
	if (!digest) return "no digest yet";
	const today = new Date(nowMs);
	today.setHours(0, 0, 0, 0);
	const fresh = digest.rangeEndMs > today.getTime();
	return `${fresh ? "digest fresh" : "digest from an earlier day"} · built ${formatClock(digest.generatedAtMs)}`;
}

export interface CliRow {
	tool: string;
	command: string;
	allowed: boolean;
	time: string;
}

/** Newest `limit` audit events, newest first. The file is append-ordered. */
export function cliRows(events: BrokerAuditEvent[], limit = 4): CliRow[] {
	return events
		.slice(-limit)
		.reverse()
		.map((e) => ({
			tool: e.toolIdentity,
			command: formatCommand(e.commandType),
			allowed: !e.outcome || e.outcome === "success",
			time: formatClock(e.timestampUnixMs),
		}));
}
