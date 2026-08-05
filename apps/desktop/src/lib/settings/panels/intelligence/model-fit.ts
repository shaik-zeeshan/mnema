// The model picker's fit verdict (round-4 decision **G8**).
//
// The instrument's real output is not the number — it is the verdict. A model's
// weights have to be resident to run, so its on-disk size is the floor on what
// it costs this Mac's memory; that floor against `totalRamBytes` is the one
// denominator this machine can actually answer. Peak RSS during a run is higher
// and nothing here measures it, so the copy says "weights", never "uses".
//
// G8's three rules apply verbatim:
//  1. A missing fact yields a null verdict — no bar, no chip, no placeholder.
//  2. Nothing is rounded past what the source supports.
//  3. Disk and RAM are the only limits consulted; there is no thermal fact.
//
// Pure — no runes, no `invoke`, unit-tested in `./model-fit.test.ts`.

/** Fraction of RAM above which the weights alone are a problem while recording. */
const HEAVY_RAM_FRACTION = 0.1;
/** Fraction of RAM above which the weights will not co-exist with capture. */
const TOO_LARGE_RAM_FRACTION = 0.25;

export type FitTone = "ok" | "warn" | "danger";

export interface ModelFit {
	/** 0–100 of this Mac's RAM, or `null` when RAM is unreadable. */
	ramPercent: number | null;
	/** Short chip copy, or `null` when no limit could be checked. */
	verdict: string | null;
	tone: FitTone | null;
}

/**
 * Weigh one model's download size against this Mac's two real limits. Returns
 * `null` when there is nothing local to weigh at all (cloud providers, OS-managed
 * models) — the caller renders no footprint for those, not a zero.
 */
export function modelFit(
	byteSize: number | null | undefined,
	totalRamBytes: number | null | undefined,
	diskFreeBytes: number | null | undefined,
): ModelFit | null {
	if (byteSize == null || byteSize <= 0) return null;

	// Disk is checked first: a model that cannot land is not a RAM question.
	if (diskFreeBytes != null && byteSize > diskFreeBytes) {
		return {
			ramPercent: totalRamBytes != null && totalRamBytes > 0
				? Math.min(100, (byteSize / totalRamBytes) * 100)
				: null,
			verdict: "not enough free disk",
			tone: "danger",
		};
	}

	if (totalRamBytes == null || totalRamBytes <= 0) {
		return { ramPercent: null, verdict: null, tone: null };
	}

	const fraction = byteSize / totalRamBytes;
	const ramPercent = Math.min(100, fraction * 100);
	if (fraction > TOO_LARGE_RAM_FRACTION) {
		return { ramPercent, verdict: "too large for this Mac", tone: "danger" };
	}
	if (fraction > HEAVY_RAM_FRACTION) {
		return { ramPercent, verdict: "heavy while recording", tone: "warn" };
	}
	return { ramPercent, verdict: "fits easily", tone: "ok" };
}
