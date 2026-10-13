// Pure mapping for the Pipeline tab: flow nodes, processor rows and the
// housekeeping lines. No Svelte, no IPC, so `bun test` covers it.
//
// Health rows are typed structurally (`feature`/`severity` strings) so this
// compiles against both the old and the 12-row health contract.

import type { AppJobDto, ProcessorPipelineStatus, RecordingSettings, SemanticIndexStatusDto } from "$lib/types";
import { DETAIL_SPECS, PROCESSOR_KEYS, type ProcessorKey } from "./detail/specs";
import { toneOf, type Tone } from "./verdict";

/** A node line, optionally toned. */
export type NodeLine = { text: string; tone?: Tone };
export type FlowNode = { name: string; big: string; unit: string; lines: NodeLine[]; bar?: number; tone?: Tone };
export type ProcRow = { key: ProcessorKey; name: string; tone: Tone; why: string; metric: string };
export type HealthLike = { feature: string; severity: string };

/** The backlog bar fills toward this many waiting jobs. */
export const BAR_FULL_JOBS = 50;
// ponytail: meaning counts passages, not jobs, so its bar uses a 10× scale
// (the mockup draws 212 behind as ~0.4). Tune here if it reads wrong.
export const BAR_FULL_PASSAGES = 500;

const CAPTURE = [
	{ feature: "screen", short: "screen" },
	{ feature: "microphone", short: "mic" },
	{ feature: "systemAudio", short: "sys audio" },
];
const TONE_RANK: Record<Tone, number> = { danger: 0, warn: 1, info: 2, off: 3, ok: 4 };
const STATE_WORD: Partial<Record<Tone, string>> = { danger: "failed", warn: "recovering", info: "waits" };

const sevOf = (health: HealthLike[], feature: string) => health.find((h) => h.feature === feature)?.severity;
const count = (n: number) => n.toLocaleString();
const bar = (n: number, full: number) => Math.min(n / full, 1);

/** Is anything capturing? Read from the always-polled health rows. */
export function isRecording(health: HealthLike[]): boolean {
	return CAPTURE.some(({ feature }) => {
		const sev = sevOf(health, feature);
		return sev != null && sev !== "off";
	});
}

function captureNode(health: HealthLike[]): FlowNode {
	const sources = CAPTURE.map((c) => ({ ...c, tone: toneOf(sevOf(health, c.feature) ?? "off") }));
	// Live = recording (a warn source is still recording while it recovers).
	const live = sources.filter((s) => s.tone === "ok" || s.tone === "warn").length;
	const node: FlowNode = { name: "Capture", big: String(live), unit: "sources live", lines: [] };
	if (!isRecording(health)) {
		node.lines.push({ text: "not recording" });
		return node;
	}
	const worst = sources.filter((s) => STATE_WORD[s.tone]).sort((a, b) => TONE_RANK[a.tone] - TONE_RANK[b.tone])[0];
	if (worst) node.lines.push({ text: worst.short, tone: worst.tone }, { text: STATE_WORD[worst.tone]!, tone: worst.tone });
	return node;
}

function laneNode(name: string, lane: ProcessorPipelineStatus, recording: boolean): FlowNode {
	// The wire's `queued` already includes `retrying` (a retry reverts to queued).
	const waiting = lane.queued;
	const first: NodeLine =
		lane.retrying > 0
			? { text: `${count(lane.retrying)} retrying`, tone: "warn" }
			: { text: lane.running > 0 || waiting > 0 ? `${count(lane.running)} running` : "idle" };
	const failed: NodeLine =
		lane.failedLast24h > 0 ? { text: `${count(lane.failedLast24h)} failed`, tone: "danger" } : { text: "0 failed" };
	// Retrying outranks failed: the live condition (e.g. a cloud provider offline,
	// which waits without burning tries) is the headline, as in the mockup.
	const tone: Tone | undefined =
		lane.retrying > 0 ? "warn" : lane.failedLast24h > 0 ? "danger" : !recording && waiting > 0 ? "info" : undefined;
	return { name, big: count(waiting), unit: "waiting", lines: [first, failed], bar: bar(waiting, BAR_FULL_JOBS), tone };
}

function meaningNode(index: SemanticIndexStatusDto | null, recording: boolean): FlowNode {
	const behind = index?.backlogCount ?? 0;
	return {
		name: "Meaning",
		big: index ? count(behind) : "—",
		unit: "behind",
		lines: [{ text: index ? `${count(index.vectorCount)} done` : "—" }],
		bar: bar(behind, BAR_FULL_PASSAGES),
		tone: !recording && behind > 0 ? "info" : undefined,
	};
}

/**
 * The five flow nodes. `lane(processor)` must zero-default a missing lane
 * (`laneFor` in features.svelte.ts). Waiting = the lane's `queued`, which
 * already counts the retrying jobs.
 */
export function flowNodes(
	health: HealthLike[],
	lane: (processor: string) => ProcessorPipelineStatus,
	index: SemanticIndexStatusDto | null,
): FlowNode[] {
	const recording = isRecording(health);
	return [
		captureNode(health),
		laneNode("Reading", lane(DETAIL_SPECS.reading.processor!), recording),
		laneNode("Transcription", lane(DETAIL_SPECS.transcription.processor!), recording),
		laneNode("Speakers", lane(DETAIL_SPECS.speakers.processor!), recording),
		meaningNode(index, recording),
	];
}

