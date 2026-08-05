// Consequence copy derived from real machine facts (round-4 decision **G8**).
//
// Three rules, enforced here so no caller has to remember them:
//  1. A fact that is `null` yields a `null` phrase — the row renders NO number
//     rather than a placeholder or a zero.
//  2. Nothing is rounded to a precision the source can't support. Durations are
//     coarse by construction ("about 3 weeks"), never minute-precise.
//  3. No temperature, ever. There is no thermal fact in `SystemFacts` to derive
//     one from.
//
// No Svelte runes and no `invoke` here — pure, unit-tested.

import type { RetentionPolicy, SystemFacts } from "$lib/types";
import { retentionToDays } from "$lib/components/retention";
import { formatBytes } from "./format";

/**
 * The measured daily capture rate projected onto a different screen capture
 * rate. Storage scales linearly with frame rate at a preset bitrate
 * (`compute_effective_screen_bitrate_bps` multiplies by the frame rate), so the
 * projection is one ratio.
 *
 * `null` unless there is a measured rate AND the fps it was measured at — a
 * projection off an unknown baseline is a guess, and G8 bans guesses.
 */
export function projectedBytesPerDay(
	facts: SystemFacts | null,
	targetFps: number | null | undefined,
): number | null {
	if (!facts || facts.measuredBytesPerDay === null) return null;
	const measuredAt = facts.screenFrameRate;
	if (!targetFps || !measuredAt || targetFps <= 0 || measuredAt <= 0) return null;
	return facts.measuredBytesPerDay * (targetFps / measuredAt);
}

/**
 * How long the free space lasts at a given daily rate, phrased coarsely. Days
 * up to a fortnight, then weeks, then months — the input is a 7-day average, so
 * anything finer would be false precision.
 */
export function coarseRuntime(freeBytes: number | null, bytesPerDay: number | null): string | null {
	if (freeBytes === null || bytesPerDay === null || bytesPerDay <= 0) return null;
	const days = freeBytes / bytesPerDay;
	if (days < 1) return "under a day";
	if (days < 2) return "about a day";
	if (days < 14) return `about ${Math.round(days)} days`;
	if (days < 60) return `about ${Math.round(days / 7)} weeks`;
	if (days < 365) return `about ${Math.round(days / 30)} months`;
	return "over a year";
}

/**
 * The capture-rate row's consequence: what this slider position costs per day,
 * and how long the disk lasts at it. `null` until a complete capture day has
 * been measured — before then Mnema genuinely does not know.
 */
export function captureRateConsequence(
	facts: SystemFacts | null,
	targetFps: number | null | undefined,
): string | null {
	const perDay = projectedBytesPerDay(facts, targetFps);
	if (perDay === null || !facts) return null;
	const runtime = coarseRuntime(facts.diskFreeBytes, perDay);
	const measured = `measured over your last ${facts.measuredDays} ${
		facts.measuredDays === 1 ? "day" : "days"
	} of capture`;
	return runtime === null
		? `About ${formatBytes(perDay)} a day at this rate — ${measured}.`
		: `About ${formatBytes(perDay)} a day at this rate — ${runtime} of free space left. ${
				measured.charAt(0).toUpperCase() + measured.slice(1)
			}.`;
}

/**
 * The retention row's consequence: how much stays on disk under a window.
 * "Forever" has no ceiling to state, so it gets the runtime instead.
 */
export function retentionConsequence(
	facts: SystemFacts | null,
	policy: RetentionPolicy,
): string | null {
	if (!facts || facts.measuredBytesPerDay === null) return null;
	const days = retentionToDays(policy);
	if (days === null) {
		const runtime = coarseRuntime(facts.diskFreeBytes, facts.measuredBytesPerDay);
		return runtime === null
			? null
			: `Nothing is deleted — at your measured rate the free space lasts ${runtime}.`;
	}
	return `Keeps about ${formatBytes(facts.measuredBytesPerDay * days)} on disk at your measured rate.`;
}

/**
 * The cost slider's read-out (direction 04's `.costout`): the stored unit on
 * the left, the unit you actually care about on the right. Both halves come
 * from the same measured projection, so the month figure is the day figure ×30
 * — not a second guess.
 */
export interface CaptureRateCost {
	/** The setting's own value, e.g. "2 fps". */
	value: string;
	/** The consequence, e.g. "≈ 1.4 GB / day · ≈ 42 GB / month". */
	cost: string;
}

export function captureRateCost(
	facts: SystemFacts | null,
	targetFps: number | null | undefined,
): CaptureRateCost | null {
	const perDay = projectedBytesPerDay(facts, targetFps);
	if (perDay === null || !targetFps) return null;
	return {
		value: `${targetFps} fps`,
		cost: `≈ ${formatBytes(perDay)} / day · ≈ ${formatBytes(perDay * 30)} / month`,
	};
}

/**
 * The retention ladder's axis (direction 04): the projection and the user's
 * ACTUAL footprint on one scale, so the choice is legible against reality
 * rather than against a duration word.
 *
 * Both numbers are real. `kept` is the measured daily rate × the window; `used`
 * is the measured rate × the days actually measured — what capture has really
 * put on disk over the window Mnema can see. The scale is the volume's free
 * space plus what is already used, i.e. the space this decision plays out in.
 * `null` whenever any of the three is unmeasurable (G8: no denominator, no bar).
 */
export interface RetentionLadder {
	keptPercent: number;
	usedPercent: number;
	phrase: string;
}

