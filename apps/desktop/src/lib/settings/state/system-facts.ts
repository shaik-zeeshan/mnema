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

/** Where the chosen retention window puts you on the disk you actually have. */
export interface RetentionFootprint {
	/** 0–100: the kept bytes as a share of (kept + free) — the axis position. */
	percent: number;
	/** "you are here · 34.2 GB" — the axis marker's own label. */
	marker: string;
	/** What is left after it, for the far end of the axis. */
	free: string;
}

/**
 * The retention ladder's axis: your real footprint marked on the ladder's own
 * scale, so the window is chosen against the disk rather than in the abstract.
 *
 * The axis spans what this window would hold PLUS what is still free — the only
 * two figures Mnema can actually measure. `null` when either is missing (G8),
 * or for "keep forever", which has no ceiling to mark.
 */
export function retentionFootprint(
	facts: SystemFacts | null,
	policy: RetentionPolicy,
): RetentionFootprint | null {
	if (!facts || facts.measuredBytesPerDay === null || facts.diskFreeBytes === null) return null;
	const days = retentionToDays(policy);
	if (days === null) return null;
	const kept = facts.measuredBytesPerDay * days;
	const span = kept + facts.diskFreeBytes;
	if (span <= 0) return null;
	return {
		percent: Math.min(100, Math.round((kept / span) * 100)),
		marker: `you are here · ${formatBytes(kept)}`,
		free: `${formatBytes(facts.diskFreeBytes)} free`,
	};
}

/** A model's fit against THIS machine — the verdict, not the gigabytes. */
export interface ModelFitVerdict {
	tone: "ok" | "warn" | "bad";
	label: string;
}

/**
 * Does this model fit on this Mac?
 *
 * Direction 02's model rows carry `size · fit verdict computed against this
 * Mac`, and the verdict — not the byte count — is the output. Two real limits
 * decide it, in order of how hard they bite:
 *
 *  1. Free disk. A model larger than the space left cannot be downloaded at
 *     all; that is a fact, not a judgement.
 *  2. Physical RAM. A weights file is resident while the model runs, so the
 *     download size is a floor on the memory it needs. Under a quarter of RAM
 *     is comfortable; up to a half is workable; past that it competes with
 *     everything else on the machine.
 *
 * `null` when either the size or both machine limits are unmeasurable — G8: no
 * denominator, no claim.
 */
export function modelFitVerdict(
	facts: SystemFacts | null,
	byteSize: number | null,
): ModelFitVerdict | null {
	if (byteSize === null || byteSize <= 0) return null;
	if (facts?.diskFreeBytes != null && byteSize > facts.diskFreeBytes) {
		return { tone: "bad", label: `needs more than the ${formatBytes(facts.diskFreeBytes)} free` };
	}
	const ram = facts?.totalRamBytes ?? null;
	if (ram === null || ram <= 0) return null;
	const share = byteSize / ram;
	if (share <= 0.25) return { tone: "ok", label: `fits in ${formatBytes(ram)} RAM` };
	if (share <= 0.5) return { tone: "warn", label: `tight for ${formatBytes(ram)} RAM` };
	return { tone: "bad", label: `too large for ${formatBytes(ram)} RAM` };
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