const PROC_NAME: Record<ProcessorKey, string> = {
	reading: "Reading",
	transcription: "Transcription",
	speakers: "Speakers",
	meaningIndex: "Meaning index",
};
const PROVIDER_NAME: Record<string, string> = {
	apple_vision: "Apple Vision",
	tesseract: "Tesseract",
	parakeet: "Parakeet",
	local_whisper: "Whisper",
	apple_speech_on_device: "Apple Speech",
	deepgram: "Deepgram",
	speakrs: "speakrs",
};
const CLOUD = new Set(["deepgram"]);

/** "Parakeet, on this Mac" / "Deepgram, cloud" — the processor's engine in plain words. */
export function engineWords(key: ProcessorKey, settings: RecordingSettings | null, lane: ProcessorPipelineStatus | null): string {
	const config = DETAIL_SPECS[key].config(settings);
	if (!config) return "";
	// The meaning index has one provider; its model id is the readable part.
	const name =
		key === "meaningIndex"
			? (config.modelId ?? "Meaning index")
			: (PROVIDER_NAME[config.provider] ?? config.provider.replaceAll("_", " "));
	if (!CLOUD.has(config.provider)) return `${name}, on this Mac`;
	return lane && lane.retrying > 0 ? `${name}, cloud · can’t be reached` : `${name}, cloud`;
}

function avg(seconds: number | null): string {
	if (seconds == null) return "";
	return seconds < 0.1 ? `${Math.round(seconds * 1000)} ms avg` : `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)} s avg`;
}

/** The four processor rows. Tone comes from the health row, so it matches Overview. */
export function processorRows(
	health: HealthLike[],
	lane: (processor: string) => ProcessorPipelineStatus,
	settings: RecordingSettings | null,
): ProcRow[] {
	return PROCESSOR_KEYS.map((key) => {
		const processor = DETAIL_SPECS[key].processor;
		const l = processor ? lane(processor) : null;
		return {
			key,
			name: PROC_NAME[key],
			tone: toneOf(sevOf(health, key) ?? "ok"),
			why: engineWords(key, settings, l),
			metric: avg(l?.averageCompletedSecondsLast24h ?? null),
		};
	});
}

// ─── Housekeeping ────────────────────────────────────────────────────────────

/** `get_retention_cleanup_status` (app-infra `RetentionCleanupSummary`), the fields read here. */
export interface RetentionCleanupSummary {
	policy: string;
	deletedCaptureSegments: number;
	status: string;
	errorMessage: string | null;
	createdAt: string | null;
}

/** `list_frame_batches` (app-infra `FrameBatch`): unfinished batches only, newest first. */
export interface FrameBatch {
	id: number;
	status: "open" | "closed" | "processing" | "completed" | "failed";
	updatedAt: string;
	lastError: string | null;
}

/** A spec line: value + the raw timestamp for the note column (formatted by the view). */
export type SpecLine = { value: string; at: string | null };

const firstLine = (s: string) => s.split("\n")[0].trim();

export function retentionLine(summary: RetentionCleanupSummary | null): SpecLine {
	if (!summary) return { value: "—", at: null };
	const days = /^days_(\d+)$/.exec(summary.policy)?.[1];
	if (!days) return { value: "keeps everything", at: null };
	if (summary.status === "failed") return { value: `failed · ${firstLine(summary.errorMessage ?? "unknown error")}`, at: summary.createdAt };
	if (!summary.createdAt) return { value: `nothing run yet · keeps ${days} days`, at: null };
	const n = summary.deletedCaptureSegments;
	return { value: `removed ${count(n)} ${n === 1 ? "file" : "files"} older than ${days} days`, at: summary.createdAt };
}

const latest = (rows: { updatedAt: string }[]) =>
	rows.reduce<string | null>((max, r) => (max == null || r.updatedAt > max ? r.updatedAt : max), null);

export function frameBatchLine(batches: FrameBatch[]): SpecLine {
	const failed = batches.filter((b) => b.status === "failed");
	const open = batches.length - failed.length;
	if (failed.length > 0) {
		const err = failed.find((b) => b.lastError)?.lastError;
		return { value: `${count(failed.length)} failed${err ? ` · ${firstLine(err)}` : ""}`, at: latest(failed) };
	}
	return { value: `${open > 0 ? count(open) : "none"} in progress, none failed`, at: latest(batches) };
}

const JOB_KIND: Record<string, string> = {
	frame_batch_combine: "Combining frames",
	debug_cpu_analysis: "Test CPU job",
};
const kindLabel = (kind: string) => JOB_KIND[kind] ?? kind.replaceAll("_", " ");

export function appJobsLine(jobs: AppJobDto[]): SpecLine {
	const running = jobs.filter((j) => j.status === "running");
	if (running.length > 0) {
		const kinds = [...new Set(running.map((j) => kindLabel(j.kind)))].join(", ");
		return { value: `${count(running.length)} running · ${kinds}`, at: latest(running) };
	}
	const queued = jobs.filter((j) => j.status === "queued").length;
	return { value: queued > 0 ? `none running · ${count(queued)} queued` : "none running", at: latest(jobs) };
}