export function retentionLadder(
	facts: SystemFacts | null,
	policy: RetentionPolicy,
): RetentionLadder | null {
	if (!facts || facts.measuredBytesPerDay === null || facts.diskFreeBytes === null) return null;
	const used = facts.measuredBytesPerDay * facts.measuredDays;
	const days = retentionToDays(policy);
	// "Forever" has no ceiling, so the projection is the whole remaining volume.
	const kept = days === null ? facts.diskFreeBytes + used : facts.measuredBytesPerDay * days;
	const scale = facts.diskFreeBytes + used;
	if (scale <= 0) return null;
	const pct = (bytes: number) => Math.max(0, Math.min(100, (bytes / scale) * 100));
	return {
		keptPercent: pct(kept),
		usedPercent: pct(used),
		phrase:
			days === null
				? `Nothing is deleted · ${formatBytes(used)} captured so far · ${formatBytes(facts.diskFreeBytes)} free`
				: `keeps ≈ ${formatBytes(kept)} · you have ${formatBytes(used)} today`,
	};
}

/**
 * The model row's verdict chip (direction 04): a download size against THIS
 * Mac's physical RAM — the comparison G8 names as real ("RAM total vs model
 * sizes"). Sizes come from the crate manifests (the corrected registry: speakrs
 * is 419 MB), never re-declared here.
 *
 * ponytail: three bands off the size:RAM ratio, not a runtime-RSS prediction.
 * Nothing on this machine measures a model's working set, so the chip states
 * the fact it has — weights vs RAM — and the copy says exactly that. Widen this
 * the day a measured peak-RSS lands in `SystemFacts`.
 */
export interface ModelFit {
	tone: "ok" | "warn" | "bad";
	/** Chip text, e.g. "FITS — 4.7 OF 16 GB". */
	label: string;
	/** True when the model is too large to offer — the caller disables Use. */
	blocked: boolean;
}

export function modelFit(facts: SystemFacts | null, byteSize: number | null): ModelFit | null {
	const ram = facts?.totalRamBytes ?? null;
	if (ram === null || byteSize === null || byteSize <= 0) return null;
	const ratio = byteSize / ram;
	const against = `${formatBytes(byteSize)} of ${formatBytes(ram)}`;
	if (ratio > 0.6) return { tone: "bad", label: `TOO LARGE — ${against}`, blocked: true };
	if (ratio > 0.25) return { tone: "warn", label: `TIGHT — ${against}`, blocked: false };
	return { tone: "ok", label: `FITS — ${against}`, blocked: false };
}

/**
 * The model-picker denominator: a download size against the two machine limits
 * it competes with. Whichever of the two is unmeasurable is simply left out.
 */
export function modelFootprint(facts: SystemFacts | null, byteSize: number | null): string | null {
	if (byteSize === null || byteSize <= 0) return null;
	const parts: string[] = [];
	if (facts?.diskFreeBytes != null) parts.push(`${formatBytes(facts.diskFreeBytes)} free on this disk`);
	if (facts?.totalRamBytes != null) parts.push(`${formatBytes(facts.totalRamBytes)} RAM on this Mac`);
	if (parts.length === 0) return null;
	return `${formatBytes(byteSize)} to download · ${parts.join(" · ")}`;
}

/**
 * A processing queue depth, in the units the user recognises. Zero is a real
 * measurement and says so; `null` (the query failed) renders nothing.
 */
export function backlogPhrase(count: number | null, unit: string): string | null {
	if (count === null) return null;
	if (count === 0) return `Nothing waiting.`;
	return `${count.toLocaleString()} ${unit}${count === 1 ? "" : "s"} waiting.`;
}

/**
 * What turning semantic search on would cost in index size — the user's real
 * pending-anchor count times the schema's bytes-per-vector (G10's
 * price-before-enable, priced per G8). Deliberately silent about how long it
 * takes: there is no measured embedding throughput, so any time figure would be
 * invented.
 */
export function semanticIndexPrice(facts: SystemFacts | null): string | null {
	if (!facts || facts.semanticPendingCount === null) return null;
	if (facts.semanticPendingCount === 0) return null;
	const bytes = facts.semanticPendingCount * facts.semanticVectorBytes;
	return `Indexing what you have captured so far adds about ${formatBytes(bytes)} to the database (${facts.semanticPendingCount.toLocaleString()} captures still to index).`;
}

/** How much of what Mnema can index actually is (round-4 decision **G10**). */
export interface SemanticCoverage {
	indexed: number;
	total: number;
	/** 0–100, floored so a part-done index never reads as finished. */
	percent: number;
	phrase: string;
}

/**
 * The coverage meter's numbers, for the ON state only — the caller gates on
 * "semantic search is enabled" (G10); this only refuses when there is nothing
 * real to draw: unreadable counts, or no indexable capture at all.
 *
 * `total` is the two real counts summed (vectors stored + anchors still
 * missing one), so the fraction has a denominator that exists on this machine
 * per G8. No time figure and no ETA: nothing measures embedding throughput, so
 * any "X minutes left" would be invented.
 */
export function semanticCoverage(facts: SystemFacts | null): SemanticCoverage | null {
	const indexed = facts?.semanticVectorCount ?? null;
	const pending = facts?.semanticPendingCount ?? null;
	if (indexed === null || pending === null) return null;
	const total = indexed + pending;
	if (total <= 0) return null;
	const percent = Math.floor((indexed / total) * 100);
	return {
		indexed,
		total,
		percent,
		phrase:
			pending === 0
				? `Indexed — all ${total.toLocaleString()} captures have a search vector.`
				: `${indexed.toLocaleString()} of ${total.toLocaleString()} captures indexed (${percent}%) — ${pending.toLocaleString()} still to go.`,
	};
}
