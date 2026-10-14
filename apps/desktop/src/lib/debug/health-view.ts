// Pure mapping for the Overview tab and the mini view: the 12 health rows as
// grouped, ranked rows, the mini dots, and the mini footer line. No Svelte, no
// IPC, so `bun test` covers it.

import type { FeatureHealthDto } from "$lib/types";
import { formatClock } from "$lib/time-format";
import type { DevTab } from "./state/controller.svelte";
import type { ProcessorKey } from "./detail/specs";
import { FEATURE_ROWS, healthVerdict, toneOf, verdict, type Tone } from "./verdict";

export { toneOf };

/** Where a row drills: its processor's drill-in, or its home tab. */
export type RowTarget = { processor: ProcessorKey } | { tab: DevTab };
export type RowView = { feature: string; tone: Tone; name: string; why: string; metric: string; target: RowTarget };
export type RowGroup = { title: string; rows: RowView[] };

const RANK: Record<Tone, number> = { danger: 0, warn: 1, info: 2, off: 3, ok: 4 };
const PROCESSORS: string[] = ["reading", "transcription", "speakers", "meaningIndex"];

/** The mini view's eight dots, in order, with their short names. */
export const MINI_DOTS: [feature: string, label: string][] = [
	["screen", "Screen"],
	["microphone", "Mic"],
	["systemAudio", "Sys audio"],
	["reading", "Reading"],
	["transcription", "Transcription"],
	["speakers", "Speakers"],
	["meaningIndex", "Meaning"],
	["aiEngine", "AI"],
];

/** `metricAtMs` wins as a local clock time ("used 14:28" for the AI engine). */
export function metricText(row: FeatureHealthDto): string {
	if (row.metricAtMs == null) return row.metric ?? "";
	return `${row.feature === "aiEngine" ? "used " : ""}${formatClock(row.metricAtMs)}`;
}

export function rowView(row: FeatureHealthDto): RowView {
	const tone = toneOf(row.severity);
	const meta = FEATURE_ROWS[row.feature];
	const target: RowTarget =
		PROCESSORS.includes(row.feature) && (tone === "warn" || tone === "danger")
			? { processor: row.feature as ProcessorKey }
			: { tab: meta?.tab ?? "overview" };
	return { feature: row.feature, tone, name: meta?.name ?? row.feature, why: row.reason, metric: metricText(row), target };
}

/** "Needs a look" · "Right now" · "Running fine", each ranked; empty groups dropped. */
export function group(rows: FeatureHealthDto[]): RowGroup[] {
	const all = rows.map(rowView).sort((a, b) => RANK[a.tone] - RANK[b.tone]);
	const pick = (...tones: Tone[]) => all.filter((r) => tones.includes(r.tone));
	return [
		{ title: "Needs a look", rows: pick("danger", "warn") },
		{ title: "Right now", rows: pick("info", "off") },
		{ title: "Running fine", rows: pick("ok") },
	].filter((g) => g.rows.length > 0);
}

/** The mini footer: the worst row as "Name: why", else the verdict sentence. */
export function miniFooter(rows: FeatureHealthDto[], error: string | null = null): { tone: Tone; text: string } {
	if (error) return healthVerdict(rows, error);
	const v = verdict(rows);
	const worst =
		v.tone === "danger" || v.tone === "warn" || v.tone === "info"
			? rows.map(rowView).sort((a, b) => RANK[a.tone] - RANK[b.tone])[0]
			: undefined;
	return { tone: v.tone, text: worst ? `${worst.name}: ${worst.why}` : v.text };
}
